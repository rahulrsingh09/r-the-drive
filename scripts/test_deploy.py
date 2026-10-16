#!/usr/bin/env python3
"""Fresh-install safety checks; no production services or credentials are used."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import threading
import unittest
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer
from unittest.mock import patch
from urllib.error import HTTPError

spec = importlib.util.spec_from_file_location('deployment', Path(__file__).with_name('deploy.py'))
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)
SOURCE = deploy.ROOT


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.root_patch = patch.object(deploy, 'ROOT', self.root)
        self.root_patch.start()
        self.settings, self.values, self.overlay = deploy.configuration(
            'synthetic-install', 'https://vehicle.example.test', 18880, 'synthetic:local')
        self.docker_implementation = deploy.docker
        self.docker_patch = patch.object(deploy, 'docker', side_effect=self.local_docker_query)
        self.docker_patch.start()

    def tearDown(self):
        self.docker_patch.stop()
        self.root_patch.stop()
        self.temporary.cleanup()

    def local_docker_query(self, arguments, **kwargs):
        responses = {
            ('compose', 'version'): '2',
            ('info', '--format', '{{.ServerVersion}}'): '28',
            ('context', 'inspect'): json.dumps([{
                'Endpoints': {'docker': {'Host': 'unix:///var/run/docker.sock'}}}]),
        }
        self.assertIn(tuple(arguments), responses, 'unexpected Docker action in unit test')
        return responses[tuple(arguments)]

    def write_configuration(self):
        (self.root / '.deploy').mkdir(mode=0o700)
        deploy.private_write(self.root / '.env', ''.join(f'{k}={v}\n' for k, v in self.values.items()))
        deploy.private_write(self.root / '.deploy/database-bootstrap-password', 'synthetic-bootstrap-only\n')

    def test_private_files_refuse_overwrite(self):
        path = self.root / 'private'
        deploy.private_write(path, 'synthetic')
        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        with self.assertRaises(FileExistsError):
            deploy.private_write(path, 'replacement')
        self.assertEqual(path.read_text(), 'synthetic')

    def test_origin_rejects_credentials_plaintext_and_extra_url_components(self):
        for origin in ['http://vehicle.example.test', 'https://owner@vehicle.example.test',
                       'https://vehicle.example.test/path', 'https://vehicle.example.test?key=value',
                       'https://vehicle.example.test#fragment', 'https://127.0.0.1:8080',
                       'https://localhost', 'https://vehicle.example.test\nINJECT=true']:
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                deploy.origin_url(origin)

    def test_generated_configuration_preserves_security_boundaries(self):
        self.assertEqual(self.values['PANDA_HOST_BIND_ADDRESS'], '127.0.0.1')
        self.assertEqual(self.values['ALLOW_INSECURE_LAN_HTTP_AUTH'], 'false')
        secrets = [self.values[k] for k in ('POSTGRES_PASSWORD', 'REDIS_PASSWORD', 'PANDA_SETUP_TOKEN')]
        self.assertEqual(len(set(secrets)), 3)
        self.assertTrue(all(len(secret) >= 64 for secret in secrets))
        self.assertEqual(self.overlay['services']['panda']['volumes'], ['custody:/run/secrets/app:ro'])
        self.assertTrue(self.overlay['volumes']['custody']['external'])
        self.assertEqual(self.values['POSTGRES_USER'], 'panda')
        database = self.overlay['services']['timescaledb']
        self.assertEqual(database['environment']['POSTGRES_USER'], 'panda_bootstrap')
        self.assertEqual(database['environment']['POSTGRES_PASSWORD'], '')
        self.assertNotIn('database_bootstrap', self.overlay['services']['panda'].get('secrets', []))

    def test_provisioning_precedes_application_and_password_is_only_stdin(self):
        self.write_configuration()
        with patch.object(deploy, 'compose', side_effect=['', '', 'panda']) as compose:
            deploy.provision_database(self.settings)
        calls = compose.call_args_list
        self.assertEqual(calls[0][0][1][-1], 'timescaledb')
        self.assertIn('panda_bootstrap', calls[1][0][1][-1])
        self.assertIn('-h timescaledb', calls[2][0][1][-1])
        self.assertIn('CREATE EXTENSION IF NOT EXISTS earthdistance', calls[1][1]['input_text'])
        self.assertIn('CREATE ROLE panda LOGIN NOSUPERUSER', calls[1][1]['input_text'])
        self.assertIn(self.values['POSTGRES_PASSWORD'], calls[1][1]['input_text'])
        self.assertEqual(calls[2][1]['input_text'],
                         self.values['POSTGRES_PASSWORD'] + '\nSELECT current_user;\n')
        for call in calls:
            self.assertNotIn(self.values['POSTGRES_PASSWORD'], repr(call[0]))
            self.assertNotIn('synthetic-bootstrap-only', repr(call))

    def test_legacy_installation_is_not_automatically_adopted(self):
        self.write_configuration()
        self.settings.pop('database_provisioning')
        with patch.object(deploy, 'compose') as compose, self.assertRaises(ValueError):
            deploy.provision_database(self.settings)
        compose.assert_not_called()
        self.assertIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())

    def test_unsafe_or_missing_bootstrap_file_is_rejected_before_start(self):
        self.write_configuration()
        secret = self.root / '.deploy/database-bootstrap-password'
        for mode in (0o644, 0o640):
            secret.chmod(mode)
            with patch.object(deploy, 'compose') as compose, self.assertRaises(ValueError):
                deploy.provision_database(self.settings)
            compose.assert_not_called()
        secret.unlink()
        with patch.object(deploy, 'compose') as compose, self.assertRaises(ValueError):
            deploy.provision_database(self.settings)
        compose.assert_not_called()

    def test_invalid_app_credentials_never_reach_database_commands(self):
        self.write_configuration()
        for assignment in ['POSTGRES_USER=other', 'POSTGRES_PASSWORD=bad\nvalue',
                           'DATABASE_URL=postgres://unexpected', 'POSTGRES_PASSWORD_FILE=/unexpected']:
            original = (self.root / '.env').read_text()
            (self.root / '.env').write_text(original + assignment + '\n')
            with patch.object(deploy, 'compose') as compose, self.assertRaises(ValueError):
                deploy.provision_database(self.settings)
            compose.assert_not_called()
            (self.root / '.env').write_text(original)

    def test_failed_provisioning_never_starts_app_or_prompts(self):
        self.write_configuration()
        before = (self.root / '.env').read_bytes()
        for outcome in [RuntimeError('provisioning failed'), 'unexpected_user']:
            responses = ['', outcome] if isinstance(outcome, Exception) else ['', '', outcome]
            with patch.object(deploy, 'compose', side_effect=responses) as compose, \
                    patch.object(deploy, 'local_api') as api, patch('builtins.input') as prompt, \
                    self.assertRaises(RuntimeError):
                deploy.start(self.settings)
            self.assertTrue(all(call[0][1][0] != 'up' or call[0][1][-1] == 'timescaledb'
                                for call in compose.call_args_list))
            api.assert_not_called()
            prompt.assert_not_called()
            self.assertEqual((self.root / '.env').read_bytes(), before)

    def test_private_docker_input_is_captured_and_failure_is_sanitized(self):
        private_input = 'synthetic-private-input'
        output = io.StringIO()
        result = subprocess.CompletedProcess([], 1, private_input, private_input)
        with patch.object(deploy.subprocess, 'run', return_value=result) as run, \
                contextlib.redirect_stdout(output), self.assertRaises(RuntimeError) as error:
            self.docker_implementation(['exec', '-i', 'synthetic'], input_text=private_input)
        self.assertEqual(run.call_args[1]['input'], private_input)
        self.assertEqual(run.call_args[1]['stdout'], subprocess.PIPE)
        self.assertEqual(run.call_args[1]['stderr'], subprocess.PIPE)
        self.assertNotIn(private_input, repr(run.call_args[0]))
        self.assertNotIn(private_input, str(error.exception) + output.getvalue())

    def test_existing_installation_is_not_overwritten(self):
        self.write_configuration()
        with patch.object(deploy, 'docker') as docker, self.assertRaises(ValueError):
            deploy.prepare('synthetic-install', self.settings['origin'], 18880)
        docker.assert_not_called()

    def test_existing_resources_and_remote_docker_are_rejected(self):
        for endpoint, containers, volumes in [
                ('ssh://example.test', '', ''),
                ('unix:///var/run/docker.sock', 'existing-container', ''),
                ('unix:///var/run/docker.sock', '', 'synthetic-install-keys'),
                ('unix:///var/run/docker.sock', '', 'synthetic-install_panda-db')]:
            context = json.dumps([{'Endpoints': {'docker': {'Host': endpoint}}}])
            with self.subTest(endpoint=endpoint, volumes=volumes), \
                    patch.dict(deploy.os.environ, {}, clear=True), \
                    patch.object(deploy, 'docker', side_effect=['2', '28', context, containers, volumes]), \
                    self.assertRaises(ValueError):
                deploy.preflight('synthetic-install')

    def test_start_rejects_remote_docker_before_any_installation_action(self):
        self.write_configuration()
        before = (self.root / '.env').read_bytes()
        context = json.dumps([{'Endpoints': {'docker': {'Host': 'ssh://remote.example.test'}}}])
        with patch.dict(deploy.os.environ, {}, clear=True), \
                patch.object(deploy, 'docker', side_effect=['2', '28', context]), \
                patch.object(deploy, 'provision_database') as provision, \
                patch.object(deploy, 'compose') as compose, \
                patch.object(deploy, 'local_api', return_value={'setup_required': False}) as api, \
                patch('builtins.input') as prompt, self.assertRaises(ValueError):
            deploy.start(self.settings)
        provision.assert_not_called()
        compose.assert_not_called()
        api.assert_not_called()
        prompt.assert_not_called()
        self.assertEqual((self.root / '.env').read_bytes(), before)

    def test_named_remote_context_cannot_be_hidden_by_local_docker_host(self):
        context = json.dumps([{'Endpoints': {'docker': {'Host': 'ssh://remote.example.test'}}}])
        environment = {'DOCKER_CONTEXT': 'remote-fixture', 'DOCKER_HOST': 'unix:///var/run/docker.sock'}
        with patch.dict(deploy.os.environ, environment, clear=True), \
                patch.object(deploy, 'docker', side_effect=['2', '28', context, '', '']) as docker, \
                self.assertRaises(ValueError):
            deploy.preflight('synthetic-install')
        self.assertEqual(docker.call_count, 3)

    def test_named_local_context_takes_precedence_over_docker_host(self):
        context = json.dumps([{'Endpoints': {'docker': {'Host': 'unix:///var/run/docker.sock'}}}])
        environment = {'DOCKER_CONTEXT': 'local-fixture', 'DOCKER_HOST': 'ssh://remote.example.test'}
        with patch.dict(deploy.os.environ, environment, clear=True), \
                patch.object(deploy, 'docker', side_effect=['2', '28', context]) as docker:
            deploy.require_local_docker()
        self.assertEqual(docker.call_count, 3)

    def test_remote_docker_host_is_rejected_without_a_named_context(self):
        context = json.dumps([{'Endpoints': {'docker': {'Host': 'unix:///var/run/docker.sock'}}}])
        with patch.dict(deploy.os.environ, {'DOCKER_HOST': 'ssh://remote.example.test'}, clear=True), \
                patch.object(deploy, 'docker', side_effect=['2', '28', context]), \
                self.assertRaises(ValueError):
            deploy.require_local_docker()

    def test_interrupted_proof_cleanup_preserves_stale_files_and_recovers(self):
        self.write_configuration()
        stale = self.root / '.deploy/env.next'
        deploy.private_write(stale, 'retained synthetic interrupted state\n')
        abandoned = self.root / '.deploy/env.next.abandoned'
        deploy.private_write(abandoned, 'retained synthetic unique temporary\n')
        with patch.object(deploy, 'provision_database'), patch.object(deploy, 'compose'), \
                patch.object(deploy, 'local_api', return_value={'setup_required': False}), \
                contextlib.redirect_stdout(io.StringIO()):
            deploy.start(self.settings)
        self.assertEqual(stale.read_text(), 'retained synthetic interrupted state\n')
        self.assertEqual(stat.S_IMODE(stale.stat().st_mode), 0o600)
        self.assertNotIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())
        self.assertEqual(stat.S_IMODE((self.root / '.env').stat().st_mode), 0o600)
        self.assertEqual(abandoned.read_text(), 'retained synthetic unique temporary\n')
        self.assertEqual(set((self.root / '.deploy').glob('env.next*')), {stale, abandoned})

    def test_stale_proof_cleanup_symlink_is_never_followed_or_replaced(self):
        self.write_configuration()
        unrelated = self.root / 'unrelated'
        deploy.private_write(unrelated, 'unrelated synthetic bytes\n')
        stale = self.root / '.deploy/env.next'
        stale.symlink_to(unrelated)
        with patch.object(deploy, 'provision_database'), patch.object(deploy, 'compose'), \
                patch.object(deploy, 'local_api', return_value={'setup_required': False}), \
                contextlib.redirect_stdout(io.StringIO()):
            deploy.start(self.settings)
        self.assertTrue(stale.is_symlink())
        self.assertEqual(unrelated.read_text(), 'unrelated synthetic bytes\n')
        self.assertNotIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())

    def test_failed_proof_replacement_retains_configuration_and_cleans_own_temporary(self):
        self.write_configuration()
        before = (self.root / '.env').read_bytes()

        def interrupted_replace(temporary, destination):
            self.assertEqual(destination, self.root / '.env')
            self.assertEqual(stat.S_IMODE(temporary.stat().st_mode), 0o600)
            self.assertEqual((self.root / '.env').read_bytes(), before)
            raise OSError('synthetic interrupted rename')

        with patch.object(deploy, 'provision_database'), patch.object(deploy, 'compose'), \
                patch.object(deploy, 'local_api', return_value={'setup_required': False}), \
                patch.object(Path, 'replace', interrupted_replace), \
                self.assertRaises(OSError):
            deploy.start(self.settings)
        self.assertEqual((self.root / '.env').read_bytes(), before)
        self.assertEqual(list((self.root / '.deploy').glob('env.next*')), [])

    def test_bad_setup_responses_retain_proof_and_never_prompt(self):
        self.write_configuration()
        for payload in [{}, [], {'setup_required': 'false'},
                        {'setup_required': True, 'setup_proof_required': False}]:
            with self.subTest(payload=payload), patch.object(deploy, 'provision_database'), \
                    patch.object(deploy, 'compose'), \
                    patch.object(deploy, 'local_api', return_value=payload), \
                    patch('builtins.input') as prompt, self.assertRaises(RuntimeError):
                deploy.start(self.settings)
            prompt.assert_not_called()
            self.assertIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())

    def test_owner_creation_removes_proof_only_after_confirmation(self):
        self.write_configuration()
        output = io.StringIO()
        with patch.object(deploy, 'compose') as compose, \
                patch.object(deploy, 'provision_database') as provision, \
                patch.object(deploy, 'local_api', side_effect=[
                    {'setup_required': True, 'setup_proof_required': True, 'setup_proof_available': True},
                    {'access_token': 'synthetic'},
                    {'setup_required': False},
                ]) as api, patch('builtins.input', return_value='owner@example.test'), \
                patch.object(deploy.getpass, 'getpass', return_value='synthetic-password-only'), \
                contextlib.redirect_stdout(output):
            deploy.start(self.settings)
        self.assertEqual(api.call_args_list[1][1]['body']['setup_token'], self.values['PANDA_SETUP_TOKEN'])
        self.assertNotIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())
        self.assertEqual(stat.S_IMODE((self.root / '.env').stat().st_mode), 0o600)
        self.assertEqual(compose.call_count, 2)
        provision.assert_called_once_with(self.settings)
        for secret in [self.values['PANDA_SETUP_TOKEN'], 'synthetic-password-only', 'owner@example.test']:
            self.assertNotIn(secret, output.getvalue())

    def test_unconfirmed_owner_creation_retains_proof(self):
        self.write_configuration()
        with patch.object(deploy, 'provision_database'), patch.object(deploy, 'compose'), \
                patch.object(deploy, 'local_api', side_effect=[
                {'setup_required': True, 'setup_proof_required': True, 'setup_proof_available': True},
                {}, {'setup_required': True}]), patch('builtins.input', return_value='owner@example.test'), \
                patch.object(deploy.getpass, 'getpass', return_value='synthetic-password-only'), \
                self.assertRaises(RuntimeError):
            deploy.start(self.settings)
        self.assertIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())

    @unittest.skipUnless(shutil.which('docker'), 'Docker Compose is unavailable')
    def test_real_compose_configuration_uses_private_ports_and_readonly_keys(self):
        self.write_configuration()
        (self.root / 'compose').mkdir()
        shutil.copyfile(SOURCE / 'compose/docker-compose.yml', self.root / 'compose/docker-compose.yml')
        deploy.private_write(self.root / '.deploy/compose.json', json.dumps(self.overlay))
        result = subprocess.run([
            'docker', 'compose', '--project-name', self.settings['project'],
            '--env-file', str(self.root / '.env'), '-f', str(self.root / 'compose/docker-compose.yml'),
            '-f', str(self.root / '.deploy/compose.json'), 'config', '--format', 'json',
        ], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, 'Compose rejected generated configuration')
        services = json.loads(result.stdout)['services']
        app = services['panda']
        self.assertEqual(app['ports'][0]['host_ip'], '127.0.0.1')
        self.assertTrue(app['read_only'])
        self.assertEqual(app['cap_drop'], ['ALL'])
        self.assertTrue(next(v for v in app['volumes'] if v['target'] == '/run/secrets/app')['read_only'])
        self.assertNotIn('ports', services['redis'])
        self.assertNotIn('ports', services['timescaledb'])
        self.assertEqual(app['environment']['POSTGRES_PASSWORD'], self.values['POSTGRES_PASSWORD'])
        self.assertEqual(app['environment']['POSTGRES_USER'], 'panda')
        self.assertNotIn('POSTGRES_PASSWORD_FILE', app['environment'])
        self.assertNotIn('secrets', app)
        database = services['timescaledb']
        self.assertEqual(database['environment']['POSTGRES_USER'], 'panda_bootstrap')
        self.assertEqual(database['environment']['POSTGRES_PASSWORD'], '')
        self.assertEqual(database['secrets'][0]['source'], 'database_bootstrap')
        self.assertNotIn('synthetic-bootstrap-only', result.stdout)

    def test_local_registration_never_follows_redirects(self):
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                requests.append(self.path)
                self.rfile.read(int(self.headers['Content-Length']))
                self.send_response(307)
                self.send_header('Location', '/unexpected')
                self.end_headers()

            def log_message(self, *_):
                pass

        server = HTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with self.assertRaises(HTTPError):
                deploy.local_api(server.server_port, '/register', body={'password': 'synthetic'})
            self.assertEqual(requests, ['/register'])
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


@unittest.skipUnless(os.environ.get('PANDA_TEST_INSTALLER_DATABASE') == '1',
                     'requires explicitly enabled disposable Docker Compose database')
class DatabaseProvisioningTests(unittest.TestCase):
    """Real role/extension/rollback checks on this test's unique database only."""

    def test_fresh_resume_and_incompatible_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = 'panda-installer-test-' + uuid.uuid4().hex[:12]
            with patch.object(deploy, 'ROOT', root):
                settings, values, overlay = deploy.configuration(
                    project, 'https://fixture.example.test', 18880, 'synthetic:unused')
                (root / '.deploy').mkdir(mode=0o700)
                (root / 'compose').mkdir()
                shutil.copyfile(SOURCE / 'compose/docker-compose.yml', root / 'compose/docker-compose.yml')
                overlay['services']['timescaledb']['mem_limit'] = '1g'
                overlay['services']['timescaledb']['command'] = [
                    'postgres', '-c', 'shared_buffers=128MB', '-c', 'max_connections=40',
                    '-c', 'max_worker_processes=8', '-c', 'timescaledb.max_background_workers=4']
                deploy.private_write(root / '.env', ''.join(f'{k}={v}\n' for k, v in values.items()))
                deploy.private_write(root / '.deploy/compose.json', json.dumps(overlay))
                deploy.private_write(root / '.deploy/database-bootstrap-password',
                                     deploy.secrets.token_urlsafe(48) + '\n')

                def admin(sql):
                    command = ('PGPASSWORD="$(cat /run/secrets/database_bootstrap)" '
                               'exec psql -X -q -t -A -h timescaledb -U panda_bootstrap '
                               '-d panda -v ON_ERROR_STOP=1')
                    return deploy.compose(settings, ['exec', '-T', 'timescaledb', '/bin/sh', '-c', command],
                                          capture=True, input_text=sql)

                try:
                    deploy.compose(settings, ['up', '-d', '--wait', '--wait-timeout', '600', 'timescaledb'])
                    interrupted = (SOURCE / 'scripts/provision_database.sql').read_text().replace(
                        'COMMIT;', 'SELECT 1 / 0;\nCOMMIT;')
                    with self.assertRaises(RuntimeError):
                        admin("\\set app_password '" + values['POSTGRES_PASSWORD'] + "'\n" + interrupted)
                    self.assertEqual(admin("SELECT count(*) FROM pg_roles WHERE rolname='panda';"), '0')
                    self.assertEqual(admin(
                        "SELECT count(*) FROM pg_namespace WHERE nspname IN ('panda','timeseries');"), '0')
                    deploy.provision_database(settings)
                    self.assertEqual(admin(
                        "SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication "
                        "OR rolbypassrls OR rolinherit FROM pg_roles WHERE rolname='panda';"), 'f')
                    self.assertEqual(admin(
                        "SELECT string_agg(extname || ':' || nspname, ',' ORDER BY extname) "
                        "FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace "
                        "WHERE extname IN ('pgcrypto','timescaledb','cube','earthdistance');"),
                        'cube:panda,earthdistance:panda,pgcrypto:public,timescaledb:public')
                    before = admin("SELECT oid::text || rolpassword FROM pg_authid WHERE rolname='panda';")
                    # Simulate existing application data and a revoked permission.
                    admin("SET ROLE panda; CREATE TABLE panda.installer_fixture (id integer); "
                          "INSERT INTO panda.installer_fixture VALUES (42); "
                          "REVOKE DELETE, TRUNCATE ON panda.installer_fixture FROM panda;")
                    deploy.provision_database(settings)
                    self.assertEqual(admin("SELECT oid::text || rolpassword FROM pg_authid WHERE rolname='panda';"),
                                     before)
                    self.assertEqual(admin("SELECT id FROM panda.installer_fixture;"), '42')
                    self.assertEqual(admin(
                        "SELECT has_table_privilege('panda','panda.installer_fixture','DELETE');"), 'f')
                    # A failed saved-login check must not silently rotate the role.
                    env = (root / '.env').read_text()
                    (root / '.env').write_text(env.replace(values['POSTGRES_PASSWORD'], 'x' * 64))
                    with self.assertRaises(RuntimeError):
                        deploy.provision_database(settings)
                    (root / '.env').write_text(env)
                    self.assertEqual(admin("SELECT oid::text || rolpassword FROM pg_authid WHERE rolname='panda';"),
                                     before)
                    # Drift is refused, not corrected by altering a pre-existing role.
                    admin("ALTER ROLE panda CREATEDB;")
                    with self.assertRaises(RuntimeError):
                        deploy.provision_database(settings)
                    self.assertEqual(admin("SELECT rolcreatedb FROM pg_roles WHERE rolname='panda';"), 't')
                    self.assertEqual(admin("SELECT id FROM panda.installer_fixture;"), '42')
                finally:
                    deploy.compose(settings, ['down', '-v', '--remove-orphans'])


if __name__ == '__main__':
    unittest.main()

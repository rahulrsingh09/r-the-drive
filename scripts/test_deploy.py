#!/usr/bin/env python3
"""Fresh-install safety checks; no production services or credentials are used."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import threading
import unittest
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

    def tearDown(self):
        self.root_patch.stop()
        self.temporary.cleanup()

    def write_configuration(self):
        (self.root / '.deploy').mkdir(mode=0o700)
        deploy.private_write(self.root / '.env', ''.join(f'{k}={v}\n' for k, v in self.values.items()))

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

    def test_bad_setup_responses_retain_proof_and_never_prompt(self):
        self.write_configuration()
        for payload in [{}, [], {'setup_required': 'false'},
                        {'setup_required': True, 'setup_proof_required': False}]:
            with self.subTest(payload=payload), patch.object(deploy, 'compose'), \
                    patch.object(deploy, 'local_api', return_value=payload), \
                    patch('builtins.input') as prompt, self.assertRaises(RuntimeError):
                deploy.start(self.settings)
            prompt.assert_not_called()
            self.assertIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())

    def test_owner_creation_removes_proof_only_after_confirmation(self):
        self.write_configuration()
        output = io.StringIO()
        with patch.object(deploy, 'compose') as compose, \
                patch.object(deploy, 'local_api', side_effect=[
                    {'setup_required': True, 'setup_proof_required': True, 'setup_proof_available': True},
                    {'access_token': 'synthetic'},
                    {'setup_required': False},
                ]) as api, patch('builtins.input', return_value='owner@example.test'), \
                patch.object(deploy.getpass, 'getpass', return_value='synthetic-password-only'), \
                contextlib.redirect_stdout(output):
            deploy.start(self.settings)
        self.assertEqual(api.call_args_list[1].kwargs['body']['setup_token'], self.values['PANDA_SETUP_TOKEN'])
        self.assertNotIn('PANDA_SETUP_TOKEN=', (self.root / '.env').read_text())
        self.assertEqual(stat.S_IMODE((self.root / '.env').stat().st_mode), 0o600)
        self.assertEqual(compose.call_count, 2)
        for secret in [self.values['PANDA_SETUP_TOKEN'], 'synthetic-password-only', 'owner@example.test']:
            self.assertNotIn(secret, output.getvalue())

    def test_unconfirmed_owner_creation_retains_proof(self):
        self.write_configuration()
        with patch.object(deploy, 'compose'), patch.object(deploy, 'local_api', side_effect=[
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


if __name__ == '__main__':
    unittest.main()

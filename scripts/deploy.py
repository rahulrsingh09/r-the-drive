#!/usr/bin/env python3
"""Fresh-install Docker setup; credentials stay in owner-only local files."""
import argparse
import getpass
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, build_opener, ProxyHandler, HTTPRedirectHandler

ROOT = Path(__file__).resolve().parent.parent


def origin_url(value):
    url = urlsplit(value)
    host = url.hostname or ''
    if (url.scheme != 'https' or not re.fullmatch(r'[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?', host)
            or '.' not in host or url.netloc not in (host, host + ':443')
            or url.path not in ('', '/') or url.query or url.fragment):
        raise ValueError('Use one HTTPS hostname, without credentials, a path, query or fragment.')
    return 'https://' + url.netloc


def private_write(path, content):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'w') as output:
        output.write(content)


def docker(args, *, capture=False, input_text=None):
    environment = {key: value for key, value in os.environ.items() if key in (
        'PATH', 'HOME', 'DOCKER_CONFIG', 'DOCKER_CONTEXT', 'DOCKER_HOST',
        'DOCKER_CERT_PATH', 'DOCKER_TLS_VERIFY', 'HTTP_PROXY', 'HTTPS_PROXY', 'NO_PROXY')}
    # Provisioning SQL can contain credentials. Never inherit stdout/stderr for
    # commands receiving private input, including their failure diagnostics.
    private = capture or input_text is not None
    result = subprocess.run(['docker', *args], cwd=ROOT, env=environment,
                            text=True, input=input_text,
                            stdout=subprocess.PIPE if private else None,
                            stderr=subprocess.PIPE if private else None)
    if result.returncode:
        raise RuntimeError('Docker operation failed. Existing files, keys and data were retained.')
    return result.stdout.strip() if capture else ''


def compose(settings, args, *, capture=False, input_text=None):
    return docker(['compose', '--project-name', settings['project'], '--env-file', str(ROOT / '.env'),
                   '-f', str(ROOT / 'compose/docker-compose.yml'),
                   '-f', str(ROOT / '.deploy/compose.json'), *args],
                  capture=capture, input_text=input_text)


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def local_api(port, path, *, origin=None, body=None):
    headers = {'Content-Type': 'application/json'}
    if origin:
        headers['Origin'] = origin
    request = Request(f'http://127.0.0.1:{port}{path}',
                      data=json.dumps(body).encode() if body is not None else None, headers=headers)
    opener = build_opener(ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=10) as response:
        return json.loads(response.read(1024 * 1024))


def configuration(project, origin, port, image):
    settings = {'project': project, 'origin': origin_url(origin), 'port': port, 'image': image,
                'database_provisioning': 1}
    values = {
        'PANDA_IMAGE': image, 'PANDA_ENV': 'production',
        'POSTGRES_USER': 'panda', 'POSTGRES_PASSWORD': secrets.token_urlsafe(48),
        'REDIS_PASSWORD': secrets.token_urlsafe(48),
        'PANDA_SETUP_TOKEN': secrets.token_urlsafe(48), 'ALLOWED_ORIGINS': settings['origin'],
        'PANDA_HOST_BIND_ADDRESS': '127.0.0.1', 'PANDA_ORIGIN_PORT': str(port),
        'PANDA_BIND_ADDRESS': '127.0.0.1', 'ALLOW_INSECURE_LAN_HTTP_AUTH': 'false', 'TZ': 'UTC',
        'PANDA_DB_SOURCE': 'panda-db', 'PANDA_REDIS_SOURCE': 'panda-redis',
        'PANDA_BACKUPS_SOURCE': 'panda-backups', 'PANDA_CACHE_SOURCE': 'panda-cache',
    }
    overlay = {'services': {'panda': {
        'environment': {'JWT_SECRET_FILE': '/run/secrets/app/keys/jwt_private.pem',
                        'JWT_PUBLIC_KEY_FILE': '/run/secrets/app/keys/jwt_public.pem',
                        'AGE_ENCRYPTION_KEY_FILE': '/run/secrets/app/keys/age_key.txt'},
        'volumes': ['custody:/run/secrets/app:ro']},
        'timescaledb': {
            'environment': {'POSTGRES_USER': 'panda_bootstrap', 'POSTGRES_PASSWORD': '',
                            'POSTGRES_PASSWORD_FILE': '/run/secrets/database_bootstrap'},
            'secrets': ['database_bootstrap'],
            'healthcheck': {'test': ['CMD', 'pg_isready', '-U', 'panda_bootstrap', '-d', 'panda']}}},
        'volumes': {'custody': {'external': True, 'name': project + '-keys'}},
        'secrets': {'database_bootstrap': {'file': str(ROOT / '.deploy/database-bootstrap-password')}}}
    return settings, values, overlay


def require_local_docker():
    docker(['compose', 'version'], capture=True)
    docker(['info', '--format', '{{.ServerVersion}}'], capture=True)
    context = json.loads(docker(['context', 'inspect'], capture=True))[0]
    endpoint = context['Endpoints']['docker']['Host']
    if not os.environ.get('DOCKER_CONTEXT'):
        endpoint = os.environ.get('DOCKER_HOST') or endpoint
    if not endpoint.startswith(('unix://', 'npipe://')):
        raise ValueError('Run this script on the Docker server; remote Docker contexts are unsupported.')


def preflight(project):
    require_local_docker()
    containers = docker(['ps', '-aq', '--filter', 'label=com.docker.compose.project=' + project], capture=True)
    volumes = docker(['volume', 'ls', '--format', '{{.Name}}'], capture=True).splitlines()
    if containers or any(v == project + '-keys' or v.startswith(project + '_') for v in volumes):
        raise ValueError('That Docker project already has resources; choose a new --project name.')


def prepare(project, origin, port, image=None):
    if (ROOT / '.env').exists() or (ROOT / '.deploy').exists():
        raise ValueError('An installation already exists here. Use start; configuration will not be overwritten.')
    if not re.fullmatch(r'[a-z][a-z0-9-]{2,40}', project):
        raise ValueError('Project names must contain 3–41 lowercase letters, digits or hyphens.')
    if not 1024 <= port <= 65535:
        raise ValueError('Choose a port from 1024 to 65535.')
    origin = origin_url(origin)
    if image and not re.fullmatch(r'[a-z0-9./_-]+@sha256:[0-9a-f]{64}', image):
        raise ValueError('An existing image must be pinned as registry/image@sha256:DIGEST.')
    preflight(project)
    if image:
        docker(['pull', image])
    else:
        image = project + ':local'
        docker(['build', '--build-arg', 'CARGO_BUILD_JOBS=2', '-f', 'compose/Dockerfile', '-t', image, '.'])
    settings, values, overlay = configuration(project, origin, port, image)
    (ROOT / '.deploy').mkdir(mode=0o700)
    private_write(ROOT / '.deploy/settings.json', json.dumps(settings) + '\n')
    private_write(ROOT / '.deploy/compose.json', json.dumps(overlay) + '\n')
    private_write(ROOT / '.env', ''.join(f'{key}={value}\n' for key, value in values.items()))
    private_write(ROOT / '.deploy/database-bootstrap-password', secrets.token_urlsafe(48) + '\n')
    key_volume = project + '-keys'
    docker(['volume', 'create', '--label', 'com.docker.compose.project=' + project, key_volume], capture=True)
    mount = 'type=volume,source=' + key_volume + ',target=/custody'
    docker(['run', '--rm', '--network', 'none', '--user', '0:0', '--cap-drop', 'ALL', '--cap-add', 'CHOWN',
            '--security-opt', 'no-new-privileges:true', '--mount', mount, '--entrypoint', '/bin/sh', image,
            '-c', 'chmod 700 /custody && chown 1001:1001 /custody'])
    docker(['run', '--rm', '--network', 'none', '--user', '1001:1001', '--cap-drop', 'ALL',
            '--security-opt', 'no-new-privileges:true', '--mount', mount,
            '--entrypoint', '/app/panda-keys', image, 'generate', '/custody/keys'])
    return settings


def provision_database(settings):
    """Prepare a managed fresh database before the restricted app migrates it.

    Also used by isolated acceptance. This does not adopt an older installation,
    reset existing passwords, alter existing role privileges, or run migrations.
    """
    if settings.get('database_provisioning') != 1:
        raise ValueError('This installation predates separate database provisioning. '
                         'Retain its data and keys; review the deployment guide before conversion.')
    secret = ROOT / '.deploy/database-bootstrap-password'
    if secret.is_symlink() or not secret.is_file() or secret.stat().st_mode & 0o077:
        raise ValueError('The owner-only database bootstrap secret is missing or has unsafe permissions. '
                         'Existing installation data was retained.')
    values = dict(line.split('=', 1) for line in (ROOT / '.env').read_text().splitlines() if '=' in line)
    password = values.get('POSTGRES_PASSWORD', '')
    if values.get('POSTGRES_USER') != 'panda' or not re.fullmatch(r'[A-Za-z0-9_-]{64,}', password):
        raise ValueError('Managed database credentials do not match the saved installation format.')
    if 'DATABASE_URL' in values or 'POSTGRES_PASSWORD_FILE' in values:
        raise ValueError('Managed installation database overrides require an explicit operator review.')
    compose(settings, ['up', '-d', '--wait', '--wait-timeout', '600', 'timescaledb'])
    # Read the bootstrap password in the DB container, not into the app's env,
    # process arguments, Compose interpolation environment, or Python output.
    bootstrap = ('PGPASSWORD="$(cat /run/secrets/database_bootstrap)" '
                 'exec psql -X -q -h timescaledb -U panda_bootstrap -d panda '
                 '-v ON_ERROR_STOP=1')
    sql = (Path(__file__).resolve().parent / 'provision_database.sql').read_text()
    compose(settings, ['exec', '-T', 'timescaledb', '/bin/sh', '-c', bootstrap],
            input_text="\\set app_password '" + password + "'\n" + sql)
    # The service address exercises host password authentication; PostgreSQL's
    # local/loopback trust rules would accept even an incorrect saved password.
    verify = ('IFS= read -r PGPASSWORD; export PGPASSWORD; '
              'exec psql -X -q -t -A -h timescaledb -U panda -d panda -v ON_ERROR_STOP=1')
    result = compose(settings, ['exec', '-T', 'timescaledb', '/bin/sh', '-c', verify],
                     capture=True, input_text=password + "\nSELECT current_user;\n")
    if result != 'panda':
        raise RuntimeError('Restricted application database login could not be verified. '
                           'Existing installation data was retained.')


def start(settings):
    require_local_docker()
    provision_database(settings)
    compose(settings, ['up', '-d', '--wait', '--wait-timeout', '600'])
    deadline = time.monotonic() + 180
    while True:
        try:
            setup = local_api(settings['port'], '/v1/auth/setup', origin=settings['origin'])
            break
        except (HTTPError, URLError, TimeoutError):
            if time.monotonic() >= deadline:
                raise RuntimeError('The local API is not ready. Configuration and volumes were retained.') from None
            time.sleep(2)
    if not isinstance(setup, dict) or type(setup.get('setup_required')) is not bool:
        raise RuntimeError('Invalid setup response. The setup proof and installation were retained.')
    if setup['setup_required']:
        if setup.get('setup_proof_required') is not True or setup.get('setup_proof_available') is not True:
            raise RuntimeError('Production setup protection is unavailable. Configuration was retained.')
        email = input('First-owner email: ').strip()
        password = getpass.getpass('First-owner password (at least 16 characters): ')
        if len(password) < 16 or password != getpass.getpass('Repeat password: '):
            raise ValueError('Passwords must match and contain at least 16 characters. Run start to retry.')
        values = dict(line.split('=', 1) for line in (ROOT / '.env').read_text().splitlines() if '=' in line)
        try:
            local_api(settings['port'], '/v1/auth/register', origin=settings['origin'],
                      body={'email': email, 'password': password, 'setup_token': values['PANDA_SETUP_TOKEN']})
            confirmed = local_api(settings['port'], '/v1/auth/setup', origin=settings['origin'])
            if not isinstance(confirmed, dict) or confirmed.get('setup_required') is not False:
                raise RuntimeError('Owner creation was not confirmed. The setup proof was retained.')
        except (HTTPError, URLError, TimeoutError):
            raise RuntimeError('Owner creation was not confirmed. Run start to check and retry; no data was deleted.') from None
    env_path = ROOT / '.env'
    content = env_path.read_text()
    updated = ''.join(line + '\n' for line in content.splitlines() if not line.startswith('PANDA_SETUP_TOKEN='))
    if updated != content:
        descriptor, name = tempfile.mkstemp(prefix='env.next.', dir=ROOT / '.deploy')
        temporary = Path(name)
        try:
            with os.fdopen(descriptor, 'w') as output:
                output.write(updated)
                output.flush()
                os.fsync(output.fileno())
            temporary.replace(env_path)
        finally:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass
        compose(settings, ['up', '-d', '--wait', '--wait-timeout', '600'])
    print('Local installation is ready. Sign in at ' + settings['origin'])
    print('Your authenticated HTTPS gateway must forward to 127.0.0.1:' + str(settings['port']))
    print('Back up the ' + settings['project'] + '-keys Docker volume separately from application data.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', nargs='?', choices=['setup', 'start'], default='setup')
    parser.add_argument('--origin', help='Exact URL of your existing authenticated HTTPS gateway')
    parser.add_argument('--project', default='vehicle-history')
    parser.add_argument('--port', type=int, default=8080)
    parser.add_argument('--image', help='Optional reviewed image@sha256:DIGEST; otherwise build this checkout')
    args = parser.parse_args()
    if args.command == 'setup':
        if not args.origin:
            parser.error('--origin is required for setup')
        settings = prepare(args.project, args.origin, args.port, args.image)
    else:
        settings = json.loads((ROOT / '.deploy/settings.json').read_text())
    start(settings)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, OSError, KeyError) as error:
        print(str(error))
        raise SystemExit(1)

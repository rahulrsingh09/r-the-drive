import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = process.cwd();
const temporaryRoot = mkdtempSync(path.join(tmpdir(), 'panda-compose-check-'));
const environmentFile = path.join(temporaryRoot, '.env');
const dataRoot = path.join(temporaryRoot, 'data').replaceAll('\\', '/');

writeFileSync(
  environmentFile,
  [
    'PANDA_IMAGE=panda:test-custody',
    'POSTGRES_PASSWORD=compose-check-database-password',
    'REDIS_PASSWORD=compose-check-redis-password',
    'ALLOWED_ORIGINS=https://panda.example.net',
    'PANDA_ENV_FILE=' + environmentFile.replaceAll('\\', '/'),
    'PANDA_DATA_DIR=' + dataRoot,
    'PANDA_ORIGIN_PORT=18080',
    'ALLOW_INSECURE_LAN_HTTP_AUTH=false',
    'TZ=UTC',
    '',
  ].join('\n'),
  'utf8'
);

try {
  const environment = { ...process.env };
  delete environment.PANDA_HOST_BIND_ADDRESS;
  delete environment.PANDA_IMAGE;
  const render = (files, overrides = {}) => JSON.parse(execFileSync(
      'docker',
      ['compose', '--env-file', environmentFile, ...files.flatMap((file) => ['-f', file]), 'config', '--format', 'json'],
      { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env: { ...environment, ...overrides } }
    ));
  const standardFile = 'compose/docker-compose.yml';
  const standard = render([standardFile]);
  assert.equal(standard.services.panda.ports[0].host_ip, '127.0.0.1');
  assert.equal(standard.services.timescaledb.ports, undefined);
  assert.equal(standard.services.redis.ports, undefined);
  assert.throws(() => render([standardFile], { PANDA_IMAGE: '' }), /Set PANDA_IMAGE/);
  const keyed = render([standardFile, 'compose/docker-compose.keys.yml'], {
    PANDA_KEYS_SOURCE: temporaryRoot,
  });
  const mount = keyed.services.panda.volumes.find((entry) => entry.target === '/run/secrets/panda');
  assert.equal(mount.read_only, true);
  assert.equal(mount.bind?.create_host_path ?? false, false);
  assert.equal(keyed.services.panda.environment.JWT_SECRET_FILE, '/run/secrets/panda/jwt_private.pem');
  assert.equal(keyed.services.panda.environment.JWT_PUBLIC_KEY_FILE, '/run/secrets/panda/jwt_public.pem');
  assert.equal(keyed.services.panda.environment.AGE_ENCRYPTION_KEY_FILE, '/run/secrets/panda/age_key.txt');
  assert.throws(() => render([standardFile, 'compose/docker-compose.keys.yml'], { PANDA_KEYS_SOURCE: '' }), /Set PANDA_KEYS_SOURCE/);
  const built = render([standardFile, 'compose/docker-compose.build.yml'], { PANDA_IMAGE: 'panda:local' });
  assert.equal(built.services.panda.image, 'panda:local');
  console.log('compose:render-check passed');
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

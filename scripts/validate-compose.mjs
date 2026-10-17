import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { isMain } from './lib/validation-files.mjs';

export function validateCompose(root, inherited = process.env) {
  const temporary = mkdtempSync(join(tmpdir(), 'panda-compose-check-'));
  try {
    const environmentFile = join(temporary, '.env').replaceAll('\\', '/');
    const values = {
      PANDA_IMAGE: 'panda:test-custody',
      POSTGRES_PASSWORD: 'compose-check-database-password',
      REDIS_PASSWORD: 'compose-check-redis-password',
      ALLOWED_ORIGINS: 'https://panda.example.net',
      PANDA_ENV_FILE: environmentFile,
      PANDA_DATA_DIR: join(temporary, 'data').replaceAll('\\', '/'),
      PANDA_ORIGIN_PORT: '18080',
      ALLOW_INSECURE_LAN_HTTP_AUTH: 'false',
      TZ: 'UTC',
    };
    writeFileSync(
      environmentFile,
      Object.entries(values)
        .map(([key, value]) => `${key}=${value}\n`)
        .join('')
    );
    const env = Object.fromEntries(
      Object.entries(inherited).filter(
        ([key]) =>
          !/^(?:PANDA_|POSTGRES_|REDIS_|COMPOSE_|ALLOWED_ORIGINS$|ALLOW_INSECURE_LAN_HTTP_AUTH$|JWT_|AGE_)/.test(
            key
          )
      )
    );
    const render = (overlay, overrides = {}) => {
      const files = [
        'compose/docker-compose.yml',
        ...(overlay ? ['compose/docker-compose.' + overlay + '.yml'] : []),
      ];
      const args = [
        'compose',
        '--env-file',
        environmentFile,
        ...files.flatMap((file) => ['-f', file]),
        'config',
        '--format',
        'json',
      ];
      const result = spawnSync('docker', args, {
        cwd: root,
        env: { ...env, ...overrides },
        encoding: 'utf8',
      });
      if (result.error || result.status !== 0)
        throw new Error(
          result.stderr?.trim() || result.error?.message || 'Compose rendering failed'
        );
      return JSON.parse(result.stdout).services;
    };
    const normal = render();
    assert.equal(normal.panda.ports[0].host_ip, '127.0.0.1');
    for (const name of ['timescaledb', 'redis']) assert.equal(normal[name].ports, undefined);
    assert.throws(() => render(undefined, { PANDA_IMAGE: '' }), /Set PANDA_IMAGE/);
    const keyed = render('keys', { PANDA_KEYS_SOURCE: temporary }).panda;
    const mount = keyed.volumes.find((entry) => entry.target === '/run/secrets/panda');
    assert.equal(mount.read_only, true);
    assert.equal(mount.bind?.create_host_path ?? false, false);
    for (const [name, file] of [
      ['JWT_SECRET_FILE', 'jwt_private.pem'],
      ['JWT_PUBLIC_KEY_FILE', 'jwt_public.pem'],
      ['AGE_ENCRYPTION_KEY_FILE', 'age_key.txt'],
    ])
      assert.equal(keyed.environment[name], `/run/secrets/panda/${file}`);
    assert.throws(() => render('keys', { PANDA_KEYS_SOURCE: '' }), /Set PANDA_KEYS_SOURCE/);
    assert.equal(render('build', { PANDA_IMAGE: 'panda:local' }).panda.image, 'panda:local');
    console.log('compose:render-check passed');
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

if (isMain(import.meta.url)) validateCompose(process.cwd());

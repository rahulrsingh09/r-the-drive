import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parse } from 'yaml';
import { isMain } from './lib/validation-files.mjs';

export function composeContractFailures(root) {
  const contents = ['docker-compose.yml', 'docker-compose.build.yml', 'docker-compose.dev.yml'].map(
    (name) => readFileSync(join(root, 'compose', name), 'utf8')
  );
  const [standard, build, dev] = contents.map((text) => parse(text));
  const services = standard.services ?? {};
  const app = services.panda ?? {};
  const db = services.timescaledb ?? {};
  const expectedServices = new Set(['panda', 'redis', 'timescaledb']);
  const checks = [
    [
      Object.keys(services).length === expectedServices.size &&
        Object.keys(services).every((name) => expectedServices.has(name)),
      'standard Compose must contain only the universal production services',
    ],
    [
      app.ports?.includes('${PANDA_HOST_BIND_ADDRESS:-127.0.0.1}:${PANDA_ORIGIN_PORT:-8080}:8080'),
      'standard Compose must use configurable host publication',
    ],
    [
      dev.services?.api?.environment?.COOKIE_INSECURE === 'true' &&
        dev.services?.api?.environment?.PANDA_ENV === 'development',
      'development Compose must omit Secure refresh cookies for HTTP browser reloads',
    ],
    [
      !Object.hasOwn(app.environment ?? {}, 'COOKIE_INSECURE'),
      'standard Compose must not enable development-only insecure cookies',
    ],
    [
      !Object.hasOwn(services, 'panda-init'),
      'standard Compose must not contain a completed init service',
    ],
    [
      build.services?.panda?.image === 'panda:local',
      'source-build overlay must run panda from the candidate image',
    ],
    [
      app.volumes?.some((volume) => volume.includes('PANDA_BACKUPS_SOURCE')),
      'backup storage must be independently configurable',
    ],
    [
      db.volumes?.some((volume) => volume.includes('PANDA_DB_SOURCE')),
      'database storage must be independently configurable',
    ],
    [
      db.healthcheck?.start_period === '300s' && db.healthcheck?.retries === 30,
      'database healthcheck must tolerate slow first initialization',
    ],
    [
      ['panda-db', 'panda-redis', 'panda-backups', 'panda-cache'].every((name) =>
        Object.hasOwn(standard.volumes ?? {}, name)
      ),
      'standard Compose must declare default Docker-managed volumes',
    ],
    [!contents[0].includes('pids:'), 'standard Compose must not set a host PID limit'],
    [!contents[0].includes('cpus:'), 'standard Compose must not set a host CPU quota'],
    [
      ['compose/docker-compose.synology.yml', 'compose/synology/docker-compose.yml'].every(
        (file) => !existsSync(join(root, file))
      ),
      'Synology must not have a second authored Compose file',
    ],
  ];
  return checks.filter(([valid]) => !valid).map(([, message]) => message);
}

if (isMain(import.meta.url)) {
  const failures = composeContractFailures(process.cwd());
  if (failures.length) throw new Error(`compose:check failed: ${failures[0]}`);
  console.log('compose:check passed');
}

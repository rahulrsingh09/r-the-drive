import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import {
  assertCatalogExtension,
  migrationIdentity,
  readPrivateCatalog,
} from './lib/private-migrations.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const base = process.argv[2] || 'HEAD';
const git = (...args) => execFileSync('git', args, { cwd: root });
const current = readPrivateCatalog(root).map((entry) => entry.identity);
const paths = git(
  'ls-tree',
  '-r',
  '--name-only',
  base,
  '--',
  'apps/api/migrations',
  'apps/api/migrations-private'
)
  .toString()
  .trim()
  .split('\n')
  .filter((path) => path.endsWith('.sql'));
const legacyCatalog = !paths.some((path) => path.includes('/migrations-private/'));
const previous = paths
  .map((path) => {
    const fileName = path.split('/').at(-1);
    const privateMigration =
      path.includes('/migrations-private/') || fileName === '0028_active_trip_checkpoints.sql';
    const identity = migrationIdentity(fileName, git('show', `${base}:${path}`), privateMigration);
    if (legacyCatalog && !privateMigration && identity.version >= 1_000_000) {
      throw new Error(
        'Choose a fork baseline; upstream databases use a different migration ledger'
      );
    }
    return identity;
  })
  .sort((a, b) => a.version - b.version);
assertCatalogExtension(previous, current);
console.log(
  JSON.stringify({
    base,
    migrations: current.length,
    catalogDigest: createHash('sha256').update(JSON.stringify(current)).digest('hex'),
    latestVersion: current.at(-1)?.version,
  })
);

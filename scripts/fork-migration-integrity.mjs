import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { assertCatalogExtension, migrationIdentity, readPrivateCatalog } from './lib/private-migrations.mjs';
import { direct } from './operational-runtime.mjs';

export function migrationReport(base = 'HEAD', root = fileURLToPath(new URL('..', import.meta.url)), execute = execFileSync) {
  const readGit = args => execute('git', args, { cwd: root });
  const current = Array.from(readPrivateCatalog(root), entry => entry.identity);
  const listing = readGit(['ls-tree', '-r', '--name-only', base, '--',
    'apps/api/migrations', 'apps/api/migrations-private']).toString().trim().split('\n');
  const sqlPaths = listing.filter(name => name.endsWith('.sql'));
  const hasPrivateDirectory = sqlPaths.some(name => name.includes('/migrations-private/'));
  const previous = [];
  for (const name of sqlPaths) {
    const file = name.substring(name.lastIndexOf('/') + 1);
    const privateEntry = file === '0028_active_trip_checkpoints.sql' || name.includes('/migrations-private/');
    const entry = migrationIdentity(file, readGit(['show', `${base}:${name}`]), privateEntry);
    if (!hasPrivateDirectory && !privateEntry && entry.version >= 1_000_000)
      throw new Error('Choose a fork baseline; upstream databases use a different migration ledger');
    previous.push(entry);
  }
  previous.sort((left, right) => left.version - right.version);
  assertCatalogExtension(previous, current);
  return { base, migrations: current.length,
    catalogDigest: createHash('sha256').update(JSON.stringify(current)).digest('hex'),
    latestVersion: current.at(-1)?.version };
}
if (direct(import.meta.url)) console.log(JSON.stringify(migrationReport(process.argv[2] || 'HEAD')));

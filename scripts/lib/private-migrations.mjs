import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const LEGACY_PRIVATE_FILE = '0028_active_trip_checkpoints.sql';
const UPSTREAM_OFFSET = 1_000_000;

export function migrationIdentity(fileName, bytes, privateMigration = false) {
  const separator = fileName.indexOf('_');
  const number = fileName.slice(0, separator);
  const description = fileName.slice(separator + 1, -4);
  if (
    !/^\d{4,}$/.test(number) ||
    !/^[a-z][a-z0-9_]*$/.test(description) ||
    !fileName.endsWith('.sql')
  )
    throw new Error(`Invalid migration name: ${fileName}`);
  const version = Number(number);
  if (!(Number.isSafeInteger(version) && version > 0 && version < UPSTREAM_OFFSET))
    throw new Error(`Migration exceeds its namespace: ${fileName}`);
  if (privateMigration && fileName !== LEGACY_PRIVATE_FILE)
    throw new Error(
      'New private schema migrations require an explicit catalog compatibility review'
    );
  new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  const forbiddenByte = bytes.some((value) => value === 0 || value === 13);
  const bom = bytes.length >= 3 && bytes[0] === 239 && bytes[1] === 187 && bytes[2] === 191;
  if (forbiddenByte || bom || bytes[bytes.length - 1] !== 10)
    throw new Error(`Migration must be nonempty UTF-8, without BOM/NUL, and use LF: ${fileName}`);
  return {
    version: privateMigration || version < 28 ? version : UPSTREAM_OFFSET + version,
    description: description.split('_').join(' '),
    checksum_sha384: createHash('sha384').update(bytes).digest('hex'),
  };
}

export function readPrivateCatalog(root) {
  const versions = new Map();
  let privateCount = 0;
  let duplicate = false;
  for (const [directory, privateMigration] of [
    ['migrations', false],
    ['migrations-private', true],
  ]) {
    const base = join(root, 'apps/api', directory);
    for (const entry of readdirSync(base, { withFileTypes: true })) {
      if (!(entry.isFile() && entry.name.endsWith('.sql'))) continue;
      const identity = migrationIdentity(
        entry.name,
        readFileSync(join(base, entry.name)),
        privateMigration
      );
      duplicate ||= versions.has(identity.version);
      versions.set(identity.version, { path: `apps/api/${directory}/${entry.name}`, identity });
      privateCount += Number(privateMigration);
    }
  }
  if (duplicate) throw new Error('Duplicate effective migration versions');
  if (privateCount !== 1)
    throw new Error('The deployed private checkpoint migration must be preserved');
  return [...versions.keys()].sort((a, b) => a - b).map((version) => versions.get(version));
}

export function assertCatalogExtension(previous, current) {
  if (current.length < previous.length) throw new Error('Applied migrations were removed');
  const serialized = current.map((identity) => JSON.stringify(identity));
  const changed = previous.findIndex(
    (identity, index) => JSON.stringify(identity) !== serialized[index]
  );
  if (changed !== -1)
    throw new Error(`Applied migration ${previous[changed].version} changed or was reordered`);
}

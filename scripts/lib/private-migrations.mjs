import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const LEGACY_PRIVATE_FILE = '0028_active_trip_checkpoints.sql';
const UPSTREAM_OFFSET = 1_000_000;

export function migrationIdentity(fileName, bytes, privateMigration = false) {
  const match = /^(\d{4,})_([a-z][a-z0-9_]*)\.sql$/.exec(fileName);
  if (!match) throw new Error(`Invalid migration name: ${fileName}`);
  const sourceVersion = Number(match[1]);
  if (
    !Number.isSafeInteger(sourceVersion) ||
    sourceVersion < 1 ||
    sourceVersion >= UPSTREAM_OFFSET
  ) {
    throw new Error(`Migration exceeds its namespace: ${fileName}`);
  }
  if (privateMigration && fileName !== LEGACY_PRIVATE_FILE) {
    throw new Error(
      'New private schema migrations require an explicit catalog compatibility review'
    );
  }
  new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  if (
    !bytes.length ||
    bytes.includes(0x0d) ||
    bytes.includes(0) ||
    bytes.at(-1) !== 0x0a ||
    bytes.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf]))
  ) {
    throw new Error(`Migration must be nonempty UTF-8, without BOM/NUL, and use LF: ${fileName}`);
  }
  return {
    version: sourceVersion + (!privateMigration && sourceVersion >= 28 ? UPSTREAM_OFFSET : 0),
    description: match[2].replaceAll('_', ' '),
    checksum_sha384: createHash('sha384').update(bytes).digest('hex'),
  };
}

export function readPrivateCatalog(root) {
  const entries = [];
  for (const directory of ['migrations', 'migrations-private']) {
    const dir = join(root, 'apps/api', directory);
    for (const file of readdirSync(dir, { withFileTypes: true })) {
      if (!file.isFile() || !file.name.endsWith('.sql')) continue;
      const bytes = readFileSync(join(dir, file.name));
      entries.push({
        path: `apps/api/${directory}/${file.name}`,
        identity: migrationIdentity(file.name, bytes, directory === 'migrations-private'),
      });
    }
  }
  entries.sort((a, b) => a.identity.version - b.identity.version);
  if (new Set(entries.map((e) => e.identity.version)).size !== entries.length) {
    throw new Error('Duplicate effective migration versions');
  }
  if (entries.filter((e) => e.path.includes('/migrations-private/')).length !== 1) {
    throw new Error('The deployed private checkpoint migration must be preserved');
  }
  return entries;
}

export function assertCatalogExtension(previous, current) {
  if (previous.length > current.length) throw new Error('Applied migrations were removed');
  for (const [index, identity] of previous.entries()) {
    if (JSON.stringify(identity) !== JSON.stringify(current[index])) {
      throw new Error(`Applied migration ${identity.version} changed or was reordered`);
    }
  }
}

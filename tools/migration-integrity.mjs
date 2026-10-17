#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isMain } from '../scripts/lib/validation-files.mjs';

const migrationPath = 'apps/api/migrations';
function fail(message) {
  throw new Error(`migration integrity failed: ${message}`);
}

export function parseMigrationFile(fileName) {
  const [stem, extension, ...extra] = fileName.split('.');
  const separator = stem.indexOf('_');
  const prefix = stem.slice(0, separator);
  const description = stem.slice(separator + 1);
  if (
    extension !== 'sql' ||
    extra.length ||
    !/^\d{4,}$/.test(prefix) ||
    !/^[a-z][a-z0-9_]*$/.test(description)
  )
    fail(`${fileName} must match NNNN_description.sql using lowercase names`);
  const version = Number(prefix);
  if (!Number.isSafeInteger(version) || version <= 0) fail(`${fileName} has an invalid version`);
  return { fileName, version, description };
}

function validateBytes(fileName, bytes) {
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    fail(`${fileName} is not valid UTF-8`);
  }
  const errors = [
    [
      bytes.length >= 3 && bytes[0] === 239 && bytes[1] === 187 && bytes[2] === 191,
      'must not contain a UTF-8 BOM',
    ],
    [bytes.indexOf(13) !== -1, 'contains CRLF or a lone CR; migrations must use LF'],
    [bytes.length === 0, 'is empty'],
    [bytes[bytes.length - 1] !== 10, 'must end with one LF newline'],
    [bytes.indexOf(0) !== -1, 'contains a NUL byte'],
  ];
  const error = errors.find(([invalid]) => invalid);
  if (error) fail(`${fileName} ${error[1]}`);
}

export function currentCatalog(root) {
  const directory = resolve(root, migrationPath);
  const names = readdirSync(directory, { withFileTypes: true }).filter(
    (entry) => entry.isFile() && entry.name.endsWith('.sql')
  );
  const ordered = names
    .map((entry) => parseMigrationFile(entry.name))
    .sort((a, b) => a.version - b.version);
  if (!ordered.length) fail('no SQL migrations were found');
  if (ordered[0].version !== 1) fail('migration catalog must start at version 0001');
  const duplicate = ordered.find(
    (file, index) => index && file.version === ordered[index - 1].version
  );
  if (duplicate) fail(`duplicate migration version ${duplicate.version}`);
  const gap = ordered.findIndex(
    (file, index) => index && file.version !== ordered[index - 1].version + 1
  );
  if (gap !== -1)
    fail(
      `migration versions must be contiguous and ordered: ${ordered[gap - 1].fileName}, ${ordered[gap].fileName}`
    );
  const migrations = ordered.map((file) => {
    const bytes = readFileSync(resolve(directory, file.fileName));
    validateBytes(file.fileName, bytes);
    return { ...file, checksumSha384: createHash('sha384').update(bytes).digest('hex'), bytes };
  });
  const ledger = migrations.map((file) => ({
    version: file.version,
    description: file.description,
    checksum_sha384: file.checksumSha384,
  }));
  return { migrations, digest: createHash('sha256').update(JSON.stringify(ledger)).digest('hex') };
}

function repository(root) {
  return (args, optional = false, binary = false) => {
    const result = spawnSync('git', args, {
      cwd: root,
      encoding: binary ? undefined : 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    if (!result.error && result.status === 0) return result.stdout;
    if (optional) return null;
    const detail = String(result.stderr ?? '').trim();
    fail(`git ${args.join(' ')} failed${detail ? `: ${detail}` : ''}`);
  };
}

export function enforceMergeBaseImmutability(root, catalog, explicit) {
  const git = repository(root);
  const candidates = [
    explicit,
    explicit && `origin/${explicit}`,
    'origin/main',
    'main',
    'HEAD^',
  ].filter(Boolean);
  const baseRef = candidates.find((ref) => git(['rev-parse', '--verify', `${ref}^{commit}`], true));
  if (!baseRef) {
    console.warn('migration integrity: no merge base was available; current-tree checks still ran');
    return;
  }
  const names = git(['ls-tree', '-r', '--name-only', baseRef, '--', migrationPath])
    .split(/\r?\n/)
    .filter((name) => name.endsWith('.sql'))
    .map((name) => name.slice(migrationPath.length + 1));
  const historical = names.map(parseMigrationFile);
  const byName = new Map(catalog.migrations.map((file) => [file.fileName, file]));
  const oldVersions = historical.map((file) => file.version).sort((a, b) => a - b);
  const cutover =
    oldVersions.join(',') === '1,2,3,4,5' &&
    catalog.migrations.length === 1 &&
    catalog.migrations[0].version === 1;
  const originalBytes = (name) => git(['show', `${baseRef}:${migrationPath}/${name}`], false, true);
  if (
    cutover &&
    catalog.migrations[0].bytes.equals(
      originalBytes(historical.find((file) => file.version === 1).fileName)
    )
  )
    fail(
      'the one-time baseline cutover must replace 0001_initial_schema.sql with the complete schema'
    );
  const changed = [];
  for (const old of historical) {
    const current = byName.get(old.fileName);
    if (!current) {
      if (!cutover || old.version === 1)
        changed.push(`deleted previously merged migration ${old.fileName}`);
    } else if (
      !current.bytes.equals(originalBytes(old.fileName)) &&
      !(cutover && old.version === 1)
    ) {
      changed.push(`modified previously merged migration ${old.fileName}`);
    }
  }
  const last = Math.max(0, ...oldVersions);
  for (const file of catalog.migrations) {
    if (!cutover && file.version <= last && !names.includes(file.fileName))
      changed.push(`added migration ${file.fileName} without appending after version ${last}`);
  }
  if (changed.length) fail(changed.join('; '));
  if (cutover)
    console.warn(
      `migration integrity: recognized the explicit one-time baseline cutover from ${names.length} migrations at ${baseRef}`
    );
  console.log(`migration integrity: merge-base immutability passed against ${baseRef}`);
}

if (isMain(import.meta.url)) {
  const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
  const args = process.argv.slice(2);
  const index = args.indexOf('--base-ref');
  const base =
    (index < 0 ? undefined : args[index + 1]) ||
    process.env.GITHUB_BASE_SHA ||
    process.env.GITHUB_BASE_REF;
  const catalog = currentCatalog(root);
  enforceMergeBaseImmutability(root, catalog, base);
  console.log(
    `migration integrity: ${catalog.migrations.length} migration(s), catalog digest ${catalog.digest}`
  );
}

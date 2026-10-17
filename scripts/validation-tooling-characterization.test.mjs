import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  migrationIdentity,
  readPrivateCatalog,
  assertCatalogExtension,
} from './lib/private-migrations.mjs';
import {
  assertDisposableUpgradeDatabase,
  assertLocalDatabaseResetAllowed,
} from './lib/database-reset-guard.mjs';
import { nextReleaseVersion } from './next-release-version.mjs';
import { latestRelease, resolveBuildVersion } from './resolve-build-version.mjs';
import { evaluateAudit } from './check-dependency-audits.mjs';

const sql = Buffer.from('SELECT 1;\n');
const resetEnv = {
  PANDA_ALLOW_DATABASE_RESET: 'local-development',
  DATABASE_URL: 'postgresql://synthetic@localhost/example',
};

test('reset refusal priority, URL parse errors, IPv6, default port and production NODE_ENV', () => {
  for (const [env, args, message] of [
    [{ PANDA_ENV: 'production' }, [], /production/],
    [{ NODE_ENV: 'production', ...resetEnv }, ['--yes'], /production/],
    [{}, [], /PANDA_ALLOW_DATABASE_RESET/],
    [resetEnv, [], /--yes/],
    [{ ...resetEnv, DATABASE_URL: 'invalid' }, ['--yes'], /valid PostgreSQL URL/],
    [{ ...resetEnv, DATABASE_URL: 'postgresql://localhost.example/data' }, ['--yes'], /loopback/],
  ])
    assert.throws(() => assertLocalDatabaseResetAllowed({ env, args }), message);
  for (const host of ['localhost', '127.0.0.1', '[::1]']) {
    assert.deepEqual(
      assertLocalDatabaseResetAllowed({
        args: ['--yes'],
        env: { ...resetEnv, DATABASE_URL: `postgres://${host}/example` },
      }),
      { database: 'example', host, port: '5432' }
    );
  }
});

test('upgrade database name cannot escape its disposable prefix', () => {
  for (const name of [
    'panda',
    'panda_upgrade_',
    'panda_upgrade-a',
    'panda_upgrade_A',
    'panda_upgrade/x',
    'panda_upgrade%2ftarget',
  ]) {
    assert.throws(
      () =>
        assertDisposableUpgradeDatabase({ UPGRADE_DATABASE_URL: `postgresql://localhost/${name}` }),
      /disposable/
    );
  }
  assert.equal(
    assertDisposableUpgradeDatabase({
      UPGRADE_DATABASE_URL: 'postgresql://[::1]/panda_upgrade_case_1',
    }),
    'postgresql://[::1]/panda_upgrade_case_1'
  );
});

test('migration identities preserve byte digest, description and public/private version boundary', () => {
  for (const [file, privateMigration, expected] of [
    ['0027_last.sql', false, 27],
    ['0028_first.sql', false, 1000028],
    ['0028_active_trip_checkpoints.sql', true, 28],
    ['999999_last.sql', false, 1999999],
  ]) {
    assert.deepEqual(migrationIdentity(file, sql, privateMigration), {
      version: expected,
      description: file.slice(file.indexOf('_') + 1, -4).replaceAll('_', ' '),
      checksum_sha384: createHash('sha384').update(sql).digest('hex'),
    });
  }
});

test('migration validation refuses every invalid byte and identity class', () => {
  for (const name of [
    '1_short.sql',
    '0000_zero.sql',
    '0001_Upper.sql',
    '0001_bad-name.sql',
    '1000000_overflow.sql',
    '99999999999999999_overflow.sql',
  ])
    assert.throws(() => migrationIdentity(name, sql));
  for (const bytes of [
    Buffer.alloc(0),
    Buffer.from('SELECT 1;'),
    Buffer.from('SELECT 1;\r\n'),
    Buffer.from([0xff, 10]),
    Buffer.from([0xef, 0xbb, 0xbf, 10]),
    Buffer.from([0, 10]),
  ])
    assert.throws(() => migrationIdentity('0001_initial.sql', bytes));
});

test('catalog rejects duplicate effective versions and missing deployed checkpoint', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'validation-catalog-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const folder of ['migrations', 'migrations-private'])
    mkdirSync(join(root, 'apps/api', folder), { recursive: true });
  writeFileSync(join(root, 'apps/api/migrations/0001_initial.sql'), sql);
  assert.throws(() => readPrivateCatalog(root), /checkpoint/);
  writeFileSync(join(root, 'apps/api/migrations-private/0028_active_trip_checkpoints.sql'), sql);
  assert.deepEqual(
    readPrivateCatalog(root).map((e) => e.identity.version),
    [1, 28]
  );
  writeFileSync(join(root, 'apps/api/migrations/0001_duplicate.sql'), sql);
  assert.throws(() => readPrivateCatalog(root), /Duplicate/);
});

test('catalog extension remains strict about all serialized identity fields', () => {
  const identity = migrationIdentity('0001_initial.sql', sql);
  assert.throws(
    () => assertCatalogExtension([identity], [{ ...identity, extra: true }]),
    /changed/
  );
  assert.throws(
    () =>
      assertCatalogExtension(
        [identity],
        [
          {
            checksum_sha384: identity.checksum_sha384,
            description: identity.description,
            version: identity.version,
          },
        ]
      ),
    /changed/
  );
  assert.doesNotThrow(() => assertCatalogExtension([], [identity]));
});

test('calendar versions ignore previews, retain stable tie ordering, and advance matching months', () => {
  assert.equal(
    nextReleaseVersion(['2026.10.09', '2026.10.10-rc', '2026.09.100'], '2026.10'),
    '2026.10.10'
  );
  for (const month of ['2026.1', '2026.00', '2026.13', '26.10', '2026.10.1'])
    assert.throws(() => nextReleaseVersion([], month), /Invalid Calendar Version month/);
  assert.equal(latestRelease(['2026.10.01', '2026.10.1']), '2026.10.01');
  assert.equal(resolveBuildVersion(['2026.10.1', '2026.10.2'], ['2026.10.1']), '2026.10.2+dev');
  assert.equal(resolveBuildVersion([], [], 0), 'unknown');
});

test('audit checks all exception metadata, review ordering, stale and exact scope', () => {
  const report = {
    settings: { ignore: [] },
    vulnerabilities: { list: [], count: 0 },
    warnings: {},
  };
  const entry = {
    ecosystem: 'cargo',
    id: 'RUSTSEC-synthetic',
    package: 'x',
    version: '1',
    owner: 'Maintainer',
    reviewed: '2026-10-01',
    expires: '2026-10-20',
    source: 'local fixture',
    reason: 'Synthetic only',
    removal: 'On fix',
  };
  for (const field of [
    'id',
    'package',
    'version',
    'owner',
    'reviewed',
    'expires',
    'source',
    'reason',
    'removal',
  ])
    assert.throws(
      () => evaluateAudit('cargo', report, [{ ...entry, [field]: '' }], '2026-10-17', 0),
      /Incomplete/
    );
  for (const reviewed of ['2026-10-18', '2026-10-21'])
    assert.throws(
      () => evaluateAudit('cargo', report, [{ ...entry, reviewed }], '2026-10-17', 0),
      /review date/
    );
  assert.match(evaluateAudit('cargo', report, [entry], '2026-10-17', 0).failures[0], /stale/);
  assert.throws(() =>
    evaluateAudit(
      'cargo',
      { ...report, vulnerabilities: { list: [], count: 1 } },
      [],
      '2026-10-17',
      0
    )
  );
});

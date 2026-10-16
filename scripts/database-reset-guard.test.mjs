import assert from 'node:assert/strict';
import test from 'node:test';

import {
  assertDisposableUpgradeDatabase,
  assertLocalDatabaseResetAllowed,
} from './lib/database-reset-guard.mjs';

const localEnv = {
  DATABASE_URL: 'postgresql://panda:secret@127.0.0.1:5435/panda',
  PANDA_ALLOW_DATABASE_RESET: 'local-development',
  PANDA_ENV: 'development',
};

test('database reset requires both acknowledgement gates', () => {
  assert.throws(
    () => assertLocalDatabaseResetAllowed({ args: [], env: localEnv }),
    /explicit --yes/
  );
  assert.throws(
    () =>
      assertLocalDatabaseResetAllowed({
        args: ['--yes'],
        env: { ...localEnv, PANDA_ALLOW_DATABASE_RESET: undefined },
      }),
    /PANDA_ALLOW_DATABASE_RESET/
  );
});

test('database reset refuses production and non-loopback targets', () => {
  assert.throws(
    () =>
      assertLocalDatabaseResetAllowed({
        args: ['--yes'],
        env: { ...localEnv, PANDA_ENV: 'production' },
      }),
    /disabled in production/
  );
  assert.throws(
    () =>
      assertLocalDatabaseResetAllowed({
        args: ['--yes'],
        env: {
          ...localEnv,
          DATABASE_URL: 'postgresql://panda:secret@timescaledb:5432/panda',
        },
      }),
    /limited to loopback hosts/
  );
});

test('database reset allows an explicitly acknowledged local target', () => {
  assert.deepEqual(
    assertLocalDatabaseResetAllowed({ args: ['--yes'], env: localEnv }),
    { database: 'panda', host: '127.0.0.1', port: '5435' }
  );
});

test('populated upgrade harness requires an explicitly disposable loopback database', () => {
  assert.throws(() => assertDisposableUpgradeDatabase({}), /explicit UPGRADE_DATABASE_URL/);
  assert.throws(
    () =>
      assertDisposableUpgradeDatabase({
        UPGRADE_DATABASE_URL: 'postgresql://panda:secret@db.example/panda_upgrade',
      }),
    /limited to loopback hosts/
  );
  assert.throws(
    () =>
      assertDisposableUpgradeDatabase({
        UPGRADE_DATABASE_URL: 'postgresql://panda:secret@localhost/panda',
      }),
    /disposable panda_upgrade database/
  );
  assert.equal(
    assertDisposableUpgradeDatabase({
      UPGRADE_DATABASE_URL:
        'postgresql://panda:secret@127.0.0.1:5432/panda_upgrade_arm64',
    }),
    'postgresql://panda:secret@127.0.0.1:5432/panda_upgrade_arm64'
  );
});

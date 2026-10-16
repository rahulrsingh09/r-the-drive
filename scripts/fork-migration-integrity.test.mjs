import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  assertCatalogExtension,
  migrationIdentity,
  readPrivateCatalog,
} from './lib/private-migrations.mjs';
import { fileURLToPath } from 'node:url';

test('the deployed checkpoint and upstream settings have distinct versions with original SQL checksums', () => {
  const bytes = Buffer.from('SELECT 1;\n');
  const legacy = migrationIdentity('0028_active_trip_checkpoints.sql', bytes, true);
  const upstream = migrationIdentity('0028_update_check_settings.sql', bytes);
  assert.equal(legacy.version, 28);
  assert.equal(upstream.version, 1_000_028);
  assert.equal(legacy.checksum_sha384, upstream.checksum_sha384);
});

test('an upgrade preserves historical identity and rejects checksum drift, removal and reordering', () => {
  const before = [migrationIdentity('0001_initial_schema.sql', Buffer.from('SELECT 1;\n'))];
  const next = migrationIdentity('0028_update_check_settings.sql', Buffer.from('SELECT 2;\n'));
  assert.doesNotThrow(() => assertCatalogExtension(before, [...before, next]));
  assert.throws(() => assertCatalogExtension(before, []), /removed/);
  assert.throws(() => assertCatalogExtension(before, [next, ...before]), /changed/);
  assert.throws(
    () => assertCatalogExtension(before, [{ ...before[0], checksum_sha384: 'tampered' }]),
    /changed/
  );
});

test('unknown private migrations and upstream namespace overflow fail closed', () => {
  const bytes = Buffer.from('SELECT 1;\n');
  assert.throws(() => migrationIdentity('0029_other_private.sql', bytes, true), /review/);
  assert.throws(() => migrationIdentity('1000000_collision.sql', bytes), /namespace/);
  assert.throws(
    () => migrationIdentity('0001_initial_schema.sql', Buffer.from('SELECT 1;\r\n')),
    /UTF-8/
  );
});

test('the compiled catalog keeps the entire deployed prefix before incoming upstream migrations', () => {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const catalog = readPrivateCatalog(root).map((entry) => entry.identity);
  assert.deepEqual(
    catalog.slice(0, 28).map((e) => e.version),
    Array.from({ length: 28 }, (_, i) => i + 1)
  );
  assert.deepEqual(
    catalog.slice(28, 31).map((e) => e.version),
    [1_000_028, 1_000_029, 1_000_030]
  );
});

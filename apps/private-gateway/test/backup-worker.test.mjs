import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBackupVault } from '../src/backup-worker.mjs';

const MAX_BYTES = 20 * 1024 * 1024;
const DAILY_SLOTS = 31;

const token = 'a'.repeat(64);
const body = new TextEncoder().encode(
  'age-encryption.org/v1\n' + 'synthetic-encrypted-data'.repeat(12)
);
async function request(overrides = {}) {
  const sha = Buffer.from(await crypto.subtle.digest('SHA-256', body)).toString('hex');
  return new Request('https://vault.synthetic/backup', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/octet-stream',
      'X-Backup-Sha256': sha,
      ...overrides.headers,
    },
    body: overrides.body ?? body,
  });
}
function fixture() {
  const records = new Map([['baseline', { body: 'untouched baseline' }]]);
  const env = {
    HISTORY_BACKUP_TOKEN: token,
    HISTORY_BACKUPS: {
      async put(key, data, options) {
        records.set(key, { body: new Uint8Array(data), ...options });
      },
    },
  };
  return { records, env };
}

test('write-only vault rejects missing/wrong credentials, methods and plaintext', async () => {
  const { env, records } = fixture();
  const vault = createBackupVault();
  for (const auth of ['', `Bearer ${'b'.repeat(64)}`, `Bearer ${token}extra`]) {
    assert.equal(
      (await vault.fetch(await request({ headers: { Authorization: auth } }), env)).status,
      403
    );
  }
  for (const method of ['GET', 'DELETE', 'PUT', 'OPTIONS']) {
    const req = new Request('https://vault.synthetic/backup', {
      method,
      headers: { Authorization: `Bearer ${token}` },
    });
    assert.equal((await vault.fetch(req, env)).status, 404);
  }
  assert.equal(
    (await vault.fetch(await request({ body: 'plaintext vehicle history'.repeat(20) }), env))
      .status,
    400
  );
  assert.equal(
    (await vault.fetch(await request({ headers: { 'X-Backup-Sha256': 'bad' } }), env)).status,
    400
  );
  assert.equal(
    (await vault.fetch(await request({ headers: { 'Content-Type': 'text/plain' } }), env)).status,
    415
  );
  assert.equal(records.size, 1);
});

test('valid archive returns a receipt and rotation bounds storage without touching baseline', async () => {
  const { env, records } = fixture();
  let timestamp = Date.UTC(2026, 9, 7);
  const vault = createBackupVault(() => timestamp);
  for (let day = 0; day < 70; day++) {
    const response = await vault.fetch(await request(), env);
    assert.equal(response.status, 201);
    const receipt = await response.json();
    assert.equal(receipt.bytes, body.length);
    assert.deepEqual(records.get(receipt.key).body, body);
    assert.equal(records.get(receipt.key).metadata.sha256, receipt.sha256);
    timestamp += 86_400_000;
  }
  assert.equal(records.size, DAILY_SLOTS + 1);
  assert.equal(records.get('baseline').body, 'untouched baseline');
  const oldest = Math.min(
    ...[...records.entries()]
      .filter(([k]) => k.startsWith('daily/'))
      .map(([, v]) => Date.parse(v.metadata.createdAt))
  );
  assert.equal(oldest, timestamp - DAILY_SLOTS * 86_400_000);
});

test('oversized streamed bodies and storage failures keep prior backups intact', async () => {
  const { env, records } = fixture();
  const vault = createBackupVault();
  assert.equal(
    (
      await vault.fetch(
        await request({
          headers: { 'Content-Length': String(MAX_BYTES + 1) },
        }),
        env
      )
    ).status,
    413
  );
  assert.equal(
    (await vault.fetch(await request({ body: new Uint8Array(MAX_BYTES + 1) }), env)).status,
    413
  );
  await vault.fetch(await request(), env);
  const prior = [...records.entries()];
  env.HISTORY_BACKUPS.put = async () => {
    throw new Error('quota exceeded with secret diagnostics');
  };
  const response = await vault.fetch(await request(), env);
  assert.equal(response.status, 503);
  assert(!JSON.stringify(await response.json()).includes('secret diagnostics'));
  assert.deepEqual([...records.entries()], prior);
});

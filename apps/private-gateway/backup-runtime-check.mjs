import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
const require = createRequire(import.meta.url);
const { Miniflare } = require(process.env.PANDA_MINIFLARE_MODULE ?? 'miniflare');
const token = 'a'.repeat(64);
const runtime = new Miniflare({
  telemetry: { enabled: false },
  logRequests: false,
  workers: [
    {
      config: {
        name: 'backup-vault',
        compatibilityDate: '2026-10-07',
        manifest: {
          mainModule: 'backup-worker.mjs',
          modules: {
            'backup-worker.mjs': {
              type: 'esm',
              contents: await readFile(
                new URL('./dist/backup-worker.mjs', import.meta.url),
                'utf8'
              ),
            },
          },
        },
        env: {
          HISTORY_BACKUP_TOKEN: { type: 'text', value: token },
          HISTORY_BACKUPS: { type: 'kv', id: 'synthetic-backup-vault' },
        },
      },
    },
  ],
});
try {
  const kv = await runtime.getKVNamespace('HISTORY_BACKUPS', 'backup-vault');
  await kv.put('baseline', 'protected synthetic baseline');
  const body = new TextEncoder().encode('age-encryption.org/v1\n' + 'synthetic'.repeat(200_000));
  const sha256 = Buffer.from(await crypto.subtle.digest('SHA-256', body)).toString('hex');
  const url = 'https://backup.synthetic/backup';
  const response = await runtime.dispatchFetch(url, {
    method: 'POST',
    body,
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/octet-stream',
      'X-Backup-Sha256': sha256,
    },
  });
  assert.equal(response.status, 201);
  const receipt = await response.json();
  assert.equal(receipt.sha256, sha256);
  assert.equal(receipt.bytes, body.length);
  const stored = await kv.getWithMetadata(receipt.key, { type: 'arrayBuffer' });
  assert.deepEqual(new Uint8Array(stored.value), body);
  assert.equal(stored.metadata.sha256, sha256);
  assert.equal(await kv.get('baseline'), 'protected synthetic baseline');
  for (const method of ['GET', 'DELETE']) {
    assert.equal(
      (
        await runtime.dispatchFetch(url, {
          method,
          headers: { Authorization: `Bearer ${token}` },
        })
      ).status,
      404
    );
  }
  assert.equal(
    (
      await runtime.dispatchFetch(url, {
        method: 'POST',
        body,
        headers: { 'Content-Type': 'application/octet-stream' },
      })
    ).status,
    403
  );
  console.log(
    JSON.stringify({
      runtime: 'workerd',
      encryptedUploadStoredExactly: true,
      retrievalAndDeletionDenied: true,
      baselineUntouched: true,
      bytes: body.length,
    })
  );
} finally {
  await runtime.dispose();
}

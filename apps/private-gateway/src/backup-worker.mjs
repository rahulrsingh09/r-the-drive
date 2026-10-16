const MAX_BYTES = 20 * 1024 * 1024;
const DAILY_SLOTS = 31;
const encoder = new TextEncoder();
const ageHeader = encoder.encode('age-encryption.org/v1\n');

function reply(status, value) {
  return Response.json(value, { status, headers: { 'Cache-Control': 'no-store' } });
}

async function authorized(request, token) {
  if (typeof token !== 'string' || !/^[a-f0-9]{64}$/.test(token)) return false;
  const supplied = request.headers.get('Authorization') ?? '';
  if (supplied.length !== 71) return false;
  const [actual, expected] = await Promise.all(
    [supplied, `Bearer ${token}`].map(
      async (value) => new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(value)))
    )
  );
  let difference = 0;
  for (let i = 0; i < actual.length; i++) difference |= actual[i] ^ expected[i];
  return difference === 0;
}

async function boundedBody(request) {
  const reader = request.body?.getReader();
  if (!reader) return null;
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_BYTES) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const body = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return body;
}

export function createBackupVault(now = () => Date.now()) {
  return {
    async fetch(request, env) {
      const url = new URL(request.url);
      if (
        url.protocol !== 'https:' ||
        url.pathname !== '/backup' ||
        url.search ||
        request.method !== 'POST'
      )
        return reply(404, { error: 'Not found' });
      if (!(await authorized(request, env.HISTORY_BACKUP_TOKEN)))
        return reply(403, { error: 'Forbidden' });
      if (request.headers.get('Content-Type') !== 'application/octet-stream')
        return reply(415, { error: 'Encrypted archive required' });
      const length = Number(request.headers.get('Content-Length'));
      if (length > MAX_BYTES) return reply(413, { error: 'Backup exceeds free storage limit' });
      try {
        const body = await boundedBody(request);
        if (!body || body.length < 100 || body.length > MAX_BYTES)
          return reply(413, { error: 'Backup size is invalid' });
        if (!ageHeader.every((byte, i) => body[i] === byte))
          return reply(400, { error: 'Encrypted archive required' });
        const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', body));
        const sha256 = Array.from(hash, (byte) => byte.toString(16).padStart(2, '0')).join('');
        if (request.headers.get('X-Backup-Sha256') !== sha256)
          return reply(400, { error: 'Checksum mismatch' });
        const timestamp = now();
        const slot = Math.floor(timestamp / 86_400_000) % DAILY_SLOTS;
        const key = `daily/${String(slot).padStart(2, '0')}`;
        // Fixed slots bound storage without deleting an archive before its replacement succeeds.
        await env.HISTORY_BACKUPS.put(key, body, {
          metadata: { createdAt: new Date(timestamp).toISOString(), sha256, bytes: body.length },
        });
        return reply(201, { key, sha256, bytes: body.length });
      } catch {
        return reply(503, { error: 'Backup storage unavailable; keep the local archive' });
      }
    },
  };
}

export default createBackupVault();

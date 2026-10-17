import { BodyLimitError, readBoundedBytes } from './bounded-body.mjs';

const MAX_BYTES = 20 * 1024 * 1024;
const DAILY_SLOTS = 31;
const encoder = new TextEncoder();
const AGE_HEADER = encoder.encode('age-encryption.org/v1\n');

function reply(status, value) {
  const headers = new Headers();
  headers.set('Cache-Control', 'no-store');
  return Response.json(value, { headers, status });
}

async function authorized(request, token) {
  const supplied = request.headers.get('Authorization');
  if (typeof token !== 'string' || !/^[a-f0-9]{64}$/.test(token) || supplied?.length !== 71) return false;
  const digests = await Promise.all([
    crypto.subtle.digest('SHA-256', encoder.encode(supplied)),
    crypto.subtle.digest('SHA-256', encoder.encode(`Bearer ${token}`)),
  ]);
  const actual = new Uint8Array(digests[0]);
  const expected = new Uint8Array(digests[1]);
  // Fixed-length digest comparison must not exit on the first differing byte.
  let difference = 0;
  for (let i = 0; i < 32; i++) difference |= actual[i] ^ expected[i];
  return difference === 0;
}

async function boundedBody(request) {
  try {
    return await readBoundedBytes(request.body, MAX_BYTES);
  } catch (error) {
    if (error instanceof BodyLimitError) return null;
    throw error;
  }
}

async function archiveReceipt(request, env, now) {
  const body = await boundedBody(request);
  if (!body || body.length < 100) return reply(413, { error: 'Backup size is invalid' });
  for (let index = 0; index < AGE_HEADER.length; index++) {
    if (body[index] !== AGE_HEADER[index]) return reply(400, { error: 'Encrypted archive required' });
  }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', body));
  let sha256 = '';
  for (const byte of digest) sha256 += byte.toString(16).padStart(2, '0');
  if (sha256 !== request.headers.get('X-Backup-Sha256')) return reply(400, { error: 'Checksum mismatch' });
  const timestamp = now();
  const slot = Math.floor(timestamp / 86_400_000) % DAILY_SLOTS;
  const key = `daily/${String(slot).padStart(2, '0')}`;
  const receipt = { key, sha256, bytes: body.length };
  await env.HISTORY_BACKUPS.put(key, body, {
    metadata: { createdAt: new Date(timestamp).toISOString(), sha256, bytes: receipt.bytes },
  });
  return reply(201, receipt);
}

export function createBackupVault(now = () => Date.now()) {
  async function fetch(request, env) {
    const { protocol, pathname, search } = new URL(request.url);
    const route = `${request.method} ${pathname}`;
    if (protocol !== 'https:' || route !== 'POST /backup' || search) return reply(404, { error: 'Not found' });
    if (!await authorized(request, env.HISTORY_BACKUP_TOKEN)) return reply(403, { error: 'Forbidden' });
    const constraints = [
      [request.headers.get('Content-Type') !== 'application/octet-stream', 415, 'Encrypted archive required'],
      [Number(request.headers.get('Content-Length')) > MAX_BYTES, 413, 'Backup exceeds free storage limit'],
    ];
    for (const [violated, status, error] of constraints) if (violated) return reply(status, { error });
    try {
      return await archiveReceipt(request, env, now);
    } catch {
      return reply(503, { error: 'Backup storage unavailable; keep the local archive' });
    }
  }
  return { fetch };
}

export default createBackupVault();

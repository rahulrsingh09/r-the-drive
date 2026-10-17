import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { Readable } from 'node:stream';
import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT } from 'jose';
import { createGateway } from '../src/worker.mjs';
import { createBackupVault } from '../src/backup-worker.mjs';
import { basemapConfiguration, describeGatewayMaps, MAP_PREFIX, mapResource, resourceKind } from '../src/maps.mjs';

const env = {
  PUBLIC_ORIGIN: 'https://edge.synthetic', UPSTREAM_ORIGIN: 'https://origin.synthetic',
  ACCESS_ISSUER: 'https://edge.cloudflareaccess.com', ACCESS_AUDIENCE: 'synthetic-audience-1234567890',
  ALLOWED_EMAILS: ' OWNER@example.test, second@example.test ', GATEWAY_TOKEN: 'x'.repeat(48),
};
const { privateKey, publicKey } = await generateKeyPair('RS256');
const keys = createLocalJWKSet({ keys: [{ ...await exportJWK(publicKey), kid: 'edge' }] });
async function assertion() {
  return new SignJWT({ email: 'owner@EXAMPLE.test' }).setProtectedHeader({ alg: 'RS256', kid: 'edge' })
    .setSubject('synthetic').setIssuer(env.ACCESS_ISSUER).setAudience(env.ACCESS_AUDIENCE)
    .setIssuedAt().setExpirationTime('5m').sign(privateKey);
}
async function input(path = '/', init = {}) {
  return new Request(env.PUBLIC_ORIGIN + path, { ...init, headers: {
    'Cf-Access-Jwt-Assertion': await assertion(), ...init.headers,
  } });
}
async function serve(t, handler) {
  const server = createServer(async (req, res) => {
    try {
      const reply = await handler(req);
      res.writeHead(reply.status, Object.fromEntries(reply.headers));
      if (reply.body) Readable.fromWeb(reply.body).pipe(res);
      else res.end();
    } catch { res.writeHead(500); res.end('synthetic harness error'); }
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }));
  return `http://127.0.0.1:${server.address().port}`;
}
function fromHttp(req, origin) {
  return new Request(origin + req.url, { method: req.method, headers: req.headers,
    ...(['GET', 'HEAD'].includes(req.method) ? {} : { body: Readable.toWeb(req), duplex: 'half' }),
  });
}

test('edge characterization: cookies are filtered exactly; browser and response security policies survive', async () => {
  let sent;
  const headers = new Headers({
    'Content-Security-Policy': "default-src 'self'; frame-ancestors 'none'",
    'Access-Control-Allow-Origin': env.PUBLIC_ORIGIN, 'Vary': 'Origin', 'X-Panda-Edge': 'secret',
  });
  headers.append('Set-Cookie', 'refresh_token=one; HttpOnly; Secure; SameSite=Strict');
  headers.append('Set-Cookie', 'theme=dark; Secure');
  const gateway = createGateway({ keySetFor: () => keys, upstreamFetch: async req => {
    sent = req; return new Response('ok', { headers });
  }});
  const response = await gateway.fetch(await input('/v1/auth/refresh', { method: 'POST', headers: {
    Cookie: 'CF_Authorization =gone; refresh_token=kept; CF_Authorization=also-gone; theme=dark',
    Origin: 'https://foreign.synthetic', 'X-Real-IP': '192.0.2.9', Authorization: 'Bearer panda_synthetic',
  }, body: 'body' }), env);
  assert.equal(sent.headers.get('Cookie'), 'refresh_token=kept; theme=dark');
  assert.equal(sent.headers.get('Origin'), 'https://foreign.synthetic');
  assert.equal(sent.headers.get('X-Real-IP'), '192.0.2.9');
  assert.equal(sent.headers.get('Authorization'), 'Bearer panda_synthetic');
  assert.equal(await sent.text(), 'body');
  assert.equal(response.headers.get('Content-Security-Policy'), headers.get('Content-Security-Policy'));
  assert.equal(response.headers.get('Access-Control-Allow-Origin'), env.PUBLIC_ORIGIN);
  assert.deepEqual(response.headers.getSetCookie(), headers.getSetCookie());
  assert.equal(response.headers.get('X-Panda-Edge'), null);
});

test('edge characterization: origin grammar, audience, token and allowlist boundaries fail closed', async () => {
  const gateway = createGateway({ keySetFor: () => keys, upstreamFetch: () => assert.fail('origin reached') });
  for (const patch of [
    { PUBLIC_ORIGIN: 'https://edge.synthetic:444' }, { UPSTREAM_ORIGIN: 'https://origin.synthetic/?q=1' },
    { UPSTREAM_ORIGIN: 'https://origin.synthetic/#fragment' }, { UPSTREAM_ORIGIN: 'http://127.0.0.1' },
    { ACCESS_ISSUER: 'https://cloudflareaccess.com' }, { ACCESS_AUDIENCE: 'a'.repeat(129) },
    { ALLOWED_EMAILS: 'owner@example.test,*@example.test' }, { ALLOWED_EMAILS: '  , ' },
    { GATEWAY_TOKEN: 'a'.repeat(42) },
  ]) assert.equal((await gateway.fetch(await input(), { ...env, ...patch })).status, 503);
});

test('edge characterization: real local HTTP denies before forwarding and preserves request routing', async t => {
  const calls = [];
  const origin = await serve(t, async req => {
    const request = fromHttp(req, env.UPSTREAM_ORIGIN);
    calls.push({ path: req.url, headers: req.headers, body: await request.text() });
    if (req.url === '/redirect') return new Response(null, { status: 302, headers: { Location: 'https://foreign.synthetic/' } });
    if (req.headers.origin === 'https://foreign.synthetic') return new Response('CSRF denied by application', { status: 403 });
    return new Response('local upstream', { headers: { 'Content-Security-Policy': "default-src 'self'" } });
  });
  const gateway = createGateway({ keySetFor: () => keys, upstreamFetch: request => {
    const url = new URL(request.url);
    return fetch(new Request(origin + url.pathname + url.search, request));
  }});
  const edge = await serve(t, req => gateway.fetch(fromHttp(req, env.PUBLIC_ORIGIN), env));
  for (const headers of [{}, { Cookie: 'CF_Authorization=forged' }, { 'Cf-Access-Jwt-Assertion': 'forged' }]) {
    assert.equal((await fetch(edge + '/v1/vehicles', { headers })).status, 403);
  }
  assert.equal(calls.length, 0);
  const headers = { 'Cf-Access-Jwt-Assertion': await assertion(), Authorization: 'Bearer synthetic-app', Origin: env.PUBLIC_ORIGIN };
  const ok = await fetch(edge + '//foreign.synthetic/v1?next=https://foreign.synthetic/', { method: 'POST', headers, body: 'payload' });
  assert.equal(ok.status, 200);
  assert.equal(calls[0].path, '//foreign.synthetic/v1?next=https://foreign.synthetic/');
  assert.equal(calls[0].body, 'payload');
  assert.equal(calls[0].headers['cf-access-jwt-assertion'], undefined);
  assert.equal(calls[0].headers['x-panda-edge'], env.GATEWAY_TOKEN);
  assert.equal(calls[0].headers.authorization, 'Bearer synthetic-app');
  assert.equal(ok.headers.get('Content-Security-Policy'), "default-src 'self'");
  assert.equal((await fetch(edge + '/redirect', { headers })).status, 502);
  assert.equal((await fetch(edge + '/v1/auth/refresh', { method: 'POST', headers: { ...headers, Origin: 'https://foreign.synthetic' } })).status, 403);
});

test('edge characterization: map grammar rejects private resources and bounding edge cases', () => {
  for (const path of ['planet/20261004_113936_pt/14/16383/16383.pbf', 'natural_earth/ne2sr/6/63/63.png', 'fonts/Noto/65280-65535.pbf']) assert(resourceKind(path), path);
  for (const path of ['planet/20261004_113936_pt/14/16384/0.pbf', 'planet/20261004_113936_pt/00/0/0.pbf', 'natural_earth/ne2sr/7/0/0.png', 'fonts/Noto/65281-65536.pbf', 'styles/dark%00', 'styles/dark#x', 'x'.repeat(513)]) assert.equal(resourceKind(path), null, path);
  const config = basemapConfiguration();
  assert.deepEqual(config.styles.map(s => s.id), ['follow-theme', 'positron', 'bright', 'liberty', 'dark', 'fiord', '3d']);
  assert.equal(config.styles[6].light_url, config.styles[3].light_url);
  const other = { id: 'other', enabled: false, nested: { unchanged: true } };
  const inventory = describeGatewayMaps({ connections: [other, { id: 'basemap', custom: true }], can_manage: false });
  assert.equal(inventory.connections[0], other);
  assert.equal(inventory.connections[1].custom, true);
  assert.equal(inventory.connections[1].allow_private_network, false);
  assert.deepEqual(inventory.connections[1].private_network_allowlist, []);
  assert.throws(() => describeGatewayMaps({ connections: {} }));
});

test('edge characterization: map JSON validation closes every auxiliary destination', async () => {
  const base = { version: 8, layers: [], sources: { p: { type: 'vector', url: 'https://tiles.openfreemap.org/planet' } } };
  for (const body of [null, [], { ...base, layers: Array(2049).fill({}) }, { ...base, sources: { p: { type: 'geojson', data: 'http://127.0.0.1' } } },
    { ...base, sources: { p: { type: 'vector' } } }, { ...base, glyphs: 'http://169.254.169.254/' },
    { ...base, sprite: 'https://tiles.openfreemap.org.evil.synthetic/sprites/ofm_a/ofm' }]) {
    assert.equal((await mapResource(await input(MAP_PREFIX + 'styles/dark'), async () => Response.json(body))).status, 502);
  }
  for (const body of [{ tiles: [] }, { tiles: ['https://tiles.openfreemap.org/planet'], grids: [] }, { tiles: Array(5).fill('https://tiles.openfreemap.org/planet') }]) {
    assert.equal((await mapResource(await input(MAP_PREFIX + 'planet'), async () => Response.json(body))).status, 502);
  }
});

test('edge characterization: map rejected response streams are cancelled without credential disclosure', async () => {
  for (const mode of ['status', 'media', 'declared', 'overflow']) {
    let cancelled = 0;
    const stream = new ReadableStream({ pull(c) { c.enqueue(new Uint8Array(600 * 1024)); }, cancel() { cancelled++; } });
    const response = await mapResource(await input(MAP_PREFIX + 'styles/dark'), async () => new Response(stream, {
      status: mode === 'status' ? 302 : 200,
      headers: { 'Content-Type': mode === 'media' ? 'text/html' : 'application/json', ...(mode === 'declared' ? { 'Content-Length': '99999999' } : {}) },
    }));
    assert.equal(response.status, 502);
    assert.equal(cancelled, 1, mode);
    assert.equal(response.headers.get('Access-Control-Allow-Origin'), null);
  }
});

test('edge characterization: write-only vault over real local HTTP denies reads and invalid uploads', async t => {
  const records = new Map();
  const token = 'b'.repeat(64);
  const vault = createBackupVault(() => Date.UTC(2026, 9, 17));
  const vaultEnv = { HISTORY_BACKUP_TOKEN: token, HISTORY_BACKUPS: { async put(key, body, options) { records.set(key, { body, options }); } } };
  const local = await serve(t, req => vault.fetch(fromHttp(req, 'https://vault.synthetic'), vaultEnv));
  const body = new TextEncoder().encode('age-encryption.org/v1\n' + 'synthetic'.repeat(20));
  const headers = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/octet-stream', 'X-Backup-Sha256': Buffer.from(await crypto.subtle.digest('SHA-256', body)).toString('hex') };
  for (const path of ['/backup?key=x', '/backup/other']) assert.equal((await fetch(local + path, { method: 'POST', headers, body })).status, 404);
  for (const method of ['GET', 'DELETE', 'OPTIONS']) assert.equal((await fetch(local + '/backup', { method, headers })).status, 404);
  for (const auth of ['', `bearer ${token}`, `Bearer ${'c'.repeat(64)}`]) assert.equal((await fetch(local + '/backup', { method: 'POST', headers: { ...headers, Authorization: auth }, body })).status, 403);
  assert.equal(records.size, 0);
  const response = await fetch(local + '/backup', { method: 'POST', headers, body });
  assert.equal(response.status, 201);
  const receipt = await response.json();
  assert.equal(records.size, 1);
  assert.deepEqual(records.get(receipt.key).body, body);
  assert.equal((await fetch(local + '/backup', { method: 'POST', headers: { ...headers, 'X-Backup-Sha256': 'wrong' }, body })).status, 400);
  assert.equal(records.size, 1);
});

test('edge characterization: vault cancels overflowing streams and contains storage errors', async () => {
  let cancelled = 0;
  let writes = 0;
  const stream = new ReadableStream({ pull(c) { c.enqueue(new Uint8Array(21 * 1024 * 1024)); }, cancel() { cancelled++; } });
  const response = await createBackupVault().fetch(new Request('https://vault.synthetic/backup', { method: 'POST', duplex: 'half',
    headers: { Authorization: `Bearer ${'a'.repeat(64)}`, 'Content-Type': 'application/octet-stream' }, body: stream,
  }), { HISTORY_BACKUP_TOKEN: 'a'.repeat(64), HISTORY_BACKUPS: { put() { writes++; } } });
  assert.equal(response.status, 413);
  assert.equal(cancelled, 1);
  assert.equal(writes, 0);
});

test('edge replacement: completed and failed map bodies release their readers', async () => {
  const bodies = [
    Response.json({ version: 8, layers: [], sources: { p: { type: 'vector', url: 'https://tiles.openfreemap.org/planet' } } }),
    new Response(new ReadableStream({ pull() { throw new Error('synthetic stream failure'); } }), { headers: { 'Content-Type': 'application/json' } }),
  ];
  for (const [index, upstream] of bodies.entries()) {
    const response = await mapResource(await input(MAP_PREFIX + 'styles/dark'), async () => upstream);
    assert.equal(response.status, index === 0 ? 200 : 502);
    assert.equal(upstream.body.locked, false);
  }
});

test('edge characterization: default Access key resolvers cache by issuer using synthetic transport', async t => {
  const calls = [];
  t.mock.method(globalThis, 'fetch', async (url, init) => {
    calls.push({ url, init });
    assert.equal(url, env.ACCESS_ISSUER + '/cdn-cgi/access/certs');
    assert.equal(init.redirect, 'manual');
    return Response.json({ keys: [{ ...await exportJWK(publicKey), kid: 'edge' }] });
  });
  for (let attempt = 0; attempt < 2; attempt++) {
    const gateway = createGateway({ upstreamFetch: async () => new Response('synthetic') });
    assert.equal((await gateway.fetch(await input(), env)).status, 200);
  }
  assert.equal(calls.length, 1);
});

test('edge characterization: provider timeout aborts the request and returns a generic failure', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let observed;
  const pending = mapResource(new Request(env.PUBLIC_ORIGIN + MAP_PREFIX + 'planet'), request => {
    observed = request.signal;
    return new Promise((resolve, reject) => request.signal.addEventListener('abort', () => reject(new Error('synthetic timeout secret')), { once: true }));
  });
  assert.equal(observed.aborted, false);
  t.mock.timers.tick(10_000);
  const response = await pending;
  assert.equal(observed.aborted, true);
  assert.equal(response.status, 502);
  assert.doesNotMatch(await response.text(), /secret/);
});

test('edge characterization: map binary formats enforce PNG signature and protobuf media contracts', async () => {
  const png = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10, 0]);
  for (const bytes of [png, new Uint8Array([1, 2, 3])]) {
    const response = await mapResource(await input(MAP_PREFIX + 'natural_earth/ne2sr/0/0/0.png'), async () => new Response(bytes, { headers: { 'Content-Type': 'image/png' } }));
    assert.equal(response.status, bytes === png ? 200 : 502);
    if (response.status === 200) assert.deepEqual(new Uint8Array(await response.arrayBuffer()), png);
  }
  for (const media of ['application/x-protobuf', 'application/vnd.mapbox-vector-tile', 'application/octet-stream']) {
    const response = await mapResource(await input(MAP_PREFIX + 'planet/20261004_113936_pt/0/0/0.pbf'), async () => new Response(new Uint8Array([1]), { headers: { 'Content-Type': media } }));
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('Content-Type'), 'application/x-protobuf');
  }
  assert.equal((await mapResource(await input(MAP_PREFIX + 'planet'), async () => new Response(null, { headers: { 'Content-Type': 'application/json' } }))).status, 502);
});

test('edge characterization: backup stream errors remain storage failures and absent bodies remain invalid size', async () => {
  const vaultEnv = { HISTORY_BACKUP_TOKEN: 'a'.repeat(64), HISTORY_BACKUPS: { put: () => assert.fail('unexpected persistence') } };
  for (const body of [null, new ReadableStream({ pull() { throw new RangeError('synthetic read failure'); } })]) {
    const req = new Request('https://vault.synthetic/backup', { method: 'POST', duplex: 'half', body,
      headers: { Authorization: `Bearer ${vaultEnv.HISTORY_BACKUP_TOKEN}`, 'Content-Type': 'application/octet-stream' } });
    const response = await createBackupVault().fetch(req, vaultEnv);
    assert.equal(response.status, body ? 503 : 413);
    assert.doesNotMatch(await response.text(), /synthetic read failure/);
    if (req.body) assert.equal(req.body.locked, false);
  }
});

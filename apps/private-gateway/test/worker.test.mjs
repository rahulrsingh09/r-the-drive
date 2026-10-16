import assert from 'node:assert/strict';
import test from 'node:test';
import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT } from 'jose';
import { createGateway } from '../src/worker.mjs';

const { privateKey, publicKey } = await generateKeyPair('RS256');
const jwk = await exportJWK(publicKey);
jwk.kid = 'synthetic-key';
const env = {
  PUBLIC_ORIGIN: 'https://private.synthetic.workers.dev',
  UPSTREAM_ORIGIN: 'https://synthetic.code.run',
  ACCESS_ISSUER: 'https://synthetic.cloudflareaccess.com',
  ACCESS_AUDIENCE: 'synthetic-audience-1234567890',
  ALLOWED_EMAILS: 'owner@example.test',
  GATEWAY_TOKEN: 'synthetic-gateway-token-abcdefghijklmnopqrstuvwxyz',
};
const keys = createLocalJWKSet({ keys: [jwk] });

async function token(overrides = {}, key = privateKey) {
  const now = Math.floor(Date.now() / 1000);
  return new SignJWT({
    email: 'owner@example.test',
    sub: 'synthetic-owner',
    iss: env.ACCESS_ISSUER,
    aud: env.ACCESS_AUDIENCE,
    iat: now,
    exp: now + 300,
    ...overrides,
  })
    .setProtectedHeader({ alg: 'RS256', kid: jwk.kid })
    .sign(key);
}

function harness(response = () => new Response('dashboard')) {
  const requests = [];
  const gateway = createGateway({
    keySetFor: () => keys,
    upstreamFetch: async (request, options) => {
      requests.push({ request, options });
      return response(request);
    },
  });
  return { gateway, requests };
}

async function request(assertion, path = '/', init = {}) {
  const headers = new Headers(init.headers);
  if (assertion) headers.set('Cf-Access-Jwt-Assertion', assertion);
  return new Request(env.PUBLIC_ORIGIN + path, { ...init, headers });
}

test('missing setup fails closed without reaching the origin', async () => {
  for (const key of Object.keys(env)) {
    const { gateway, requests } = harness();
    const config = { ...env };
    delete config[key];
    assert.equal((await gateway.fetch(await request(await token()), config)).status, 503, key);
    assert.equal(requests.length, 0);
  }
});

test('unsafe configuration fails closed', async () => {
  for (const config of [
    { PUBLIC_ORIGIN: 'http://private.synthetic.workers.dev' },
    { UPSTREAM_ORIGIN: 'https://user:password@synthetic.code.run' },
    { UPSTREAM_ORIGIN: 'https://synthetic.code.run/private' },
    { UPSTREAM_ORIGIN: env.PUBLIC_ORIGIN },
    { ACCESS_ISSUER: 'https://synthetic.cloudflareaccess.com.evil.test' },
    { ALLOWED_EMAILS: '*@example.test' },
    { GATEWAY_TOKEN: 'short' },
  ]) {
    const { gateway, requests } = harness();
    assert.equal(
      (await gateway.fetch(await request(await token()), { ...env, ...config })).status,
      503
    );
    assert.equal(requests.length, 0);
  }
});

test('missing, forged, expired and wrong-application credentials never reach the origin', async () => {
  const other = await generateKeyPair('RS256');
  const assertions = [
    undefined,
    'not-a-jwt',
    'a'.repeat(16_385),
    await token({ exp: Math.floor(Date.now() / 1000) - 60 }),
    await token({ iat: Math.floor(Date.now() / 1000) + 60 }),
    await token({ aud: 'a-different-access-application' }),
    await token({ iss: 'https://other.cloudflareaccess.com' }),
    await token({ email: 'another@example.test' }),
    await token({ email: undefined }),
    await token({ sub: undefined }),
    await token({ exp: undefined }),
    await token({}, other.privateKey),
  ];
  for (const assertion of assertions) {
    const { gateway, requests } = harness();
    assert.equal((await gateway.fetch(await request(assertion), env)).status, 403);
    assert.equal(requests.length, 0);
  }
});

test('email and edge headers cannot substitute for a verified identity', async () => {
  const { gateway, requests } = harness();
  const input = await request(undefined, '/', {
    headers: {
      'Cf-Access-Authenticated-User-Email': env.ALLOWED_EMAILS,
      'X-Panda-Edge': env.GATEWAY_TOKEN,
      Cookie: 'CF_Authorization=forged',
    },
  });
  assert.equal((await gateway.fetch(input, env)).status, 403);
  assert.equal(requests.length, 0);
});

test('seven-day sessions remain valid beyond eight hours and one day', async () => {
  const now = Math.floor(Date.now() / 1000);
  const week = 7 * 24 * 60 * 60;
  for (const age of [0, 8 * 60 * 60 + 60, 24 * 60 * 60 + 60, week - 60]) {
    const { gateway, requests } = harness();
    const assertion = await token({ iat: now - age, exp: now - age + week });
    assert.equal((await gateway.fetch(await request(assertion), env)).status, 200);
    assert.equal(requests.length, 1);
  }
});

test('expired, missing-age and overlong sessions never reach the origin', async () => {
  const now = Math.floor(Date.now() / 1000);
  const week = 7 * 24 * 60 * 60;
  for (const claims of [
    { iat: now - week - 60, exp: now - 60 },
    { iat: now, exp: now + week + 1 },
    { iat: now - week - 60, exp: now + 60 },
    { iat: now, exp: now },
    { iat: undefined, exp: now + week },
  ]) {
    const { gateway, requests } = harness();
    assert.equal(
      (await gateway.fetch(await request(await token(claims)), env)).status,
      403
    );
    assert.equal(requests.length, 0);
  }
});

test('preview and alternative hostnames are rejected even with a valid identity', async () => {
  const { gateway, requests } = harness();
  const input = new Request('https://preview.private.synthetic.workers.dev/', {
    headers: { 'Cf-Access-Jwt-Assertion': await token() },
  });
  assert.equal((await gateway.fetch(input, env)).status, 403);
  assert.equal(requests.length, 0);
});

test('authenticated requests target only the configured origin and replace client edge headers', async () => {
  const { gateway, requests } = harness();
  const input = await request(await token(), '//evil.test/api/trips?next=https://evil.test/', {
    headers: {
      'X-Panda-Edge': 'attacker',
      'X-Forwarded-Host': 'evil.test',
      'X-Forwarded-For': 'spoofed',
      Forwarded: 'host=evil.test',
      Authorization: 'Bearer synthetic-app-token',
      'Cf-Access-Client-Secret': 'synthetic-client-secret',
      'Cf-Access-Token': 'synthetic-cli-token',
      Cookie: 'CF_Authorization=synthetic; refresh_token=synthetic-refresh; theme=dark',
    },
  });
  const result = await gateway.fetch(input, env);
  assert.equal(result.status, 200);
  const forwarded = requests[0].request;
  assert.equal(new URL(forwarded.url).origin, env.UPSTREAM_ORIGIN);
  assert.equal(new URL(forwarded.url).pathname, '//evil.test/api/trips');
  assert.equal(forwarded.headers.get('X-Panda-Edge'), env.GATEWAY_TOKEN);
  assert.equal(forwarded.headers.get('Authorization'), 'Bearer synthetic-app-token');
  assert.equal(forwarded.headers.get('X-Forwarded-Host'), 'private.synthetic.workers.dev');
  assert.equal(forwarded.headers.get('X-Forwarded-For'), null);
  assert.equal(forwarded.headers.get('Forwarded'), null);
  assert.equal(forwarded.headers.get('Cf-Access-Jwt-Assertion'), null);
  assert.equal(forwarded.headers.get('Cf-Access-Client-Secret'), null);
  assert.equal(forwarded.headers.get('Cf-Access-Token'), null);
  assert.equal(forwarded.headers.get('Cookie'), 'refresh_token=synthetic-refresh; theme=dark');
  assert.equal(forwarded.redirect, 'manual');
  assert.equal(result.headers.get('Cache-Control'), 'private, no-store');
  assert.equal(requests[0].options.cf.cacheTtl, 0);
});

test('app request bodies and browser Origin are preserved', async () => {
  const { gateway, requests } = harness();
  const input = await request(await token(), '/api/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Origin: env.PUBLIC_ORIGIN },
    body: JSON.stringify({ email: 'synthetic@example.test', password: 'synthetic-only' }),
  });
  assert.equal((await gateway.fetch(input, env)).status, 200);
  assert.equal(requests[0].request.headers.get('Origin'), env.PUBLIC_ORIGIN);
  assert.deepEqual(await requests[0].request.json(), {
    email: 'synthetic@example.test',
    password: 'synthetic-only',
  });
});

test('WebSocket upgrade preserves app authentication and includes the trusted edge credential', async () => {
  const upgraded = { status: 101, webSocket: { synthetic: true } };
  const { gateway, requests } = harness(() => upgraded);
  const input = await request(await token(), '/api/ws', {
    headers: {
      Upgrade: 'websocket',
      Connection: 'Upgrade',
      'Sec-WebSocket-Protocol': 'bearer, bearer.synthetic',
    },
  });
  assert.equal(await gateway.fetch(input, env), upgraded);
  assert.equal(requests[0].request.headers.get('Upgrade'), 'websocket');
  assert.equal(
    requests[0].request.headers.get('Sec-WebSocket-Protocol'),
    'bearer, bearer.synthetic'
  );
  assert.equal(requests[0].request.headers.get('X-Panda-Edge'), env.GATEWAY_TOKEN);
});

test('relative and origin redirects are rewritten without following them', async () => {
  for (const location of ['/login', env.UPSTREAM_ORIGIN + '/login?next=trips']) {
    const { gateway, requests } = harness(() =>
      Response.redirect(new URL(location, env.UPSTREAM_ORIGIN))
    );
    const response = await gateway.fetch(await request(await token()), env);
    assert.equal(response.status, 302);
    assert.equal(new URL(response.headers.get('Location')).origin, env.PUBLIC_ORIGIN);
    assert.equal(requests.length, 1);
  }
});

test('external redirects are blocked instead of forwarding credentials to another host', async () => {
  const { gateway, requests } = harness(() => Response.redirect('https://evil.test'));
  assert.equal((await gateway.fetch(await request(await token()), env)).status, 502);
  assert.equal(requests.length, 1);
});

test('malformed redirects produce a generic failure', async () => {
  const { gateway } = harness(
    () => new Response(null, { status: 302, headers: { Location: 'https://[' } })
  );
  assert.equal((await gateway.fetch(await request(await token()), env)).status, 502);
});

test('origin failures do not disclose exception details', async () => {
  const { gateway } = harness(() => {
    throw new Error('synthetic-secret');
  });
  const response = await gateway.fetch(await request(await token()), env);
  assert.equal(response.status, 502);
  assert.doesNotMatch(await response.text(), /synthetic-secret/);
});

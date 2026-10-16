import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';
import { generateKeyPair, exportJWK, SignJWT } from 'jose';
const require = createRequire(import.meta.url);
const { Miniflare, Response, WebSocketPair } = require(
  process.env.PANDA_MINIFLARE_MODULE ?? 'miniflare'
);
const { privateKey, publicKey } = await generateKeyPair('RS256');
const key = { ...(await exportJWK(publicKey)), kid: 'runtime-test' };
const bindings = {
  PUBLIC_ORIGIN: 'https://private.synthetic.workers.dev',
  UPSTREAM_ORIGIN: 'https://synthetic.code.run',
  ACCESS_ISSUER: 'https://synthetic.cloudflareaccess.com',
  ACCESS_AUDIENCE: 'synthetic-audience-1234567890',
  ALLOWED_EMAILS: 'owner@example.test',
  GATEWAY_TOKEN: 'synthetic-gateway-token-abcdefghijklmnopqrstuvwxyz',
  ENABLE_FREE_MAPS: 'true',
};
let originRequests = 0;
let mapRequests = 0;
const runtime = new Miniflare({
  telemetry: { enabled: false },
  logRequests: false,
  workers: [
    {
      config: {
        name: 'private-gateway',
        compatibilityDate: '2026-10-06',
        manifest: {
          mainModule: 'worker.mjs',
          modules: {
            'worker.mjs': {
              type: 'esm',
              contents: await readFile(new URL('./dist/worker.mjs', import.meta.url), 'utf8'),
            },
          },
        },
        env: Object.fromEntries(
          Object.entries(bindings).map(([name, value]) => [name, { type: 'text', value }])
        ),
      },
      dev: {
        outboundService: {
          type: 'fetcher',
          handler: async (request) => {
            const url = new URL(request.url);
            if (url.origin === bindings.ACCESS_ISSUER && url.pathname === '/cdn-cgi/access/certs') {
              return Response.json({ keys: [key] });
            }
            if (url.origin === 'https://tiles.openfreemap.org') {
              mapRequests++;
              assert.equal(request.headers.get('Authorization'), null);
              assert.equal(request.headers.get('Cookie'), null);
              assert.equal(request.headers.get('X-Panda-Edge'), null);
              assert.equal(request.headers.get('Cf-Access-Jwt-Assertion'), null);
              return Response.json({
                version: 8,
                sources: {
                  planet: { type: 'vector', url: 'https://tiles.openfreemap.org/planet' },
                },
                layers: [],
              });
            }
            assert.equal(url.origin, bindings.UPSTREAM_ORIGIN);
            assert.equal(request.headers.get('X-Panda-Edge'), bindings.GATEWAY_TOKEN);
            assert.equal(request.headers.get('Cf-Access-Jwt-Assertion'), null);
            originRequests++;
            if (url.pathname === '/v1/external/basemap/config') {
              return request.headers.get('Authorization') === 'Bearer synthetic-app'
                ? Response.json({ enabled: false })
                : new Response('Unauthorized', { status: 401 });
            }
            if (request.headers.get('Upgrade') === 'websocket') {
              assert.deepEqual(
                request.headers
                  .get('Sec-WebSocket-Protocol')
                  .split(',')
                  .map((value) => value.trim()),
                ['bearer', 'bearer.synthetic']
              );
              const pair = new WebSocketPair();
              pair[1].accept();
              pair[1].addEventListener('message', (event) => pair[1].send('echo:' + event.data));
              return new Response(null, {
                status: 101,
                webSocket: pair[0],
                headers: { 'Sec-WebSocket-Protocol': 'bearer' },
              });
            }
            return Response.json({
              body: request.method === 'POST' ? await request.json() : null,
              path: url.pathname,
              cookie: request.headers.get('Cookie'),
            });
          },
        },
      },
    },
  ],
});
try {
  const token = await new SignJWT({ email: bindings.ALLOWED_EMAILS })
    .setProtectedHeader({ alg: 'RS256', kid: key.kid })
    .setSubject('runtime-owner')
    .setIssuer(bindings.ACCESS_ISSUER)
    .setAudience(bindings.ACCESS_AUDIENCE)
    .setIssuedAt()
    .setExpirationTime('5m')
    .sign(privateKey);
  const denied = await runtime.dispatchFetch(bindings.PUBLIC_ORIGIN + '/');
  assert.equal(denied.status, 403);
  assert.equal(originRequests, 0);
  const now = Math.floor(Date.now() / 1000);
  const week = 7 * 24 * 60 * 60;
  async function sessionToken(age, duration, email = bindings.ALLOWED_EMAILS) {
    return new SignJWT({ email })
      .setProtectedHeader({ alg: 'RS256', kid: key.kid })
      .setSubject('runtime-owner')
      .setIssuer(bindings.ACCESS_ISSUER)
      .setAudience(bindings.ACCESS_AUDIENCE)
      .setIssuedAt(now - age)
      .setExpirationTime(now - age + duration)
      .sign(privateKey);
  }
  const continued = await runtime.dispatchFetch(bindings.PUBLIC_ORIGIN + '/', {
    headers: { 'Cf-Access-Jwt-Assertion': await sessionToken(6 * 24 * 60 * 60, week) },
  });
  assert.equal(continued.status, 200);
  const beforeInvalidSessions = originRequests;
  for (const assertion of [
    await sessionToken(week + 60, week),
    await sessionToken(0, week + 1),
    await sessionToken(6 * 24 * 60 * 60, week, 'another@example.test'),
  ]) {
    const rejected = await runtime.dispatchFetch(bindings.PUBLIC_ORIGIN + '/', {
      headers: { 'Cf-Access-Jwt-Assertion': assertion },
    });
    assert.equal(rejected.status, 403);
  }
  assert.equal(originRequests, beforeInvalidSessions);
  const configPath = bindings.PUBLIC_ORIGIN + '/v1/external/basemap/config';
  const configDenied = await runtime.dispatchFetch(configPath, {
    headers: { 'Cf-Access-Jwt-Assertion': token },
  });
  assert.equal(configDenied.status, 401);
  const mapConfig = await runtime.dispatchFetch(configPath, {
    headers: { 'Cf-Access-Jwt-Assertion': token, Authorization: 'Bearer synthetic-app' },
  });
  assert.equal((await mapConfig.json()).resolved_provider, 'openfreemap');
  const originBeforeMap = originRequests;
  const map = await runtime.dispatchFetch(
    bindings.PUBLIC_ORIGIN + '/v1/external/basemap/openfreemap/styles/dark',
    {
      headers: {
        'Cf-Access-Jwt-Assertion': token,
        Authorization: 'Bearer synthetic-app',
        Cookie: 'refresh_token=synthetic',
      },
    }
  );
  assert.equal(map.status, 200);
  assert.equal(
    (await map.json()).sources.planet.url,
    'https://panda.invalid/v1/external/basemap/openfreemap/planet'
  );
  assert.equal(mapRequests, 1);
  assert.equal(originRequests, originBeforeMap);
  const result = await runtime.dispatchFetch(bindings.PUBLIC_ORIGIN + '/api/auth/login', {
    method: 'POST',
    headers: {
      'Cf-Access-Jwt-Assertion': token,
      'Content-Type': 'application/json',
      Cookie: 'CF_Authorization=synthetic; refresh_token=synthetic',
    },
    body: JSON.stringify({ email: 'synthetic@example.test', password: 'synthetic-only' }),
  });
  assert.equal(result.status, 200);
  const payload = await result.json();
  assert.equal(payload.body.password, 'synthetic-only');
  assert.equal(payload.cookie, 'refresh_token=synthetic');
  const upgrade = await runtime.dispatchFetch(bindings.PUBLIC_ORIGIN + '/api/ws', {
    headers: {
      'Cf-Access-Jwt-Assertion': token,
      Upgrade: 'websocket',
      Connection: 'Upgrade',
      'Sec-WebSocket-Protocol': 'bearer, bearer.synthetic',
    },
  });
  if (upgrade.status !== 101) console.error('Runtime upgrade diagnostic:', await upgrade.text());
  assert.equal(upgrade.status, 101);
  const socket = upgrade.webSocket;
  assert(socket);
  socket.accept();
  const reply = new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('WebSocket timeout')), 5000);
    socket.addEventListener(
      'message',
      (event) => {
        clearTimeout(timeout);
        resolve(event.data);
      },
      { once: true }
    );
  });
  socket.send('probe');
  assert.equal(await reply, 'echo:probe');
  socket.close();
  const evidence = {
    testedAt: new Date().toISOString(),
    runtime: 'workerd via Miniflare',
    unauthorizedDeniedBeforeOrigin: true,
    realJwtValidationWithJwksFetch: true,
    sevenDaySessionsAcceptSixDayOldToken: true,
    expiredOverlongAndWrongOwnerSessionsDenied: true,
    httpRequestBodyAndCookiePreserved: true,
    actualWebSocketUpgradeAndEcho: true,
    mapConfigurationRequiresAppAuthentication: true,
    mapAssetsExcludeCredentialsAndNorthflankTraffic: true,
    originRequests,
    syntheticOnly: true,
  };
  console.log(JSON.stringify(evidence));
} finally {
  await runtime.dispose();
}

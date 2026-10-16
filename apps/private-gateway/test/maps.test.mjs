import assert from 'node:assert/strict';
import test from 'node:test';
import { createLocalJWKSet, exportJWK, generateKeyPair, SignJWT } from 'jose';
import { createGateway } from '../src/worker.mjs';
import { MAP_CONFIG, MAP_PREFIX, CONNECTIONS, resourceKind, mapResource } from '../src/maps.mjs';

const env = {
  PUBLIC_ORIGIN: 'https://private.synthetic.workers.dev',
  UPSTREAM_ORIGIN: 'https://synthetic.code.run',
  ACCESS_ISSUER: 'https://synthetic.cloudflareaccess.com',
  ACCESS_AUDIENCE: 'synthetic-audience-1234567890',
  ALLOWED_EMAILS: 'owner@example.test',
  GATEWAY_TOKEN: 'synthetic-gateway-token-abcdefghijklmnopqrstuvwxyz',
  ENABLE_FREE_MAPS: 'true',
};
const { privateKey, publicKey } = await generateKeyPair('RS256');
const jwk = { ...(await exportJWK(publicKey)), kid: 'maps-test' };
const keys = createLocalJWKSet({ keys: [jwk] });
async function request(path, { authenticated = true, method = 'GET', ...init } = {}) {
  const headers = new Headers(init.headers);
  if (authenticated)
    headers.set(
      'Cf-Access-Jwt-Assertion',
      await new SignJWT({ email: env.ALLOWED_EMAILS })
        .setProtectedHeader({ alg: 'RS256', kid: jwk.kid })
        .setSubject('synthetic-owner')
        .setIssuer(env.ACCESS_ISSUER)
        .setAudience(env.ACCESS_AUDIENCE)
        .setIssuedAt()
        .setExpirationTime('5m')
        .sign(privateKey)
    );
  return new Request(env.PUBLIC_ORIGIN + path, { ...init, method, headers });
}
const style = {
  version: 8,
  sources: { planet: { type: 'vector', url: 'https://tiles.openfreemap.org/planet' } },
  sprite: 'https://tiles.openfreemap.org/sprites/ofm_f384/ofm',
  glyphs: 'https://tiles.openfreemap.org/fonts/{fontstack}/{range}.pbf',
  layers: [],
};

test('free maps are opt-in, require Access, and bypass the Northflank origin only for public assets', async () => {
  let origin = 0;
  let provider = 0;
  const gateway = createGateway({
    keySetFor: () => keys,
    upstreamFetch: async () => {
      origin++;
      return new Response('disabled', { status: 403 });
    },
    mapFetch: async () => {
      provider++;
      return Response.json(style);
    },
  });
  assert.equal(
    (await gateway.fetch(await request(MAP_PREFIX + 'styles/dark', { authenticated: false }), env))
      .status,
    403
  );
  assert.equal(provider, 0);
  assert.equal(
    (
      await gateway.fetch(await request(MAP_PREFIX + 'styles/dark'), {
        ...env,
        ENABLE_FREE_MAPS: 'false',
      })
    ).status,
    403
  );
  assert.equal(provider, 0);
  assert.equal(origin, 1);
  assert.equal((await gateway.fetch(await request(MAP_PREFIX + 'styles/dark'), env)).status, 200);
  assert.equal(origin, 1);
  assert.equal(provider, 1);
});

test('map provider receives no user, Rivian, application, or gateway credentials', async () => {
  const sent = [];
  const gateway = createGateway({
    keySetFor: () => keys,
    upstreamFetch: () => assert.fail('tile downloads must not use Northflank'),
    mapFetch: async (req) => {
      sent.push(req);
      return Response.json(style, {
        headers: { 'Set-Cookie': 'tracking=bad', 'Access-Control-Allow-Origin': '*' },
      });
    },
  });
  const response = await gateway.fetch(
    await request(MAP_PREFIX + 'styles/dark?v=test', {
      headers: {
        Authorization: 'Bearer synthetic-secret',
        Cookie: 'refresh=synthetic-secret',
        'Cf-Access-Token': 'synthetic-secret',
        'X-Panda-Edge': 'synthetic-secret',
        Referer: 'https://private.synthetic.workers.dev/trips/secret-trip',
        'X-Forwarded-For': '192.0.2.1',
      },
    }),
    env
  );
  assert.equal(response.status, 200);
  assert.equal(sent[0].url, 'https://tiles.openfreemap.org/styles/dark');
  assert.deepEqual([...sent[0].headers], [['accept', '*/*']]);
  assert.equal(sent[0].redirect, 'manual');
  assert.equal(sent[0].credentials, 'omit');
  assert.equal(response.headers.get('Set-Cookie'), null);
  assert.equal(response.headers.get('Access-Control-Allow-Origin'), null);
  assert.match(response.headers.get('Cache-Control'), /private/);
});

test('configuration keeps app authentication and changes only a successful expected response', async () => {
  for (const status of [401, 403, 429, 500]) {
    const gateway = createGateway({
      keySetFor: () => keys,
      upstreamFetch: async () => new Response('denied', { status }),
    });
    assert.equal((await gateway.fetch(await request(MAP_CONFIG), env)).status, status);
  }
  const gateway = createGateway({
    keySetFor: () => keys,
    upstreamFetch: async () =>
      Response.json(
        { enabled: false },
        {
          headers: { 'Content-Length': '17', ETag: 'old', 'Content-Encoding': 'gzip' },
        }
      ),
  });
  const response = await gateway.fetch(await request(MAP_CONFIG), env);
  const config = await response.json();
  assert.equal(config.enabled, true);
  assert.equal(config.resolved_provider, 'openfreemap');
  assert.equal(config.styles.length, 7);
  assert(
    config.styles.every(
      (item) => item.light_url.startsWith(MAP_PREFIX) && item.dark_url.startsWith(MAP_PREFIX)
    )
  );
  assert.equal(response.headers.get('Content-Length'), null);
  assert.equal(response.headers.get('Content-Encoding'), null);
  assert.equal(response.headers.get('ETag'), null);
  assert.equal(response.headers.get('Cache-Control'), 'private, no-store');
  const invalid = createGateway({
    keySetFor: () => keys,
    upstreamFetch: async () => Response.json({ unexpected: true }),
  });
  assert.equal((await invalid.fetch(await request(MAP_CONFIG), env)).status, 502);
});

test('inventory identifies gateway maps without unlocking other optional providers', async () => {
  const other = { id: 'nominatim', enabled: false, editable: false };
  const gateway = createGateway({
    keySetFor: () => keys,
    upstreamFetch: async () =>
      Response.json({
        can_manage: true,
        connections: [{ id: 'basemap', enabled: false, editable: false }, other],
      }),
  });
  const inventory = await (await gateway.fetch(await request(CONNECTIONS), env)).json();
  assert.equal(inventory.connections[0].enabled, true);
  assert.equal(inventory.connections[0].editable, false);
  assert.equal(inventory.connections[0].execution, 'Cloudflare gateway');
  assert.deepEqual(inventory.connections[1], other);
  const disabled = await gateway.fetch(
    await request(CONNECTIONS + '/disable-optional', { method: 'POST' }),
    env
  );
  assert.equal(
    disabled.status,
    403,
    'A settings action must not falsely claim to disable deployment-managed maps'
  );
});

test('valid resource paths have bounded tile coordinates, glyph ranges, and file types', () => {
  for (const path of [
    'styles/dark',
    'planet',
    'planet/20261004_113936_pt/14/4567/6543.pbf',
    'natural_earth/ne2sr/6/3/4.png',
    'sprites/ofm_f384/ofm@2x.json',
    'sprites/ofm_f384/ofm.png',
    'fonts/Noto Sans Regular,Noto Sans Bold/0-255.pbf',
  ])
    assert(resourceKind(path), path);
  for (const path of [
    'https://evil.test/',
    '//evil.test',
    '../planet',
    'styles/../../secret',
    'styles/unknown',
    'planet/20261004_113936_pt/25/0/0.pbf',
    'planet/20261004_113936_pt/0/1/0.pbf',
    'fonts/Noto/0-256.pbf',
    'fonts/Noto/1-256.pbf',
    'fonts/Noto/65536-65791.pbf',
    'fonts/%2e%2e/0-255.pbf',
    'sprites/ofm_f384/evil.svg',
    'styles/dark?token=secret',
    'planet\\secret',
    'planet\u0000',
  ])
    assert.equal(resourceKind(path), null, path);
});

test('invalid paths, methods and secret query strings are rejected before provider calls', async () => {
  for (const [path, method] of [
    ['styles/unknown', 'GET'],
    ['styles/dark', 'POST'],
    ['styles/dark?token=secret', 'GET'],
    ['fonts/Noto%2520Sans/0-255.pbf', 'GET'],
    ['fonts/Noto%2fSans/0-255.pbf', 'GET'],
    ['fonts/%/0-255.pbf', 'GET'],
    ['styles/dark?url=https://evil.test/', 'GET'],
  ]) {
    const response = await mapResource(await request(MAP_PREFIX + path, { method }), () =>
      assert.fail('unexpected provider request')
    );
    assert(response.status >= 400, path);
  }
});

test('styles and tile manifests keep all resource loads on the authenticated app origin', async () => {
  const response = await mapResource(await request(MAP_PREFIX + 'styles/dark'), async () =>
    Response.json(style)
  );
  const rewritten = await response.json();
  for (const url of [rewritten.sprite, rewritten.glyphs, rewritten.sources.planet.url]) {
    assert(url.startsWith('https://panda.invalid' + MAP_PREFIX));
  }
  const manifest = await (
    await mapResource(await request(MAP_PREFIX + 'planet'), async () =>
      Response.json({
        tiles: ['https://__TILEJSON_DOMAIN__/planet/20261004_113936_pt/{z}/{x}/{y}.pbf'],
      })
    )
  ).json();
  assert(manifest.tiles[0].startsWith('https://panda.invalid' + MAP_PREFIX));
  for (const url of [
    'https://evil.test/planet',
    'http://169.254.169.254/',
    'https://tiles.openfreemap.org@evil.test/planet',
    '/v1/vehicles/private',
    'https://tiles.openfreemap.org/planet?key=secret',
  ]) {
    const badStyle = { ...style, sources: { planet: { type: 'vector', url } } };
    assert.equal(
      (
        await mapResource(await request(MAP_PREFIX + 'styles/dark'), async () =>
          Response.json(badStyle)
        )
      ).status,
      502
    );
  }
});

test('redirects, invalid media, malformed JSON, and provider errors are generic failures', async () => {
  const responses = [
    () => Response.redirect('https://evil.test/'),
    () => new Response('secret', { status: 503 }),
    () => new Response('<script>secret</script>', { headers: { 'Content-Type': 'text/html' } }),
    () => new Response('secret', { headers: { 'Content-Type': 'application/json' } }),
    () => Response.json({ ...style, imports: [{ url: 'https://evil.test' }] }),
  ];
  for (const response of responses) {
    const result = await mapResource(await request(MAP_PREFIX + 'styles/dark'), async () =>
      response()
    );
    assert.equal(result.status, 502);
    assert.doesNotMatch(await result.text(), /secret|evil\.test/);
  }
});

test('declared and streamed oversized responses are rejected and cancelled', async () => {
  for (const declared of [false, true]) {
    let cancelled = false;
    const body = new ReadableStream({
      pull(controller) {
        controller.enqueue(new Uint8Array(600 * 1024));
      },
      cancel() {
        cancelled = true;
      },
    });
    const headers = {
      'Content-Type': 'application/json',
      ...(declared ? { 'Content-Length': String(600 * 1024) } : {}),
    };
    const response = await mapResource(
      await request(MAP_PREFIX + 'styles/dark'),
      async () => new Response(body, { headers })
    );
    assert.equal(response.status, 502);
    assert.equal(cancelled, true);
  }
});

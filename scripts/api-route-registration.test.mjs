import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { routeSecurityInventory } from './check-api-route-security.mjs';

const factories = [
  '(Public, auth::router)',
  '(Metadata, auth::metadata_router)',
  '(Common, auth::protected_router)',
  '(Common, vehicles::router)',
  '(Heavy, live::router)',
  '(Upload, backups::upload_router)',
  '(Common, backups::router)',
  '(Themes, themes::router)',
];

function fixture(t, entries = factories, extra = '') {
  const root = mkdtempSync(join(tmpdir(), 'route-registration-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const directory = join(root, 'apps/api/src/routes');
  mkdirSync(directory, { recursive: true });
  for (const module of ['auth', 'vehicles', 'live', 'backups', 'themes'])
    writeFileSync(join(directory, `${module}.rs`), 'pub fn router() -> Router { route_table([]) }');
  writeFileSync(
    join(directory, 'mod.rs'),
    `fn registrations() -> Vec<(RouteScope, RouteFactory)> {\n  vec![${entries.join(',')}]\n}\n${extra}`
  );
  return root;
}

test('route factories remain visible when modules use the shared route table', (t) => {
  assert.deepEqual(routeSecurityInventory(fixture(t)), { count: 5, failures: [] });
});

test('missing factory is rejected even when a test contains the old merge expression', (t) => {
  const root = fixture(
    t,
    factories.filter((entry) => !entry.includes('vehicles::')),
    '#[cfg(test)]\nmod tests { fn smoke() { Router::new().merge(vehicles::router()); } }'
  );
  assert.deepEqual(routeSecurityInventory(root).failures, [
    'API route modules are not mounted behind the protected router: vehicles',
  ]);
});

test('both authenticated auth factories are required in the registration table', (t) => {
  const root = fixture(t, factories.filter((entry) => !entry.includes('auth::metadata_router')));
  assert.deepEqual(routeSecurityInventory(root).failures, [
    'API route modules are not mounted behind the protected router: auth',
  ]);
});

for (const [factory, expected, wrong] of [
  ['auth::router', 'Public', 'Common'],
  ['auth::metadata_router', 'Metadata', 'Public'],
  ['auth::protected_router', 'Common', 'Public'],
  ['vehicles::router', 'Common', 'Public'],
  ['live::router', 'Heavy', 'Common'],
  ['backups::upload_router', 'Upload', 'Common'],
  ['themes::router', 'Themes', 'Common'],
]) {
  test(`rejects the wrong protection scope for ${factory}`, (t) => {
    const entries = factories.map((entry) => entry.replace(`(${expected}, ${factory})`, `(${wrong}, ${factory})`));
    assert.deepEqual(routeSecurityInventory(fixture(t, entries)).failures, [
      `API route registration ${factory} uses ${wrong}; expected ${expected}`,
    ]);
  });
}

test('unknown route factories still require authorization matrix entries', (t) => {
  const root = fixture(t, [...factories, '(Common, unreviewed::router)']);
  writeFileSync(join(root, 'apps/api/src/routes/unreviewed.rs'), 'pub fn router() -> Router { route_table([]) }');
  assert.deepEqual(routeSecurityInventory(root).failures, [
    'Mounted API route modules missing authorization-matrix entries: unreviewed',
  ]);
});

test('the integrated registration table passes the inventory', () => {
  const root = fileURLToPath(new URL('..', import.meta.url));
  assert.match(readFileSync(join(root, 'apps/api/src/routes/mod.rs'), 'utf8'), /fn registrations\(\)/);
  assert.deepEqual(routeSecurityInventory(root).failures, []);
});

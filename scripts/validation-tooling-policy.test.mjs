import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { colorTokenViolations } from './check-color-tokens.mjs';
import { architectureGuardFailures, runArchitectureGuards } from './check-architecture-guards.mjs';
import { routeSecurityInventory } from './check-api-route-security.mjs';

function fixture(t, files) {
  const root = mkdtempSync(join(tmpdir(), 'validation-policy-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const [file, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    writeFileSync(join(root, file), content);
  }
  return root;
}

test('color policy covers all consumer roots, accepted owners, file exclusions and every raw-color family', (t) => {
  const root = fixture(t, {
    'apps/web/src/Raw.tsx': 'const color = "#aab";\nconst safe = "var(--color-text)";\n',
    'packages/ui/src/Raw.tsx': 'color: rgb(1, 2, 3)\n',
    'packages/dashboards/src/Raw.css': 'background: rgba(1, 2, 3, .5);\n',
    'packages/hooks/src/raw.ts': 'const className = "bg-red-500/50";\n',
    'packages/themes/src/raw.ts': 'const className = "border-blue";\n',
    'packages/themes/src/index.ts': 'const color = "#abc";\n',
    'packages/ui/src/tokens/globals.css': ':root { --color: #abc; }',
    'packages/ui/src/Raw.test.tsx': 'const color = "#fff";',
    'packages/ui/src/Raw.spec.ts': 'const color = "rgb(0,0,0)";',
    'packages/ui/src/Raw.stories.tsx': 'const className = "bg-black";',
    'apps/web/src/dist/Raw.tsx': 'const color = "#fff";',
    'apps/web/src/unknown.json': '{"color":"#fff"}',
  });
  const failures = colorTokenViolations(root);
  assert.equal(failures.length, 5);
  assert.match(failures[0], /^apps\/web\/src\/Raw.tsx:1:/);
  assert.ok(failures.some((line) => line.startsWith('packages/themes/src/raw.ts:1:')));
});

test('architecture byte budgets normalize CRLF, retain zero-line files and report missing hotspots', (t) => {
  const root = fixture(t, {
    'apps/web/src/empty.ts': '',
    'apps/web/src/lines.ts': 'a\r\nb\r',
    'config/architecture-budgets.json': JSON.stringify({
      hotspots: {
        'apps/web/src/empty.ts': { maxLines: 0, maxBytes: 0 },
        'apps/web/src/lines.ts': { maxLines: 2, maxBytes: 4 },
        'apps/web/src/missing.ts': { maxLines: 1 },
      },
    }),
  });
  assert.deepEqual(architectureGuardFailures(root), [
    'Architecture hotspot is missing from its budget: apps/web/src/missing.ts',
  ]);
});

test('architecture child command failures preserve both diagnostic channels without skipping later guards', (t) => {
  const root = fixture(t, {
    'config/architecture-budgets.json': '{}',
    'scripts/sync-dashboard-defaults.mjs':
      'console.error("synthetic dashboard drift"); process.exitCode=1;',
    'scripts/check-api-route-security.mjs':
      'console.error("synthetic policy failure"); process.exitCode=1;',
  });
  assert.deepEqual(runArchitectureGuards(root), [
    'Dashboard defaults are not in sync:\nsynthetic dashboard drift',
    'API route security guard failed:\nsynthetic policy failure',
  ]);
});

test('route inventory excludes declared test-only modules but requires both auth mounts', (t) => {
  const root = fixture(t, {
    'apps/api/src/routes/mod.rs':
      '.merge(auth::protected_router()).merge(auth::metadata_router()).merge(battery::router())',
    'apps/api/src/routes/auth.rs': '.route("/login", handler)',
    'apps/api/src/routes/battery.rs':
      '#[cfg(test)]\n#[path = "battery_support.rs"]\nmod tests;\n.route("/battery", handler)',
    'apps/api/src/routes/battery_support.rs':
      'require_vehicle_owned(); .route("/synthetic-test", handler)',
  });
  assert.deepEqual(routeSecurityInventory(root), { count: 3, failures: [] });
  writeFileSync(
    join(root, 'apps/api/src/routes/mod.rs'),
    '.merge(auth::protected_router()).merge(battery::router())'
  );
  assert.deepEqual(routeSecurityInventory(root).failures, [
    'API route modules are not mounted behind the protected router: auth',
  ]);
});

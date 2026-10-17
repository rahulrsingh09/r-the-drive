import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const repo = fileURLToPath(new URL('..', import.meta.url));
const tracked = execFileSync('git', ['ls-files', '-z'], { cwd: repo, encoding: 'utf8' })
  .split('\0')
  .filter(Boolean);
function fixture(t, prefixes = ['']) {
  const root = mkdtempSync(join(tmpdir(), 'validation-command-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const file of tracked.filter((file) => prefixes.some((prefix) => file.startsWith(prefix)))) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    copyFileSync(join(repo, file), join(root, file));
  }
  symlinkSync(join(repo, 'node_modules'), join(root, 'node_modules'), 'dir');
  const page = 'architecture/validation-tooling-replacement';
  const sidebar = join(root, 'apps/docs/sidebars.ts');
  if (existsSync(join(root, 'docs', page + '.md')) && existsSync(sidebar)) {
    const contents = readFileSync(sidebar, 'utf8');
    if (!contents.includes(`'${page}'`))
      writeFileSync(
        sidebar,
        contents.replace('developmentSidebar: [', `developmentSidebar: [\n    '${page}',`)
      );
  }
  return root;
}
function command(root, file, args = []) {
  const result = spawnSync(process.execPath, [join(root, file), ...args], {
    cwd: root,
    encoding: 'utf8',
    env: { PATH: process.env.PATH, HOME: tmpdir(), CI: 'true' },
  });
  return { ...result, output: result.stdout + result.stderr };
}
function edit(root, file, before, after) {
  const body = readFileSync(join(root, file), 'utf8');
  assert.ok(body.includes(before), `fixture seam missing: ${file}`);
  writeFileSync(join(root, file), body.replace(before, after));
}

test('default sync reports every drift without writing, then is byte-idempotent', (t) => {
  const root = fixture(t, [
    'scripts/',
    'packages/dashboards/src/',
    'apps/api/charts/',
    'apps/api/dashboards/',
  ]);
  for (const [script, target] of [
    ['scripts/sync-chart-defaults.mjs', 'apps/api/charts/defaults.json'],
    ['scripts/sync-dashboard-defaults.mjs', 'apps/api/dashboards/dashboard.json'],
  ]) {
    const before = readFileSync(join(root, target));
    writeFileSync(join(root, target), 'synthetic drift\n');
    const check = command(root, script, ['--check']);
    assert.equal(check.status, 1, check.output);
    assert.match(check.stderr, /drift:/);
    assert.equal(readFileSync(join(root, target), 'utf8'), 'synthetic drift\n');
    assert.equal(command(root, script).status, 0);
    assert.deepEqual(readFileSync(join(root, target)), before);
    const repeat = command(root, script);
    assert.equal(repeat.status, 0);
    assert.equal(repeat.stdout, '');
    const synced = command(root, script, ['--check']);
    assert.equal(synced.status, 0, synced.output);
    assert.match(synced.stdout, /in sync/);
  }
});

test('route guard refuses unmounted modules, missing matrix entries, and deprecated authorization helpers', (t) => {
  const root = fixture(t, ['scripts/', 'apps/api/src/routes/']);
  assert.equal(command(root, 'scripts/check-api-route-security.mjs').status, 0);
  writeFileSync(
    join(root, 'apps/api/src/routes/synthetic.rs'),
    'pub fn router() { Router::new().route("/synthetic", get(handler)); }'
  );
  assert.match(command(root, 'scripts/check-api-route-security.mjs').stderr, /not mounted/);
  edit(
    root,
    'apps/api/src/routes/mod.rs',
    '(Common, battery::router),',
    '(Common, battery::router),\n(Common, synthetic::router),'
  );
  assert.match(
    command(root, 'scripts/check-api-route-security.mjs').stderr,
    /authorization-matrix/
  );
  rmSync(join(root, 'apps/api/src/routes/synthetic.rs'));
  edit(
    root,
    'apps/api/src/routes/battery.rs',
    'pub fn router() -> Router<AppState> {',
    'pub fn router() -> Router<AppState> {\nrequire_vehicle_owned();'
  );
  assert.match(command(root, 'scripts/check-api-route-security.mjs').stderr, /Deprecated/);
});

test('docs check reports broken links, missing env coverage and insecure production contracts', (t) => {
  const root = fixture(t);
  assert.equal(command(root, 'scripts/check-docs.mjs').status, 0);
  writeFileSync(
    join(root, 'docs/decision-log.md'),
    readFileSync(join(root, 'docs/decision-log.md'), 'utf8') +
      '\n[Missing](./nonexistent-validation-fixture.md)\n'
  );
  edit(root, 'compose/.env.full.example', 'TZ=', 'SYNTHETIC_UNDOCUMENTED=\nTZ=');
  edit(root, 'compose/docker-compose.yml', 'start_period: 300s', 'start_period: 1s');
  const result = command(root, 'scripts/check-docs.mjs');
  assert.equal(result.status, 1);
  for (const message of [/broken markdown link/, /SYNTHETIC_UNDOCUMENTED/, /start_period: 300s/])
    assert.match(result.stderr, message);
});

test('dependency policy rejects catalog bypass and strict install/action policy changes', (t) => {
  const root = fixture(t);
  assert.equal(command(root, 'scripts/check-dependency-policy.mjs').status, 0);
  const manifest = JSON.parse(readFileSync(join(root, 'apps/web/package.json')));
  const name = Object.keys(manifest.dependencies).find(
    (name) => manifest.dependencies[name] === 'catalog:'
  );
  manifest.dependencies[name] = '0.0.0';
  writeFileSync(join(root, 'apps/web/package.json'), JSON.stringify(manifest));
  assert.match(
    command(root, 'scripts/check-dependency-policy.mjs', ['--catalogs-only']).stderr,
    /must use catalog:/
  );
  edit(root, 'pnpm-workspace.yaml', 'esbuild: true', 'esbuild: false');
  edit(root, '.github/workflows/quality.yml', 'actions/checkout@', 'actions/checkout@mutable-');
  const result = command(root, 'scripts/check-dependency-policy.mjs');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /build allowlist/);
  assert.match(result.stderr, /mutable action/);
});

test('Compose guard rejects publication, cookie and initialization policy regressions', (t) => {
  const root = fixture(t, ['scripts/', 'compose/']);
  const target = join(root, 'compose/docker-compose.yml');
  const original = readFileSync(target, 'utf8');
  assert.equal(command(root, 'scripts/check-compose-contracts.mjs').status, 0);
  for (const [before, after, message] of [
    ['127.0.0.1', '0.0.0.0', /host publication/],
    ['start_period: 300s', 'start_period: 3s', /slow first initialization/],
    ['retries: 30', 'retries: 3', /slow first initialization/],
  ]) {
    writeFileSync(target, original.replace(before, after));
    const result = command(root, 'scripts/check-compose-contracts.mjs');
    assert.equal(result.status, 1);
    assert.match(result.stderr, message);
  }
});

test('release contract rejects mutable promotion, premature tagging and missing smoke gates', (t) => {
  const root = fixture(t);
  assert.equal(command(root, 'scripts/check-release-workflows.mjs').status, 0);
  edit(
    root,
    '.github/workflows/publish-release-images.yml',
    'f"{image}:candidate-{source}-{arch}"',
    'f"{image}:latest-{arch}"'
  );
  edit(
    root,
    '.github/workflows/prepare-release.yml',
    'docker buildx imagetools inspect "$candidate"',
    'echo skipped'
  );
  const result = command(root, 'scripts/check-release-workflows.mjs');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /exact AMD64/);
  assert.match(result.stderr, /before pushing/);
});

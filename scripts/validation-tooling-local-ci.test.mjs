import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { createLocalCi } from './local-ci.mjs';

function harness(
  t,
  { env = {}, head = true, fail = () => false, tracked = 'README.md', colors = [] } = {}
) {
  const root = mkdtempSync(join(tmpdir(), 'validation-ci-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const calls = [],
    sleeps = [];
  const execute = (command, args, options) => {
    calls.push({ command, args, options });
    if (fail(command, args, calls)) return { status: 7, stdout: '' };
    if (command === 'git' && args[0] === 'ls-files') return { status: 0, stdout: tracked };
    if (command === 'git' && args[0] === 'rev-parse')
      return { status: head ? 0 : 1, stdout: head ? 'synthetic-head' : '' };
    return { status: 0, stdout: '' };
  };
  const run = createLocalCi({
    root,
    environment: env,
    execute,
    colors: () => colors,
    sleep: async (milliseconds) => {
      sleeps.push(milliseconds);
    },
  });
  return { root, calls, sleeps, run };
}

test('hook gate uses staged whitespace on initial commit plus migration checks; bypass has no commands', async (t) => {
  const fresh = harness(t, { head: false });
  await fresh.run('hook');
  assert.deepEqual(
    fresh.calls.map((c) => [c.command, ...c.args]),
    [
      ['git', 'ls-files'],
      ['git', 'rev-parse', '--verify', 'HEAD'],
      ['git', 'diff', '--cached', '--check'],
      ['node', 'tools/migration-integrity.mjs'],
    ]
  );
  const bypass = harness(t, { env: { SKIP_LOCAL_CI: '1', PANDA_FULL_LOCAL_CI: '1' } });
  await bypass.run('hook');
  assert.deepEqual(bypass.calls, []);
});

test('hygiene refuses tracked generated files before tests', async (t) => {
  const fixture = harness(t, { tracked: 'README.md\napps/api/target/generated.tmp' });
  await assert.rejects(fixture.run('hook'), /Generated or local-only files/);
  assert.equal(fixture.calls.length, 1);
});

test('local command executes the complete check order and stops on failure', async (t) => {
  const fixture = harness(t);
  await fixture.run('local');
  const labels = fixture.calls
    .filter((c) => c.options.stdio === 'inherit')
    .map((c) => c.options.label);
  assert.deepEqual(labels, [
    'Install locked dependencies',
    'Repository whitespace check',
    'Migration integrity',
    'Dependency policy',
    'Peer dependencies',
    'Lint',
    'Typecheck',
    'Documentation check',
    'Compose contract',
    'Release workflow contract',
    'Compose render check',
    'Architecture guards',
    'Route security inventory',
    'Dashboard default drift',
    'Chart default drift',
    'Workspace tests',
    'API library tests',
    'Rust format check',
  ]);
  const failed = harness(t, { fail: (_, args) => args[0] === 'deps:check' });
  await assert.rejects(failed.run('local'), /Dependency policy failed with exit code 7/);
  assert.equal(failed.calls.at(-1).args[0], 'deps:check');
  const colored = harness(t, { colors: ['bad color'] });
  await assert.rejects(colored.run('local'), /design tokens/);
});

test('full CI retries migration startup then checks SQLx, clippy, DB tests and exact ignored guard before cleanup', async (t) => {
  let migrationCalls = 0;
  const fixture = harness(t, {
    fail: (command, args) => command === 'cargo' && args[0] === 'run' && ++migrationCalls < 3,
  });
  await fixture.run('ci');
  assert.equal(migrationCalls, 3);
  assert.deepEqual(fixture.sleeps, [2000, 2000]);
  const calls = fixture.calls;
  const sqlx = calls.find(
    (c) => c.command === 'cargo' && c.args[0] === 'sqlx' && c.args[1] === 'prepare'
  );
  assert.deepEqual(sqlx.args, [
    'sqlx',
    'prepare',
    '--check',
    '--workspace',
    '--',
    '--all-targets',
    '--all-features',
  ]);
  const ignored = calls.find((c) => c.args.includes('--ignored'));
  assert.deepEqual(ignored.args, [
    'test',
    '--lib',
    'db::migrations::tests::valid_public_ledger_removes_empty_legacy_ledger',
    '--',
    '--ignored',
    '--exact',
    '--test-threads=1',
  ]);
  assert.equal(calls.filter((c) => c.args.includes('down')).length, 1);
  assert.deepEqual(calls.at(-1).args.slice(-3), ['down', '--volumes', '--remove-orphans']);
  assert.equal(ignored.options.env.PANDA_ENV, 'development');
  assert.equal(
    ignored.options.env.CARGO_TARGET_DIR,
    join(fixture.root, 'apps/api/target-ci-local')
  );
});

test('full CI cleans up after exhausted retries and never creates a PR after a failed gate', async (t) => {
  const fixture = harness(t, { fail: (command, args) => command === 'cargo' && args[0] === 'run' });
  await assert.rejects(fixture.run('create-pr', ['--draft']), /Canonical fork migrations failed/);
  assert.equal(fixture.sleeps.length, 4);
  assert.equal(
    fixture.calls.some((c) => c.command === 'gh'),
    false
  );
  assert.equal(fixture.calls.filter((c) => c.args.includes('down')).length, 1);
});

test('hook config and PR argument forwarding run only through the injected executor', async (t) => {
  const fixture = harness(t);
  await fixture.run('install-hooks');
  await fixture.run('uninstall-hooks');
  assert.deepEqual(
    fixture.calls.map((c) => c.args),
    [
      ['config', 'core.hooksPath', '.githooks'],
      ['config', '--unset', 'core.hooksPath'],
    ]
  );
  await fixture.run('create-pr', ['--draft', '--title', 'synthetic']);
  assert.deepEqual(fixture.calls.at(-1).args, ['pr', 'create', '--draft', '--title', 'synthetic']);
  await assert.rejects(fixture.run('invalid'), /Usage:/);
});

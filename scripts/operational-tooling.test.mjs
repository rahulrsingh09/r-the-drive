import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import { Commands, pollUntil, runChild } from './operational-runtime.mjs';
import { database } from './database.mjs';
import { imageBuildPlan } from './verify-release-image.mjs';
import { restore, restorePlan } from './restore-backup.mjs';
import { procedureArguments } from './dev-harness-procedure.mjs';
import { runSynchronization } from './fork-sync-runner.mjs';
import { POLICY } from './fork-deploy-readiness.mjs';
import { DevelopmentStack, parsePids, extractViteLocalUrl } from './dev.mjs';
import { inspectPackage, parseArgs, buildPlan, executePlan } from './dev-harness.mjs';
import { FreshInstall, readSetupToken, sortJson } from './verify-fresh-install.mjs';
import { PopulatedUpgrade, parseStats } from './verify-populated-upgrade.mjs';
import { S3RestoreDrill } from './verify-backup-restore-s3.mjs';
import { RestoreLab, verifyDatabase, manifestLedgerVersions, targetLedgerVersions, assertManifestExpectations } from '../tools/restore-lab/run.mjs';

function directory(t) {
  const path = mkdtempSync(join(tmpdir(), 'operational-test-'));
  t.after(() => rmSync(path, { recursive: true, force: true }));
  return path;
}
const instantPoll = async (probe, options) => {
  for (let count = 0; count < 15; count += 1) { const result = await probe(); if (result) return result; }
  throw new Error(options.message);
};
function restoreFixture(t) {
  const dir = directory(t);
  const files = ['fixture.rma.tar.gz', '.env', 'compose.yml'].map(name => join(dir, name));
  files.forEach(file => writeFileSync(file, 'synthetic'));
  writeFileSync(files[1], 'POSTGRES_USER=synthetic\nPANDA_ORIGIN_PORT=19991\nPANDA_HEALTH_HOST=127.0.0.1\n');
  const args = ['--package', files[0], '--env-file', files[1], '--compose-file', files[2], '--project', 'operational-test', '--force'];
  return { dir, files, args, plan: restorePlan(args, { FIXTURE: 'only' }) };
}

test('command boundary captures exact argv/env and never uses a shell implicitly', () => {
  const calls = [];
  const commands = new Commands('/synthetic', { ONLY: 'fixture' }, (...args) => { calls.push(args); return ' answer\n'; });
  assert.equal(commands.invoke('synthetic', ['literal;$(never)'], { capture: true }), 'answer');
  assert.deepEqual(calls[0], ['synthetic', ['literal;$(never)'], { cwd: '/synthetic', env: { ONLY: 'fixture' }, encoding: 'utf8' }]);
});

test('child and bounded polling propagate failures without retrying mutations', async () => {
  const child = new EventEmitter();
  let launches = 0;
  const pending = runChild('synthetic', ['one'], {}, () => { launches++; queueMicrotask(() => child.emit('exit', 7, null)); return child; });
  await assert.rejects(pending, /exit code 7/); assert.equal(launches, 1);
  let time = 0, probes = 0;
  await assert.rejects(pollUntil(() => { probes++; return false; }, { timeout: 3, interval: 1,
    now: () => time, sleep: async n => { time += n; }, message: 'deadline' }), /deadline/);
  assert.equal(probes, 3);
});

test('database reset validates first and stops exactly at the failed command', async () => {
  const calls = [];
  const executor = async (...args) => { calls.push(args); if (calls.length === 2) throw new Error('synthetic-create-failure'); };
  await assert.rejects(database(['reset', '--yes'], { DATABASE_URL: 'postgres://fixture@remote.invalid/panda' }, executor));
  assert.equal(calls.length, 0);
  const env = { PANDA_ALLOW_DATABASE_RESET: 'local-development', DATABASE_URL: 'postgres://fixture@127.0.0.1/panda', PANDA_ENV: 'development' };
  await assert.rejects(database(['reset', '--yes'], env, executor), /synthetic-create-failure/);
  assert.deepEqual(calls.map(([cmd, args]) => [cmd, args]), [['sqlx', ['database', 'drop', '-y']], ['sqlx', ['database', 'create']]]);
  calls.length = 0;
  await database(['migrate'], {}, executor);
  assert.equal(calls[0][2].env.SQLX_OFFLINE, 'true');
  assert.equal(calls[0][2].env.PGOPTIONS, '-c search_path=public');
});

test('release image contract exports OCI locally with bounded timeout and no push/load', () => {
  const plan = imageBuildPlan(['--all-platforms', '--platforms', 'ignored', '--no-cache', '--cargo-build-jobs', '2', '--cache-ref', 'registry.invalid/cache'], 17);
  assert.equal(plan.platforms, 'linux/amd64,linux/arm64');
  assert.ok(plan.args.includes('type=oci,dest=' + plan.output));
  assert.ok(plan.args.includes('CARGO_BUILD_JOBS=2'));
  assert.ok(!plan.args.includes('--push') && !plan.args.includes('--load'));
  for (const value of ['0', '-1', 'NaN', 'Infinity']) assert.throws(() => imageBuildPlan(['--timeout-minutes', value]), /positive/);
});

test('restore requires five consecutive database successes and preserves read-only package mount', async t => {
  const { plan } = restoreFixture(t), calls = [], readiness = [0, 0, 1, 0, 0, 0, 0, 0];
  let probes = 0;
  await restore(plan, { commands: { invoke: (command, args) => { calls.push([command, args]); } },
    probe: (_command, args, options) => {
      assert.deepEqual(args.slice(-3), ['-At', '-c', 'SELECT 1']);
      assert.equal(options.env.FIXTURE, 'only'); probes++; return { status: readiness.shift() };
    }, fetchImpl: async url => url.endsWith('/health') ? { ok: true } : { json: async () => ({ setup_required: false }) }, poll: instantPoll });
  assert.equal(probes, 8);
  const actions = calls.map(([, args]) => args.find(arg => arg.startsWith('host-'))).filter(Boolean);
  assert.deepEqual(actions, ['host-restore', 'host-activate', 'host-finalize']);
  assert.ok(calls.some(([, args]) => args.includes(`${plan.packagePath}:/restore-package:ro`) && args.includes('--force')));
});

for (const failure of ['host-restore', 'host-activate', 'setup', 'host-finalize', 'rollback']) {
  test(`restore state machine contains failure at ${failure}`, async t => {
    const { plan } = restoreFixture(t), actions = [], starts = [];
    const injected = new Error('original-failure');
    const commands = { invoke: (_command, args) => {
      const action = args.find(arg => arg.startsWith('host-'));
      if (action) actions.push(action);
      if (args.includes('up') && args.at(-1) === 'panda') starts.push('start');
      if (action === failure || (failure === 'rollback' && action === 'host-rollback')) throw injected;
    } };
    const fetchImpl = async url => {
      if (url.endsWith('/health')) return { ok: true };
      if (['setup', 'rollback'].includes(failure)) throw injected;
      return { json: async () => ({ setup_required: false }) };
    };
    await assert.rejects(restore(plan, { commands, probe: () => ({ status: 0 }), fetchImpl, poll: instantPoll }), error => error === injected);
    assert.equal(actions.includes('host-rollback'), failure !== 'host-restore');
    if (failure === 'host-activate') assert.equal(starts.length, 0);
    if (failure === 'setup' || failure === 'host-finalize') assert.equal(starts.length, 2);
    if (failure === 'rollback') assert.equal(starts.length, 1);
  });
}

test('procedure maps only named non-secret inputs and preserves explicit plan preference', () => {
  const env = Object.fromEntries(['BASELINE_IMAGE','DEV_IMAGE','PRODUCTION_ROOT','PRODUCTION_PORT','TEST_ROOT','ENV_FILE','COMPOSE_FILE','PROJECT']
    .map(key => ['PANDA_HARNESS_' + key, ' synthetic-' + key + ' ']));
  Object.assign(env, { PANDA_HARNESS_PACKAGE: ' archive ', PANDA_HARNESS_PLAN_ONLY: 'TRUE', PANDA_HARNESS_RESET_TEST_STORAGE: 'true', UNRELATED_SECRET: 'never-forward' });
  const args = procedureArguments(env);
  assert.deepEqual(args.slice(0, 2), ['--package', 'archive']);
  assert.ok(args.includes('--plan') && args.includes('--reset-test-storage') && !args.includes('--execute'));
  assert.ok(!args.includes('never-forward'));
  assert.throws(() => procedureArguments({}), /PACKAGE/);
});

test('synchronization wrapper scrubs both credentials before invoking its isolated preparation', () => {
  const env = { GITHUB_REPOSITORY: POLICY.repository, GITHUB_REF: 'refs/heads/' + POLICY.branch,
    GITHUB_WORKFLOW_REF: `${POLICY.repository}/.github/workflows/fork-upstream-sync.yml@refs/heads/${POLICY.branch}`,
    GITHUB_SHA: 'a'.repeat(40), GITHUB_EVENT_NAME: 'schedule', FORK_SYNC_SOURCE: 'dev', FORK_SYNC_SSH_KEY: 'synthetic', FORK_SYNC_TOKEN: 'discard' };
  let called = false;
  const result = runSynchronization({ env }, input => {
    called = true; assert.equal(env.FORK_SYNC_TOKEN, undefined); assert.equal(env.FORK_SYNC_SSH_KEY, undefined);
    assert.equal(input.source, 'main'); assert.equal(input.sshKey, 'synthetic'); return { status: 'current' };
  });
  assert.equal(result.status, 'current'); assert.ok(called);
  assert.equal(runSynchronization({ env: {} }, () => assert.fail('untrusted context executed')).status, 'error');
});

test('development launcher keeps PID parsing, Vite discovery and key reuse local', async t => {
  assert.deepEqual(parsePids('12 12 0 nope 34', 34), [12]);
  assert.equal(extractViteLocalUrl('\u001b[32m Local: https://localhost:5179/\u001b[0m'), 'http://localhost:5179');
  const stack = new DevelopmentStack({ env: {}, platform: 'linux', log: () => {} });
  const dir = directory(t); stack.keyFile = join(dir, 'key'); stack.markerFile = join(dir, 'marker');
  writeFileSync(stack.keyFile, 'saved synthetic key\n'); writeFileSync(stack.markerFile, 'old');
  await stack.ensureKey(); assert.equal(readFileSync(stack.keyFile, 'utf8'), 'saved synthetic key\n'); assert.equal(existsSync(stack.markerFile), false);
  const child = new EventEmitter(); child.stdout = new EventEmitter(); child.stderr = new EventEmitter();
  const url = stack.viteUrl(child, 1000); child.stdout.emit('data', 'Local: http://localhost:5181/');
  assert.equal(await url, 'http://localhost:5181'); assert.equal(child.stdout.listenerCount('data'), 0);
});

test('development Compose identity rejection precedes all listener and launch actions', async () => {
  const stack = new DevelopmentStack({ env: {}, log: () => {} });
  stack.capture = async () => ({ code: 0, stdout: JSON.stringify({ Service: 'redis' }), stderr: '' });
  stack.listeners = async () => assert.fail('probed after missing identity');
  await assert.rejects(stack.allocatePorts(), /expected development project/);
  assert.equal(stack.env.DEV_API_PORT, undefined);
});

test('Windows process ownership is decided locally from exact binaries and repository Vite commands', async () => {
  const stack = new DevelopmentStack({ env: {}, platform: 'win32', log: () => {} });
  const processes = [
    { ProcessId: 1, Name: 'api', ExecutablePath: stack.binary('panda-api').toUpperCase() },
    { ProcessId: 2, Name: 'restore', ExecutablePath: stack.binary('panda-restore-agent') },
    { ProcessId: 3, Name: 'node', CommandLine: stack.webDir.toUpperCase() + ' VITE.CMD' },
    { ProcessId: 4, Name: 'foreign', ExecutablePath: '/foreign/panda-api.exe' },
    { ProcessId: 5, Name: 'foreign-vite', CommandLine: '/foreign/vite.js' },
    { ProcessId: 6, Name: 'same-repo', CommandLine: stack.webDir + ' vitest' },
    { ProcessId: 0, Name: 'invalid', ExecutablePath: stack.binary('panda-api') },
  ];
  stack.capture = async (command, args) => {
    assert.equal(command, 'powershell.exe');
    assert.ok(!args.at(-1).includes(stack.webDir));
    return { code: 0, stdout: JSON.stringify(processes), stderr: '' };
  };
  assert.deepEqual(await stack.ownedProcesses(), [
    { pid: 1, name: 'api', kind: 'api' }, { pid: 2, name: 'restore', kind: 'restore-agent' },
    { pid: 3, name: 'node', kind: 'vite' },
  ]);
  stack.capture = async () => ({ code: 0, stdout: '{malformed', stderr: '' });
  assert.deepEqual(await stack.ownedProcesses(), []);
  stack.capture = async () => ({ code: 1, stdout: JSON.stringify(processes), stderr: 'unavailable' });
  assert.deepEqual(await stack.ownedProcesses(), []);
});

function archive(files) {
  const blocks = [];
  for (const [path, content] of files) {
    const data = Buffer.from(content), header = Buffer.alloc(512);
    header.write(path); header.write(data.length.toString(8).padStart(11, '0'), 124); header[156] = 48;
    blocks.push(header, data, Buffer.alloc((512 - data.length % 512) % 512));
  }
  return gzipSync(Buffer.concat([...blocks, Buffer.alloc(1024)]));
}
function harnessFixture(t, version = 3) {
  const dir = directory(t), path = join(dir, 'fixture.rma.tar.gz');
  const data = { 'database.dump': 'PGDMP-synthetic', 'backup-settings.json': '{}', 'operational-history.json': '{}' };
  const components = {};
  for (const [name, file] of [['database','database.dump'],['backup_settings','backup-settings.json'],['operational_history','operational-history.json']])
    components[name] = { path: file, sha256: createHash('sha256').update(data[file]).digest('hex'), size_bytes: Buffer.byteLength(data[file]), version: 1, restore_policy: 'fixture', redactions: [] };
  const manifest = { format: 'panda-recovery-v' + version, format_version: version, components,
    source: { postgres_major: 18, migration_version: 1, migration_ledger: [{ version: 1, checksum_sha384: 'synthetic' }], schema_fingerprint: 'synthetic',
      migration_chain_id: 'synthetic', migration_catalog_digest: 'synthetic', schema_contract_version: '1', app_version: 'synthetic' }, restore: { engine_version: 3 } };
  const save = () => {
    const bytes = archive([['manifest.json', JSON.stringify(manifest)], ...Object.entries(data)]); writeFileSync(path, bytes);
    writeFileSync(path + '.sha256', createHash('sha256').update(bytes).digest('hex'));
  };
  save();
  const envFile = join(dir, 'fixture.env'), composeFile = join(dir, 'compose.yml');
  writeFileSync(envFile, 'PANDA_ENV=development\nCOOKIE_INSECURE=false\n'); writeFileSync(composeFile, 'services: {}\n');
  const raw = { packagePath: path, baselineImage: 'registry.invalid/base:1@sha256:' + 'a'.repeat(64), devImage: 'registry.invalid/dev:2@sha256:' + 'b'.repeat(64),
    productionRoot: join(dir, 'production'), productionPort: 19000, testRoot: join(dir, 'target-test'), envFile, composeFile,
    port: 19001, healthHost: '127.0.0.1', project: 'operational-test', reset: false };
  return { dir, path, raw, manifest, save };
}

test('recovery package v1-v3 contracts reject corrupt components and mismatched ledgers', t => {
  for (const version of [1,2,3]) {
    const f = harnessFixture(t, version);
    assert.equal(inspectPackage(f.path, undefined, { requireChecksum: true }).manifest.format_version, version);
    f.manifest.components.database.sha256 = '0'.repeat(64); f.save();
    assert.throws(() => inspectPackage(f.path), /checksum or size mismatch/);
  }
  const f = harnessFixture(t); f.manifest.source.migration_version = 2; f.save();
  assert.throws(() => inspectPackage(f.path), /does not end/);
  f.manifest.source.migration_version = 1; f.manifest.components.database.restore_policy = ''; f.save();
  assert.throws(() => inspectPackage(f.path), /invalid contract/);
});

test('harness executor records baseline before upgrade and does not mark a failed upgrade complete', async t => {
  const f = harnessFixture(t), plan = buildPlan(f.raw), commands = [];
  const execute = (command, args, options) => {
    commands.push([command, args, options]);
    return args.includes('psql') ? '1,2,3,4,5,0\n' : '';
  };
  await executePlan(f.raw, plan, { execute, fetchImpl: async url => url.endsWith('/health') ? { ok: true } : { json: async () => ({ setup_required: false }) }, poll: instantPoll });
  const state = JSON.parse(readFileSync(plan.target.state)); assert.equal(state.phase, 'complete');
  assert.equal(state.baselineData.users, 1); assert.equal(state.devData.telemetry, 4);
  assert.equal(commands.filter(([, args]) => args.includes('--skip-build')).length, 1);
  assert.ok(commands.find(([, args]) => args.includes('up'))[2].env.PANDA_ENV_FILE.endsWith('/dev.env'));
  const failed = harnessFixture(t), failedPlan = buildPlan(failed.raw);
  await assert.rejects(executePlan(failed.raw, failedPlan, { execute: (command, args) => { if (args.includes('up')) throw new Error('upgrade-failed'); return execute(command, args); },
    fetchImpl: async () => assert.fail('health ran after failed up'), poll: instantPoll }), /upgrade-failed/);
  assert.equal(JSON.parse(readFileSync(failedPlan.target.lock)).phase, 'baseline-verified');
});

test('fresh owner setup sends proof once, closes registration and refuses missing proof before POST', async t => {
  const dir = directory(t), env = join(dir, 'env'); writeFileSync(env, 'PANDA_SETUP_TOKEN="synthetic-proof"\n');
  assert.equal(readSetupToken(env), 'synthetic-proof'); assert.deepEqual(sortJson({ z: [2, { b: 1, a: 2 }], a: 0 }), { a: 0, z: [2, { a: 2, b: 1 }] });
  const calls = [];
  const fresh = new FreshInstall(['--production-env', env], { poll: instantPoll, fetchImpl: async (url, options) => {
    calls.push([url, options]);
    if (url.endsWith('/health')) return { ok: true };
    if (url.endsWith('/setup')) return { json: async () => ({ setup_required: true, setup_proof_required: true }) };
    const body = JSON.parse(options.body);
    if (body.email.startsWith('owner')) { assert.equal(body.setup_token, 'synthetic-proof'); return { status: 201, json: async () => ({ access_token: 'synthetic-token' }) }; }
    assert.equal(body.setup_token, undefined); return { status: 403 };
  } });
  let dashboards = 0; fresh.dashboards = async () => { dashboards++; };
  assert.equal(await fresh.owner('http://127.0.0.1:1'), 'synthetic-token'); assert.equal(dashboards, 1);
  writeFileSync(env, 'SAFE=1'); calls.length = 0;
  await assert.rejects(fresh.owner('http://127.0.0.1:1'), /requires PANDA_SETUP_TOKEN/);
  assert.ok(calls.every(([, options]) => !options));
});

test('upgrade and restore-lab parsers preserve historical refusal contracts', () => {
  assert.deepEqual(parseStats('prefix status=running pending=1 payloads=250000 identities=200000'), { status: 'running', pending: 1, payloads: 250000, identities: 200000 });
  assert.throws(() => parseStats('not stats'), /Could not parse/);
  assert.throws(() => new PopulatedUpgrade({ DATABASE_URL: 'postgres://user@remote.invalid/panda' }), /./);
  assert.throws(() => manifestLedgerVersions({ source: { migration_ledger: [{ version: 1 }, { version: 3 }] } }), /not contiguous/);
  assert.deepEqual(targetLedgerVersions({ plan: { target: { migration_ledger: [{ version: 100 }] } } }, {}), [100]);
  assert.throws(() => assertManifestExpectations({ a: { b: 2 } }, { id: 'fixture', manifest_expectations: { 'a.b': 1 } }), /expected manifest/);
});

test('restore-lab database evidence requires histories, approved ledger and hypertable', () => {
  const sql = query => query.includes('to_regclass') || query.includes('hypertables') ? 't' : query.includes('_sqlx_migrations') ? '1\n2' : '1';
  const manifest = { source: { migration_ledger: [{ version: 1 }, { version: 2 }], migration_version: 2 } };
  assert.deepEqual(verifyDatabase(manifest, {}, sql).migration_rows, [1,2]);
  assert.throws(() => verifyDatabase(manifest, {}, query => query.includes('count(*) FROM panda.backup_runs') ? '0' : sql(query)), /operational history backup_runs/);
  assert.throws(() => verifyDatabase(manifest, {}, query => query.includes('_sqlx_migrations') ? '1' : sql(query)), /Expected migration ledger/);
});

test('S3 backup requires both storage artifacts; restore polling fails terminal jobs and tolerates restart', async t => {
  const drill = new S3RestoreDrill([], { poll: instantPoll }); t.after(() => rmSync(drill.directory, { recursive: true, force: true }));
  drill.request = async () => ({ recent_runs: [{ id: 'fixture', status: 'succeeded' }], artifacts: [{ run_id: 'fixture', storage_type: 'local' }] });
  await assert.rejects(drill.backup({}, 'token', 'fixture'), /both local and S3/);
  let attempts = 0;
  drill.fetchImpl = async () => { attempts++; if (attempts === 1) throw new Error('restart'); return { ok: true, json: async () => ({ phase: 'completed' }) }; };
  await drill.restored({ origin: 'http://127.0.0.1:1' }, { job: { id: 'fixture' }, capability_token: 'fixture' }); assert.equal(attempts, 2);
  drill.fetchImpl = async () => ({ ok: true, json: async () => ({ phase: 'failed', error_message: 'synthetic' }) });
  await assert.rejects(drill.restored({ origin: 'http://127.0.0.1:1' }, { job: { id: 'fixture' } }), /Remote restore failed: synthetic/);
});

test('restore-lab unsupported checkpoint must fail with the precise migration-chain reason', async t => {
  const dir = directory(t), file = join(dir, 'fixture.rma.tar.gz'); writeFileSync(file, 'synthetic');
  const manifest = { format: 'panda-recovery-v1', source: {} };
  const lab = new RestoreLab(['--package', file], { commands: { invoke: () => JSON.stringify(manifest) }, probe: () => ({ status: 1, stdout: '', stderr: 'unsupported_migration_chain' }) });
  t.after(() => rmSync(lab.directory, { recursive: true, force: true }));
  lab.checkpoint = { id: 'fixture', expected_outcome: 'unsupported_migration_chain' };
  const report = await lab.exercise(); assert.equal(report.status, 'passed');
  lab.probe = () => ({ status: 0, stdout: '', stderr: '' });
  await assert.rejects(lab.exercise(), /must be unsupported/);
  lab.probe = () => ({ status: 1, stdout: '', stderr: 'unrelated failure' });
  await assert.rejects(lab.exercise(), /without the expected/);
});

test('native queue treats a synchronous transport failure as an uncertain single enqueue', async () => {
  const { queueRelease } = await import('./fork-native-release.mjs');
  const sha = 'a'.repeat(40), checks = POLICY.requiredChecks;
  const repository = { id: POLICY.repositoryId, full_name: POLICY.repository, private: false,
    default_branch: POLICY.branch, archived: false, disabled: false };
  const branch = { name: POLICY.branch, protected: true, commit: { sha }, protection: {
    required_status_checks: { enforcement_level: 'everyone', contexts: [...checks], checks: checks.map(context => ({ context, app_id: POLICY.checkAppId })) } } };
  const workflow = { id: POLICY.workflowId, path: POLICY.workflowPath, name: POLICY.workflowName, state: 'active' };
  const run = { id: 101, run_attempt: 1, run_number: 1, repository, head_repository: repository,
    workflow_id: POLICY.workflowId, path: POLICY.workflowPath, name: POLICY.workflowName,
    event: 'push', head_branch: POLICY.branch, head_sha: sha, status: 'completed', conclusion: 'success' };
  const jobs = { total_count: checks.length, jobs: checks.map((name, index) => ({ id: index + 1, name,
    run_id: 101, run_attempt: 1, head_sha: sha, head_branch: POLICY.branch, workflow_name: POLICY.workflowName,
    status: 'completed', conclusion: 'success' })) };
  let writes = 0;
  const fetchImpl = (target, options) => {
    const url = new URL(target);
    if (options.method === 'POST') { writes++; throw new Error('sensitive transport detail'); }
    const data = url.pathname.endsWith('/jobs') ? jobs : url.pathname.endsWith('/runs/101') ? run
      : url.pathname.endsWith('/runs') ? { total_count: 1, workflow_runs: [run] }
      : url.pathname.includes('/branches/') ? branch : url.pathname.includes('/workflows/') ? workflow : repository;
    return Response.json(data);
  };
  await assert.rejects(queueRelease({ token: 'synthetic', webhook: 'https://webhooks.northflank.com/workflows/' + 'b'.repeat(64), sha, runId: 101 },
    { fetchImpl }), /enqueue outcome unknown/);
  assert.equal(writes, 1);
});

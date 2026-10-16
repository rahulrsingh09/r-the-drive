import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checkDeploymentReadiness, evaluateDeploymentReadiness, githubCliFetch, POLICY } from './fork-deploy-readiness.mjs';

const sha = 'a'.repeat(40);
const otherSha = 'b'.repeat(40);
const emptyState = () => ({ deployedShas: [] });

function fixture() {
  const repository = {
    id: POLICY.repositoryId, full_name: POLICY.repository, private: false,
    default_branch: POLICY.branch, archived: false, disabled: false,
  };
  const branch = {
    name: POLICY.branch, protected: true, commit: { sha },
    protection: {
      required_status_checks: {
        enforcement_level: 'everyone',
        contexts: [...POLICY.requiredChecks],
        checks: POLICY.requiredChecks.map((context) => ({ context, app_id: POLICY.checkAppId })),
      },
    },
  };
  const workflow = {
    id: POLICY.workflowId, path: POLICY.workflowPath, name: POLICY.workflowName, state: 'active',
  };
  const run = {
    id: 101, run_number: 20, run_attempt: 1,
    repository, head_repository: structuredClone(repository),
    workflow_id: workflow.id, path: workflow.path, name: workflow.name,
    event: 'push', head_branch: POLICY.branch, head_sha: sha,
    status: 'completed', conclusion: 'success',
  };
  const jobs = {
    total_count: 2,
    jobs: POLICY.requiredChecks.map((name, index) => ({
      id: 201 + index, name, run_id: run.id, run_attempt: run.run_attempt,
      head_sha: sha, head_branch: POLICY.branch, workflow_name: workflow.name,
      status: 'completed', conclusion: 'success',
    })),
  };
  return {
    repository, branch, workflow, run, jobs,
    latestBranch: structuredClone(branch), latestRun: structuredClone(run),
  };
}

test('exact protected default push emits a stable SHA key and no repository-authored text', () => {
  const snapshot = fixture();
  snapshot.run.display_title = 'untrusted text $(do-not-execute)';
  const result = evaluateDeploymentReadiness(snapshot, emptyState());
  assert.deepEqual(result, {
    schemaVersion: 1, ready: true, reason: 'ready',
    items: [{
      id: `${POLICY.repository}:${POLICY.branch}:${sha}`, repository: POLICY.repository,
      branch: POLICY.branch, sha, workflowId: POLICY.workflowId, runId: 101, runAttempt: 1,
    }],
  });
});

const rejectedSnapshots = [
  ['another repository', (s) => { s.repository.full_name = 'attacker/Panda'; }],
  ['repository replaced at the same name', (s) => { s.repository.id++; }],
  ['private repository', (s) => { s.repository.private = true; }],
  ['archived repository', (s) => { s.repository.archived = true; }],
  ['disabled repository', (s) => { s.repository.disabled = true; }],
  ['default branch changed', (s) => { s.repository.default_branch = 'main'; }],
  ['unprotected branch', (s) => { s.branch.protected = false; }],
  ['branch name mismatch', (s) => { s.branch.name = 'main'; }],
  ['protection removed during polling', (s) => { s.latestBranch.protected = false; }],
  ['required contexts removed', (s) => { s.branch.protection.required_status_checks.contexts = []; }],
  ['required checks from another app', (s) => { s.branch.protection.required_status_checks.checks[0].app_id++; }],
  ['admin exemption', (s) => { s.branch.protection.required_status_checks.enforcement_level = 'non_admins'; }],
  ['malformed protection', (s) => { s.branch.protection.required_status_checks.checks = {}; }],
  ['replacement workflow ID', (s) => { s.workflow.id++; }],
  ['workflow path mismatch', (s) => { s.workflow.path = '.github/workflows/fork-upstream-sync.yml'; }],
  ['workflow name mismatch', (s) => { s.workflow.name = 'another workflow'; }],
  ['disabled workflow', (s) => { s.workflow.state = 'disabled_manually'; }],
  ['run from another workflow', (s) => { s.run.workflow_id++; }],
  ['run with another workflow path', (s) => { s.run.path = '.github/workflows/spoof.yml'; }],
  ['run from a foreign head repository', (s) => { s.run.head_repository.full_name = 'attacker/Panda'; }],
  ['candidate branch', (s) => { s.run.head_branch = `review/upstream/main/${sha}`; }],
  ['pull request', (s) => { s.run.event = 'pull_request'; }],
  ['scheduled reusable candidate validation', (s) => { s.run.event = 'schedule'; }],
  ['workflow run event', (s) => { s.run.event = 'workflow_run'; }],
  ['wrong commit', (s) => { s.run.head_sha = otherSha; }],
  ['short SHA', (s) => { s.branch.commit.sha = 'abcdef0'; }],
  ['branch moved during polling', (s) => { s.latestBranch.commit.sha = otherSha; }],
  ['rerun began during polling', (s) => { s.latestRun.run_attempt++; }],
  ['run restarted during polling', (s) => { s.latestRun.status = 'in_progress'; }],
  ['run replaced during polling', (s) => { s.latestRun.id++; }],
  ['missing job', (s) => { s.jobs.jobs.pop(); s.jobs.total_count--; }],
  ['duplicate required job', (s) => { s.jobs.jobs.push(s.jobs.jobs[0]); s.jobs.total_count++; }],
  ['incomplete job pagination', (s) => { s.jobs.total_count++; }],
  ['job from another run', (s) => { s.jobs.jobs[0].run_id++; }],
  ['job from previous attempt', (s) => { s.jobs.jobs[0].run_attempt++; }],
  ['job on wrong SHA', (s) => { s.jobs.jobs[0].head_sha = otherSha; }],
  ['job from a candidate branch', (s) => { s.jobs.jobs[0].head_branch = 'review/candidate'; }],
  ['job not complete', (s) => { s.jobs.jobs[0].status = 'in_progress'; }],
  ['required job renamed', (s) => { s.jobs.jobs[0].name = 'validate / Fork frontend and policy'; }],
];
for (const conclusion of ['failure', 'cancelled', 'skipped', 'neutral', 'timed_out', null]) {
  rejectedSnapshots.push(
    [`run conclusion ${conclusion}`, (s) => { s.run.conclusion = conclusion; }],
    [`job conclusion ${conclusion}`, (s) => { s.jobs.jobs[1].conclusion = conclusion; }],
  );
}
for (const [name, mutate] of rejectedSnapshots) {
  test(`withholds readiness for ${name}`, () => {
    const snapshot = fixture();
    mutate(snapshot);
    const result = evaluateDeploymentReadiness(snapshot, emptyState());
    assert.equal(result.ready, false);
    assert.deepEqual(result.items, []);
  });
}

test('deployment dedup survives reruns and distinct successful runs for the same SHA', () => {
  const snapshot = fixture();
  const state = { deployedShas: [otherSha, sha] };
  assert.equal(evaluateDeploymentReadiness(snapshot, state).reason, 'already-deployed');
  snapshot.run.id = snapshot.latestRun.id = 999;
  snapshot.run.run_attempt = snapshot.latestRun.run_attempt = 3;
  for (const job of snapshot.jobs.jobs) {
    job.run_id = 999;
    job.run_attempt = 3;
  }
  assert.equal(evaluateDeploymentReadiness(snapshot, state).reason, 'already-deployed');
  assert.equal(evaluateDeploymentReadiness(snapshot, emptyState()).ready, true);
  assert.deepEqual(state, { deployedShas: [otherSha, sha] });
});

for (const state of [undefined, {}, { deployedShas: 'all' }, { deployedShas: ['short'] }]) {
  test(`invalid deployment state fails closed: ${JSON.stringify(state)}`, async () => {
    const result = await checkDeploymentReadiness(state, {
      fetchImpl: () => assert.fail('invalid state must not query GitHub'),
    });
    assert.equal(result.reason, 'invalid-state');
  });
}

function fakeApi(snapshot, { runs, onRequest } = {}) {
  const calls = [];
  let branchReads = 0;
  const fetchImpl = async (url, options) => {
    calls.push({ url, options });
    onRequest?.(url, options);
    const parsed = new URL(url);
    const base = `/repos/${POLICY.repository}`;
    const path = parsed.pathname.slice(base.length);
    assert.equal(parsed.origin, 'https://api.github.com');
    assert.ok(parsed.pathname.startsWith(base));
    assert.equal(options.method, 'GET');
    assert.equal(options.redirect, 'error');
    assert.equal(Object.keys(options.headers).some((key) => key.toLowerCase() === 'authorization'), false);
    let body;
    if (path === '') body = snapshot.repository;
    else if (path === `/branches/${encodeURIComponent(POLICY.branch)}`) {
      body = branchReads++ === 0 ? snapshot.branch : snapshot.latestBranch;
    } else if (path === `/actions/workflows/${POLICY.workflowId}`) body = snapshot.workflow;
    else if (path === `/actions/workflows/${POLICY.workflowId}/runs`) {
      assert.equal(parsed.searchParams.has('event'), false);
      assert.equal(parsed.searchParams.get('branch'), POLICY.branch);
      assert.equal(parsed.searchParams.get('head_sha'), sha);
      assert.equal(parsed.searchParams.has('status'), false);
      body = runs ?? { total_count: 1, workflow_runs: [snapshot.run] };
    } else if (path === `/actions/runs/${snapshot.run.id}/attempts/${snapshot.run.run_attempt}/jobs`) {
      assert.equal(parsed.searchParams.get('per_page'), '100');
      body = snapshot.jobs;
    } else if (path === `/actions/runs/${snapshot.run.id}`) body = snapshot.latestRun;
    else assert.fail(`Unexpected API path: ${path}`);
    return new Response(JSON.stringify(body), { status: 200 });
  };
  return { fetchImpl, calls };
}

test('collector uses only fixed public GET endpoints and rereads branch/run after attempt jobs', async () => {
  const mock = fakeApi(fixture());
  const result = await checkDeploymentReadiness(emptyState(), mock);
  assert.equal(result.ready, true);
  assert.equal(mock.calls.length, 7);
  assert.match(mock.calls[4].url, /\/attempts\/1\/jobs/);
  assert.match(mock.calls[5].url, /\/branches\//);
  assert.match(mock.calls[6].url, /\/actions\/runs\/101$/);
});

test('newer failed or queued push blocks an older successful run, regardless of list order', async () => {
  for (const status of ['completed', 'queued']) {
    const snapshot = fixture();
    const newer = { ...snapshot.run, id: 102, run_number: 21, status, conclusion: 'failure' };
    const mock = fakeApi(snapshot, { runs: { total_count: 2, workflow_runs: [snapshot.run, newer] } });
    const result = await checkDeploymentReadiness(emptyState(), mock);
    assert.equal(result.ready, false);
    assert.equal(mock.calls.length, 4);
  }
});

test('missing or truncated runs never authorize a commit', async () => {
  for (const runs of [
    { total_count: 0, workflow_runs: [] },
    { total_count: 2, workflow_runs: [fixture().run] },
    { total_count: 1, workflow_runs: [{ id: '../other', run_number: 20 }] },
  ]) {
    assert.equal((await checkDeploymentReadiness(emptyState(), fakeApi(fixture(), { runs }))).ready, false);
  }
});

test('collector detects branch movement or a rerun after the job response', async () => {
  for (const change of [
    (snapshot) => { snapshot.latestBranch.commit.sha = otherSha; },
    (snapshot) => { snapshot.latestRun.run_attempt++; },
  ]) {
    const snapshot = fixture();
    const mock = fakeApi(snapshot, {
      onRequest: (url) => { if (url.includes('/jobs?')) change(snapshot); },
    });
    assert.equal((await checkDeploymentReadiness(emptyState(), mock)).ready, false);
  }
});

test('HTTP errors, invalid JSON and network failures cannot produce readiness', async () => {
  for (const fetchImpl of [
    async () => new Response('{}', { status: 403 }),
    async () => new Response('invalid JSON', { status: 200 }),
    async () => { throw new Error('network unavailable'); },
  ]) {
    await assert.rejects(checkDeploymentReadiness(emptyState(), { fetchImpl }));
  }
});

test('CLI errors emit no items, exit nonzero and leave state unchanged', (t) => {
  const directory = mkdtempSync(join(tmpdir(), 'fork-readiness-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const state = join(directory, 'state.json');
  writeFileSync(state, '{"deployedShas":["bad"]}');
  for (const args of [[], ['--state', join(directory, 'missing')], ['--state', state]]) {
    const result = spawnSync(process.execPath, ['scripts/fork-deploy-readiness.mjs', ...args], {
      cwd: new URL('..', import.meta.url), encoding: 'utf8',
    });
    assert.equal(result.status, 1);
    assert.deepEqual(JSON.parse(result.stdout).items, []);
  }
  assert.equal(readFileSync(state, 'utf8'), '{"deployedShas":["bad"]}');
});

test('CLI also executes through an installed path alias', (t) => {
  const directory = mkdtempSync(join(tmpdir(), 'fork-readiness-alias-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const alias = join(directory, 'installed.mjs');
  const state = join(directory, 'state.json');
  symlinkSync(new URL('./fork-deploy-readiness.mjs', import.meta.url), alias);
  writeFileSync(state, '{"deployedShas":["bad"]}');
  const result = spawnSync(process.execPath, [alias, '--state', state], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.equal(JSON.parse(result.stdout).reason, 'invalid-state');
});

function cliFixture(t) {
  const directory = realpathSync(mkdtempSync(join(tmpdir(), 'fork-gh-cli-')));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const binary = join(directory, 'gh');
  writeFileSync(binary, 'synthetic executable', { mode: 0o700 });
  return binary;
}

test('authenticated transport preserves every readiness gate and uses only fixed read-only CLI requests', async (t) => {
  const binary = cliFixture(t);
  const mock = fakeApi(fixture());
  const fetchImpl = githubCliFetch(binary, {
    execute: async (file, args, options) => {
      assert.equal(file, binary);
      assert.deepEqual(args.slice(0, 6), ['api', '--hostname', 'github.com', '--method', 'GET', '--header']);
      assert.equal(args.at(-2), '--');
      assert.equal(options.timeout, 20_000);
      assert.equal(options.maxBuffer, 8 * 1024 * 1024);
      assert.deepEqual(Object.keys(options.env).sort(),
        ['GH_PROMPT_DISABLED', 'GIT_TERMINAL_PROMPT', 'HOME', 'LANG', 'PATH']);
      assert.equal(options.shell, undefined);
      const response = await mock.fetchImpl(`https://api.github.com/${args.at(-1)}`, {
        method: 'GET', redirect: 'error', headers: {},
      });
      return { stdout: await response.text() };
    },
  });
  assert.equal((await checkDeploymentReadiness(emptyState(), { fetchImpl })).ready, true);
  assert.equal(mock.calls.length, 7);
  const failed = fixture();
  failed.jobs.jobs[0].conclusion = 'failure';
  const rejected = fakeApi(failed);
  const failedTransport = githubCliFetch(binary, {
    execute: async (_, args) => ({
      stdout: await (await rejected.fetchImpl(`https://api.github.com/${args.at(-1)}`,
        { method: 'GET', redirect: 'error', headers: {} })).text(),
    }),
  });
  assert.equal((await checkDeploymentReadiness(emptyState(), { fetchImpl: failedTransport })).ready, false);
});

test('authenticated transport rejects unsafe binaries, foreign destinations and writes', async (t) => {
  const binary = cliFixture(t);
  const execute = () => assert.fail('must reject before CLI execution');
  assert.throws(() => githubCliFetch('gh', { execute }));
  symlinkSync(binary, `${binary}-link`);
  assert.throws(() => githubCliFetch(`${binary}-link`, { execute }));
  chmodSync(binary, 0o777);
  assert.throws(() => githubCliFetch(binary, { execute }));
  chmodSync(binary, 0o700);
  const fetchImpl = githubCliFetch(binary, { execute });
  for (const url of [
    'https://evil.example/repos/your-org/r-the-drive',
    'https://api.github.com/repos/attacker/Panda',
    'https://api.github.com/repos/your-org/r-the-drive-other',
    'https://user:password@api.github.com/repos/your-org/r-the-drive',
    'https://api.github.com/repos/your-org/r-the-drive#fragment',
  ]) await assert.rejects(fetchImpl(url, { method: 'GET', redirect: 'error' }));
  const url = `https://api.github.com/repos/${POLICY.repository}`;
  await assert.rejects(fetchImpl(url, { method: 'POST', redirect: 'error' }));
  await assert.rejects(fetchImpl(url, { method: 'GET', redirect: 'follow' }));
});

test('GitHub CLI authentication errors and malformed output remain redacted with no public fallback', async (t) => {
  const binary = cliFixture(t);
  for (const execute of [
    async () => { throw new Error('synthetic-secret-in-cli-stderr'); },
    async () => ({ stdout: 'synthetic-secret-invalid-json' }),
  ]) {
    const fetchImpl = githubCliFetch(binary, { execute });
    await assert.rejects(fetchImpl(`https://api.github.com/repos/${POLICY.repository}`,
      { method: 'GET', redirect: 'error' }), { message: 'Authenticated GitHub metadata unavailable' });
  }
});


test('genuine explicit CI dispatch on the protected commit satisfies the same checks', async () => {
  const snapshot = fixture();
  snapshot.run.event = snapshot.latestRun.event = 'workflow_dispatch';
  assert.equal(evaluateDeploymentReadiness(snapshot, emptyState()).ready, true);
  assert.equal((await checkDeploymentReadiness(emptyState(), fakeApi(snapshot))).ready, true);
  snapshot.jobs.jobs[0].head_sha = otherSha;
  assert.equal(evaluateDeploymentReadiness(snapshot, emptyState()).ready, false);
});

import assert from 'node:assert/strict';
import test from 'node:test';
import { advanceUpstream, checkCandidateJobs, githubApi } from './fork-auto-promote.mjs';
import { POLICY } from './fork-deploy-readiness.mjs';

const base = 'a'.repeat(40);
const candidate = 'b'.repeat(40);
const upstream = 'c'.repeat(40);
const branch = `review/upstream/main/${candidate}`;
const input = { base, candidate, source: 'main', status: 'candidate-ready' };

function fixture() {
  const repo = {
    id: POLICY.repositoryId,
    full_name: POLICY.repository,
    private: false,
    archived: false,
    disabled: false,
    default_branch: POLICY.branch,
  };
  const protectedBranch = {
    name: POLICY.branch,
    protected: true,
    commit: { sha: base },
    protection: {
      required_status_checks: {
        enforcement_level: 'everyone',
        contexts: [...POLICY.requiredChecks],
        checks: POLICY.requiredChecks.map((context) => ({ context, app_id: POLICY.checkAppId })),
      },
    },
  };
  const workflow = {
    id: POLICY.workflowId,
    path: POLICY.workflowPath,
    name: POLICY.workflowName,
    state: 'active',
  };
  const run = {
    id: 101,
    run_attempt: 1,
    run_number: 10,
    repository: repo,
    head_repository: repo,
    event: 'push',
    head_sha: candidate,
    head_branch: branch,
    workflow_id: workflow.id,
    path: workflow.path,
    name: workflow.name,
    status: 'completed',
    conclusion: 'success',
  };
  const jobs = {
    total_count: 2,
    jobs: POLICY.requiredChecks.map((name, index) => ({
      id: index + 201,
      name,
      run_id: run.id,
      run_attempt: run.run_attempt,
      head_sha: candidate,
      head_branch: branch,
      workflow_name: workflow.name,
      status: 'completed',
      conclusion: 'success',
    })),
  };
  const prepared = {
    source: 'main',
    status: 'candidate-ready',
    safeToValidate: true,
    base,
    candidate,
    branch,
    upstream,
    changedControls: [],
    conflicts: [],
  };
  const f = {
    repo,
    protectedBranch,
    workflow,
    run,
    jobs,
    prepared,
    writes: [],
    candidateRuns: [run],
    defaultRuns: [],
    missingRef: false,
    prepareCount: 0,
    mutateBeforeFinalPrepare: null,
    mutateOnFreshRun: null,
    onRequest: null,
  };
  let elapsed = 0;
  f.api = async (path, body, method = body ? 'POST' : 'GET') => {
    f.onRequest?.(path, body, method);
    if (method !== 'GET') {
      f.writes.push({ path, body, method });
      if (method === 'PATCH') {
        assert.equal(path, `/git/refs/heads/${POLICY.branch}`);
        assert.deepEqual(body, { sha: candidate, force: false });
        protectedBranch.commit.sha = candidate;
      } else {
        assert.equal(path, `/actions/workflows/${POLICY.workflowId}/dispatches`);
        assert.equal(body.ref, POLICY.branch);
      }
      return null;
    }
    if (path === '') return structuredClone(repo);
    if (path === `/branches/${encodeURIComponent(POLICY.branch)}`)
      return structuredClone(protectedBranch);
    if (path === `/actions/workflows/${POLICY.workflowId}`) return structuredClone(workflow);
    if (path === `/git/ref/heads/${branch}`)
      return {
        ref: `refs/heads/${branch}`,
        object: { type: 'commit', sha: f.missingRef ? upstream : candidate },
      };
    if (path.startsWith(`/actions/workflows/${POLICY.workflowId}/runs?`)) {
      const query = new URL(`https://example.invalid${path}`).searchParams;
      assert.equal(query.get('per_page'), '100');
      const runs = query.get('branch') === branch ? f.candidateRuns : f.defaultRuns;
      return structuredClone({ total_count: runs.length, workflow_runs: runs });
    }
    if (path === `/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`) {
      return structuredClone(jobs);
    }
    if (path === `/actions/runs/${run.id}`) {
      f.mutateOnFreshRun?.();
      return structuredClone(run);
    }
    assert.fail(`Unexpected request ${path}`);
  };
  f.options = {
    api: f.api,
    prepare: async (expectedBase) => {
      assert.equal(expectedBase, base);
      if (++f.prepareCount === 2) f.mutateBeforeFinalPrepare?.();
      return structuredClone(prepared);
    },
    now: () => elapsed,
    sleep: async (ms) => {
      elapsed += ms;
    },
  };
  return f;
}

test('only a recomputed stable candidate with exact successful CI is fast-forwarded', async () => {
  const f = fixture();
  const result = await advanceUpstream(input, f.options);
  assert.equal(result.promoted, true);
  assert.equal(result.sha, candidate);
  assert.equal(f.prepareCount, 2);
  assert.deepEqual(f.writes, [
    {
      path: `/git/refs/heads/${POLICY.branch}`,
      body: { sha: candidate, force: false },
      method: 'PATCH',
    },
    {
      path: `/actions/workflows/${POLICY.workflowId}/dispatches`,
      body: { ref: POLICY.branch },
      method: 'POST',
    },
  ]);
});

test('candidate push CI may appear after publishing without any candidate dispatch', async () => {
  const f = fixture();
  f.candidateRuns = [];
  f.options.sleep = async () => { f.candidateRuns = [f.run]; };
  assert.equal((await advanceUpstream(input, f.options)).promoted, true);
  assert.equal(f.writes[0].method, 'PATCH');
  assert.ok(f.writes.every((w) => w.method !== 'POST' || w.body.ref === POLICY.branch));
});

const blocked = [
  [
    'manual candidate CI does not satisfy protected promotion',
    (f) => { f.run.event = 'workflow_dispatch'; },
  ],
  [
    'private repository',
    (f) => {
      f.repo.private = true;
    },
  ],
  [
    'changed repository ID',
    (f) => {
      f.repo.id++;
    },
  ],
  [
    'renamed default branch',
    (f) => {
      f.repo.default_branch = 'main';
    },
  ],
  [
    'removed protection',
    (f) => {
      f.protectedBranch.protected = false;
    },
  ],
  [
    'removed required checks',
    (f) => {
      f.protectedBranch.protection.required_status_checks.contexts = [];
    },
  ],
  [
    'different check app',
    (f) => {
      f.protectedBranch.protection.required_status_checks.checks[0].app_id++;
    },
  ],
  [
    'base changed',
    (f) => {
      f.protectedBranch.commit.sha = upstream;
    },
  ],
  [
    'workflow replaced',
    (f) => {
      f.workflow.id++;
    },
  ],
  [
    'disabled workflow',
    (f) => {
      f.workflow.state = 'disabled_manually';
    },
  ],
  [
    'security control changed',
    (f) => {
      f.prepared.changedControls = ['apps/api/src/private_deployment/keys.rs'];
    },
  ],
  [
    'migration changed',
    (f) => {
      f.prepared.changedControls = ['apps/api/migrations/0031.sql'];
    },
  ],
  [
    'merge conflict',
    (f) => {
      f.prepared.conflicts = ['README.md'];
    },
  ],
  [
    'non-deterministic candidate',
    (f) => {
      f.prepared.candidate = upstream;
    },
  ],
  [
    'moved candidate ref',
    (f) => {
      f.missingRef = true;
    },
  ],
  [
    'unreviewed dev source',
    (f) => {
      f.prepared.source = 'dev';
    },
  ],
  [
    'failed CI',
    (f) => {
      f.run.conclusion = 'failure';
    },
  ],
  [
    'skipped CI',
    (f) => {
      f.run.conclusion = 'skipped';
    },
  ],
  [
    'CI from another repository',
    (f) => {
      f.run.head_repository = { ...f.repo, id: 999 };
    },
  ],
  [
    'wrong CI SHA',
    (f) => {
      f.run.head_sha = upstream;
    },
  ],
  [
    'PR event',
    (f) => {
      f.run.event = 'pull_request';
    },
  ],
  [
    'scheduled reusable event',
    (f) => {
      f.run.event = 'schedule';
    },
  ],
  [
    'wrong CI branch',
    (f) => {
      f.run.head_branch = POLICY.branch;
    },
  ],
  [
    'duplicate job name',
    (f) => {
      f.jobs.jobs[1].name = f.jobs.jobs[0].name;
    },
  ],
  [
    'missing job page',
    (f) => {
      f.jobs.total_count++;
    },
  ],
  [
    'previous attempt job',
    (f) => {
      f.jobs.jobs[0].run_attempt++;
    },
  ],
  [
    'job from another SHA',
    (f) => {
      f.jobs.jobs[0].head_sha = upstream;
    },
  ],
  [
    'job from protected branch',
    (f) => {
      f.jobs.jobs[0].head_branch = POLICY.branch;
    },
  ],
  [
    'neutral job',
    (f) => {
      f.jobs.jobs[0].conclusion = 'neutral';
    },
  ],
];
for (const [name, mutate] of blocked) {
  test(`no promotion or dispatch for ${name}`, async () => {
    const f = fixture();
    mutate(f);
    await assert.rejects(advanceUpstream(input, f.options));
    assert.deepEqual(f.writes, []);
  });
}

test('changed base, upstream or run attempt during validation cannot be promoted', async () => {
  for (const mutate of [
    (f) => {
      f.protectedBranch.commit.sha = upstream;
    },
    (f) => {
      f.prepared.upstream = '';
    },
    (f) => {
      f.prepared.candidate = upstream;
    },
  ]) {
    const f = fixture();
    f.mutateBeforeFinalPrepare = () => mutate(f);
    await assert.rejects(advanceUpstream(input, f.options));
    assert.deepEqual(f.writes, []);
  }
  const f = fixture();
  f.mutateOnFreshRun = () => {
    f.run.run_attempt++;
  };
  await assert.rejects(advanceUpstream(input, f.options));
  assert.deepEqual(f.writes, []);
});

test('a newer failing CI run cannot fall back to an older success', async () => {
  const f = fixture();
  f.candidateRuns.push({ ...f.run, id: 102, run_number: 11, conclusion: 'failure' });
  await assert.rejects(advanceUpstream(input, f.options));
  assert.deepEqual(f.writes, []);
});

test('waiting for candidate CI is bounded and never promotes a pending result', async () => {
  const f = fixture();
  f.run.status = 'queued';
  await assert.rejects(advanceUpstream(input, f.options), /candidate-ci-timeout/);
  assert.deepEqual(f.writes, []);
});

test('missing push CI stops without manufacturing or dispatching candidate checks', async () => {
  const f = fixture();
  f.candidateRuns = [];
  await assert.rejects(advanceUpstream(input, f.options), /candidate-ci-timeout/);
  assert.deepEqual(f.writes, []);
});

test('current upstream does not rerun existing protected CI or deploy anything', async () => {
  const f = fixture();
  f.prepared = { status: 'current', base, source: 'main' };
  f.options.prepare = async () => f.prepared;
  f.defaultRuns = [{ ...f.run, head_sha: base, head_branch: POLICY.branch, event: 'push' }];
  const result = await advanceUpstream({ base, source: 'main', status: 'current' }, f.options);
  assert.equal(result.promoted, false);
  assert.equal(result.defaultCi.dispatched, false);
  assert.deepEqual(f.writes, []);
});

test('current-state retry resumes missing protected CI after an interrupted promotion', async () => {
  const f = fixture();
  f.options.prepare = async () => ({ status: 'current', base, source: 'main' });
  const result = await advanceUpstream({ base, source: 'main', status: 'current' }, f.options);
  assert.equal(result.promoted, false);
  assert.equal(result.defaultCi.dispatched, true);
  assert.deepEqual(f.writes, [
    {
      path: `/actions/workflows/${POLICY.workflowId}/dispatches`,
      body: { ref: POLICY.branch },
      method: 'POST',
    },
  ]);
});

test('candidate CI identity validator refuses reused check names from other workflows', () => {
  const f = fixture();
  f.run.workflow_id++;
  assert.throws(() => checkCandidateJobs(f.run, f.jobs, candidate));
});

test('API token only reaches fixed GitHub repository requests and approved writes', async () => {
  const calls = [];
  const api = githubApi('synthetic-token', {
    fetchImpl: async (url, options) => {
      calls.push({ url, options });
      assert.ok(url.startsWith(`https://api.github.com/repos/${POLICY.repository}/`));
      assert.equal(options.headers.Authorization, 'Bearer synthetic-token');
      assert.equal(options.redirect, 'error');
      return new Response(null, { status: 204 });
    },
  });
  await api(`/actions/workflows/${POLICY.workflowId}/dispatches`, { ref: POLICY.branch });
  await api(`/git/refs/heads/${POLICY.branch}`, { sha: candidate, force: false }, 'PATCH');
  assert.equal(calls.length, 2);
  for (const [path, body, method] of [
    ['https://attacker.invalid', undefined, 'GET'],
    ['/../other/repo', undefined, 'GET'],
    ['/contents/runtime', { data: 'unsafe' }, 'PUT'],
    [`/git/refs/heads/${POLICY.branch}`, { sha: candidate, force: true }, 'PATCH'],
    [`/actions/workflows/${POLICY.workflowId}/dispatches`, { ref: 'dev' }, 'POST'],
    [`/actions/workflows/${POLICY.workflowId}/dispatches`, { ref: branch }, 'POST'],
    [`/actions/workflows/${POLICY.workflowId}/dispatches`, { ref: branch, inputs: {} }, 'POST'],
  ])
    await assert.rejects(api(path, body, method));
  assert.equal(calls.length, 2);
});

test('HTTP and transport failures redact response bodies and credential-bearing errors', async () => {
  for (const fetchImpl of [
    async () => new Response('synthetic-secret', { status: 403 }),
    async () => {
      throw new Error('synthetic-secret');
    },
    async () => new Response('synthetic-secret', { status: 200 }),
  ]) {
    await assert.rejects(
      githubApi('synthetic-token', { fetchImpl })('/actions/runs/101'),
      (error) => error.message === 'github-request-unavailable-or-outcome-unknown'
    );
  }
});

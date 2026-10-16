import { appendFileSync, realpathSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { POLICY, trustedBranch, trustedRun } from './fork-deploy-readiness.mjs';
import { syncFork } from './fork-sync.mjs';

const shaPattern = /^[0-9a-f]{40}$/;
const candidateBranch = (sha) => `review/upstream/main/${sha}`;
const branchPath = `/branches/${encodeURIComponent(POLICY.branch)}`;
const workflowPath = `/actions/workflows/${POLICY.workflowId}`;
const require = (condition, reason) => {
  if (!condition) throw new Error(reason);
};

export function githubApi(token, { fetchImpl = fetch } = {}) {
  require(typeof token === 'string' && token.length > 0, 'missing-github-token');
  return async (path, body, method = body ? 'POST' : 'GET') => {
    require(typeof path === 'string' &&
      (path === '' || path.startsWith('/')) &&
      !path.includes('..') &&
      !path.includes('#') &&
      !path.includes('\\'), 'untrusted-api-path');
    if (method !== 'GET') {
      const dispatch =
        method === 'POST' &&
        path === `${workflowPath}/dispatches` &&
        body &&
        Object.keys(body).length === 1 &&
        body.ref === POLICY.branch;
      const promote =
        method === 'PATCH' &&
        path === `/git/refs/heads/${POLICY.branch}` &&
        body &&
        Object.keys(body).length === 2 &&
        body.force === false &&
        shaPattern.test(body.sha);
      require(dispatch || promote, 'untrusted-api-write');
    } else require(body === undefined, 'unexpected-get-body');
    try {
      const response = await fetchImpl(`https://api.github.com/repos/${POLICY.repository}${path}`, {
        method,
        redirect: 'error',
        signal: AbortSignal.timeout(20_000),
        headers: {
          Accept: 'application/vnd.github+json',
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
          'X-GitHub-Api-Version': '2022-11-28',
          'User-Agent': 'panda-upstream-promotion',
          'Cache-Control': 'no-cache',
        },
        ...(body ? { body: JSON.stringify(body) } : {}),
      });
      require(response.ok, 'github-request-failed');
      const text = await response.text();
      require(text.length <= 2 * 1024 * 1024, 'github-response-too-large');
      return text ? JSON.parse(text) : null;
    } catch {
      throw new Error('github-request-unavailable-or-outcome-unknown');
    }
  };
}

async function context(api, expected) {
  const [repo, branch, workflow] = await Promise.all([api(''), api(branchPath), api(workflowPath)]);
  require(repo?.id === POLICY.repositoryId &&
    repo.full_name === POLICY.repository &&
    repo.private === false &&
    repo.default_branch === POLICY.branch &&
    repo.archived === false &&
    repo.disabled === false, 'repository-mismatch');
  require(trustedBranch(branch) && branch.commit.sha === expected, 'protected-base-moved');
  require(workflow?.id === POLICY.workflowId &&
    workflow.path === POLICY.workflowPath &&
    workflow.name === POLICY.workflowName &&
    workflow.state === 'active', 'workflow-mismatch');
  return branch;
}

function eligible(result, base, candidate) {
  require(result?.source === 'main' &&
    result.status === 'candidate-ready' &&
    result.safeToValidate === true &&
    result.base === base &&
    result.candidate === candidate &&
    result.branch === candidateBranch(candidate) &&
    shaPattern.test(result.upstream) &&
    Array.isArray(result.changedControls) &&
    result.changedControls.length === 0 &&
    Array.isArray(result.conflicts) &&
    result.conflicts.length === 0, 'candidate-requires-review-or-changed');
}

async function latestRun(api, branch, sha) {
  const query = new URLSearchParams({ branch, head_sha: sha, per_page: '100' });
  const data = await api(`${workflowPath}/runs?${query}`);
  require(Array.isArray(data?.workflow_runs) &&
    data.total_count === data.workflow_runs.length &&
    data.workflow_runs.every(
      (r) =>
        Number.isSafeInteger(r.id) &&
        r.id > 0 &&
        Number.isSafeInteger(r.run_number) &&
        r.run_number > 0
    ), 'incomplete-run-list');
  return data.workflow_runs.toSorted((a, b) => b.run_number - a.run_number)[0];
}

async function candidateRef(api, candidate) {
  const branch = candidateBranch(candidate);
  const ref = await api(`/git/ref/heads/${branch}`);
  require(ref?.ref === `refs/heads/${branch}` &&
    ref.object?.sha === candidate &&
    ref.object.type === 'commit', 'candidate-ref-moved');
}

export function checkCandidateJobs(run, jobs, candidate) {
  const branch = candidateBranch(candidate);
  require(trustedRun(run, candidate, branch) &&
    run.event === 'push', 'candidate-ci-not-successful');
  require(Array.isArray(jobs?.jobs) && jobs.total_count === jobs.jobs.length, 'incomplete-jobs');
  for (const name of POLICY.requiredChecks) {
    const matches = jobs.jobs.filter((job) => job?.name === name);
    require(matches.length === 1, 'required-job-missing-or-ambiguous');
    const job = matches[0];
    require(Number.isSafeInteger(job.id) &&
      job.id > 0 &&
      job.run_id === run.id &&
      job.run_attempt === run.run_attempt &&
      job.head_sha === candidate &&
      job.head_branch === branch &&
      job.workflow_name === POLICY.workflowName &&
      job.status === 'completed' &&
      job.conclusion === 'success', 'required-job-not-successful');
  }
}

async function ensureDefaultCi(api, sha) {
  await context(api, sha);
  const run = await latestRun(api, POLICY.branch, sha);
  if (run) {
    require(run.workflow_id === POLICY.workflowId &&
      run.path === POLICY.workflowPath &&
      run.head_sha === sha &&
      run.head_branch === POLICY.branch &&
      ['push', 'workflow_dispatch'].includes(run.event), 'default-ci-mismatch');
    require(run.status !== 'completed' || run.conclusion === 'success', 'default-ci-failed');
    return { dispatched: false, existingRunId: run.id };
  }
  await context(api, sha);
  await api(`${workflowPath}/dispatches`, { ref: POLICY.branch });
  return { dispatched: true };
}

export async function advanceUpstream(
  { base, candidate = '', source = 'main', status },
  {
    api,
    prepare = (expectedBase) => syncFork({ expectedBase, source: 'main' }),
    now = Date.now,
    sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  }
) {
  require(shaPattern.test(base) &&
    source === 'main' &&
    ['current', 'candidate-ready'].includes(status), 'upstream-review-required');
  await context(api, base);
  if (status === 'current') {
    require(candidate === '', 'unexpected-candidate');
    const result = await prepare(base);
    require(result.status === 'current' &&
      result.base === base &&
      result.source === 'main', 'upstream-changed');
    return { status: 'current', promoted: false, defaultCi: await ensureDefaultCi(api, base) };
  }
  require(shaPattern.test(candidate) && candidate !== base, 'invalid-candidate');
  eligible(await prepare(base), base, candidate);
  await candidateRef(api, candidate);
  const branch = candidateBranch(candidate);
  let run = await latestRun(api, branch, candidate);
  const deadline = now() + 55 * 60_000;
  while (!run || run.status !== 'completed') {
    require(now() < deadline, 'candidate-ci-timeout');
    await sleep(15_000);
    await context(api, base);
    run = await latestRun(api, branch, candidate);
  }
  require(trustedRun(run, candidate, branch) &&
    run.event === 'push', 'candidate-ci-not-successful');
  const jobs = await api(`/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`);
  checkCandidateJobs(run, jobs, candidate);
  eligible(await prepare(base), base, candidate);
  await candidateRef(api, candidate);
  const fresh = await api(`/actions/runs/${run.id}`);
  const latest = await latestRun(api, branch, candidate);
  require(trustedRun(fresh, candidate, branch) &&
    fresh.run_attempt === run.run_attempt &&
    fresh.id === run.id &&
    latest?.id === run.id &&
    latest.run_attempt === run.run_attempt &&
    latest.status === 'completed' &&
    latest.conclusion === 'success', 'candidate-ci-changed');
  await context(api, base);
  await api(`/git/refs/heads/${POLICY.branch}`, { sha: candidate, force: false }, 'PATCH');
  await context(api, candidate);
  return {
    status: 'promoted',
    promoted: true,
    sha: candidate,
    candidateRunId: run.id,
    defaultCi: await ensureDefaultCi(api, candidate),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  try {
    require(process.env.GITHUB_REPOSITORY === POLICY.repository &&
      process.env.GITHUB_REF === `refs/heads/${POLICY.branch}` &&
      process.env.GITHUB_WORKFLOW_REF ===
        `${POLICY.repository}/.github/workflows/fork-upstream-sync.yml@refs/heads/${POLICY.branch}` &&
      ['schedule', 'workflow_dispatch'].includes(process.env.GITHUB_EVENT_NAME) &&
      process.env.GITHUB_SHA === process.env.FORK_SYNC_BASE, 'untrusted-workflow-context');
    const token = process.env.GITHUB_TOKEN;
    delete process.env.GITHUB_TOKEN;
    const result = await advanceUpstream(
      {
        base: process.env.FORK_SYNC_BASE,
        candidate: process.env.FORK_SYNC_CANDIDATE || '',
        source: process.env.FORK_SYNC_SOURCE || 'main',
        status: process.env.FORK_SYNC_STATUS,
      },
      { api: githubApi(token) }
    );
    console.log(JSON.stringify(result));
    if (process.env.GITHUB_STEP_SUMMARY)
      appendFileSync(
        process.env.GITHUB_STEP_SUMMARY,
        `## Upstream automation\n\n\`\`\`json\n${JSON.stringify(result, null, 2)}\n\`\`\`\n\n` +
          'A promoted commit still requires protected-branch CI and the Northflank backup, deployment ' +
          'and verification workflow. No vehicle data or credentials are changed by this job.\n'
      );
  } catch (error) {
    const reason =
      typeof error?.message === 'string' && /^[a-z][a-z-]{1,100}$/.test(error.message)
        ? error.message
        : 'metadata-or-workflow-error';
    console.error(
      `Upstream automation stopped: ${reason}. Inspect sync metadata, CI and the protected branch before retrying.`
    );
    if (process.env.GITHUB_STEP_SUMMARY)
      appendFileSync(
        process.env.GITHUB_STEP_SUMMARY,
        `## Upstream automation stopped\n\nReason: \`${reason}\`.\n\nReview the sync report and candidate CI. If promotion ` +
          'already succeeded but dispatch did not, the next daily current-state check can resume CI. ' +
          'Do not force a merge or bypass required checks.\n'
      );
    process.exitCode = 1;
  }
}

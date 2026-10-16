import { execFile } from 'node:child_process';
import { readFileSync, realpathSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, isAbsolute } from 'node:path';
import { promisify } from 'node:util';
import { pathToFileURL } from 'node:url';

export const POLICY = Object.freeze({
  repository: 'your-org/r-the-drive',
  repositoryId: 0,
  branch: 'mainline',
  workflowId: 0,
  workflowPath: '.github/workflows/fork-ci.yml',
  workflowName: 'Fork validation',
  checkAppId: 15368,
  requiredChecks: Object.freeze([
    'Fork frontend and policy',
    'Fork backend and security regressions',
  ]),
});

const isSha = (value) => typeof value === 'string' && /^[0-9a-f]{40}$/.test(value);
const isId = (value) => Number.isSafeInteger(value) && value > 0;
const withheld = (reason) => ({ schemaVersion: 1, ready: false, reason, items: [] });
const validState = (state) =>
  Array.isArray(state?.deployedShas) && state.deployedShas.every(isSha);
const trustedRepository = (repository) =>
  repository?.id === POLICY.repositoryId &&
  repository.full_name === POLICY.repository &&
  repository.private === false;

export function githubCliFetch(binary, { execute = promisify(execFile) } = {}) {
  if (!isAbsolute(binary) || realpathSync(binary) !== binary) throw new Error('Untrusted GitHub CLI');
  const info = statSync(binary);
  if (!info.isFile() || info.uid !== process.getuid() || (info.mode & 0o022)) {
    throw new Error('Untrusted GitHub CLI');
  }
  return async (url, options) => {
    const parsed = new URL(url);
    const base = `/repos/${POLICY.repository}`;
    if (parsed.origin !== 'https://api.github.com' || parsed.username || parsed.password ||
        parsed.hash || (parsed.pathname !== base && !parsed.pathname.startsWith(`${base}/`)) ||
        options.method !== 'GET' || options.redirect !== 'error') {
      throw new Error('Untrusted metadata request');
    }
    try {
      const { stdout } = await execute(binary, [
        'api', '--hostname', 'github.com', '--method', 'GET',
        '--header', 'Accept: application/vnd.github+json',
        '--header', 'X-GitHub-Api-Version: 2022-11-28',
        '--', parsed.pathname.slice(1) + parsed.search,
      ], {
        encoding: 'utf8', timeout: 20_000, maxBuffer: 8 * 1024 * 1024,
        env: {
          HOME: homedir(), PATH: `${dirname(binary)}:/usr/bin:/bin`, LANG: 'C.UTF-8',
          GH_PROMPT_DISABLED: '1', GIT_TERMINAL_PROMPT: '0',
        },
      });
      JSON.parse(stdout);
      return new Response(stdout, { status: 200 });
    } catch {
      throw new Error('Authenticated GitHub metadata unavailable');
    }
  };
}

export function trustedBranch(branch) {
  const protection = branch?.protection?.required_status_checks;
  return branch?.name === POLICY.branch &&
    branch.protected === true &&
    isSha(branch.commit?.sha) &&
    protection?.enforcement_level === 'everyone' &&
    Array.isArray(protection.contexts) && Array.isArray(protection.checks) &&
    POLICY.requiredChecks.every((name) =>
      protection.contexts.includes(name) &&
      protection.checks.some((check) =>
        check?.context === name && check.app_id === POLICY.checkAppId));
}

export function trustedRun(run, sha, branch = POLICY.branch) {
  return isId(run?.id) && isId(run.run_attempt) &&
    trustedRepository(run.repository) && trustedRepository(run.head_repository) &&
    run.workflow_id === POLICY.workflowId && run.path === POLICY.workflowPath &&
    run.name === POLICY.workflowName && ['push', 'workflow_dispatch'].includes(run.event) &&
    run.head_branch === branch && run.head_sha === sha &&
    run.status === 'completed' && run.conclusion === 'success';
}

export function evaluateDeploymentReadiness(snapshot, state) {
  if (!validState(state)) return withheld('invalid-state');
  const { repository, branch, workflow, run, jobs, latestBranch, latestRun } = snapshot ?? {};
  if (!trustedRepository(repository) || repository.default_branch !== POLICY.branch ||
      repository.archived !== false || repository.disabled !== false) {
    return withheld('untrusted-repository');
  }
  if (!trustedBranch(branch) || !trustedBranch(latestBranch)) {
    return withheld('branch-protection-mismatch');
  }
  if (workflow?.id !== POLICY.workflowId || workflow.path !== POLICY.workflowPath ||
      workflow.name !== POLICY.workflowName || workflow.state !== 'active') {
    return withheld('untrusted-workflow');
  }
  const sha = branch.commit.sha;
  if (latestBranch.commit.sha !== sha) return withheld('default-branch-moved');
  if (!trustedRun(run, sha)) return withheld('ci-run-not-successful');
  if (!trustedRun(latestRun, sha) || latestRun.id !== run.id ||
      latestRun.run_attempt !== run.run_attempt) {
    return withheld('run-changed');
  }
  if (!Array.isArray(jobs?.jobs) || jobs.total_count !== jobs.jobs.length) {
    return withheld('incomplete-jobs');
  }
  for (const name of POLICY.requiredChecks) {
    const matches = jobs.jobs.filter((job) => job?.name === name);
    if (matches.length !== 1) return withheld('required-check-missing-or-ambiguous');
    const job = matches[0];
    if (!isId(job.id) || job.run_id !== run.id || job.run_attempt !== run.run_attempt ||
        job.head_sha !== sha || job.head_branch !== POLICY.branch ||
        job.workflow_name !== POLICY.workflowName ||
        job.status !== 'completed' || job.conclusion !== 'success') {
      return withheld('required-check-not-successful');
    }
  }
  if (state.deployedShas.includes(sha)) return withheld('already-deployed');
  return {
    schemaVersion: 1,
    ready: true,
    reason: 'ready',
    items: [{
      id: `${POLICY.repository}:${POLICY.branch}:${sha}`,
      repository: POLICY.repository,
      branch: POLICY.branch,
      sha,
      workflowId: POLICY.workflowId,
      runId: run.id,
      runAttempt: run.run_attempt,
    }],
  };
}

export async function checkDeploymentReadiness(state, { fetchImpl = fetch } = {}) {
  if (!validState(state)) return withheld('invalid-state');
  const api = async (path) => {
    const response = await fetchImpl(
      `https://api.github.com/repos/${POLICY.repository}${path}`,
      {
        method: 'GET',
        redirect: 'error',
        signal: AbortSignal.timeout(15_000),
        headers: {
          Accept: 'application/vnd.github+json',
          'X-GitHub-Api-Version': '2022-11-28',
          'User-Agent': 'panda-deployment-readiness',
          'Cache-Control': 'no-cache',
        },
      },
    );
    if (!response.ok) throw new Error(`GitHub metadata request failed: HTTP ${response.status}`);
    return response.json();
  };
  const branchPath = `/branches/${encodeURIComponent(POLICY.branch)}`;
  const [repository, branch, workflow] = await Promise.all([
    api(''), api(branchPath), api(`/actions/workflows/${POLICY.workflowId}`),
  ]);
  if (!isSha(branch?.commit?.sha)) return withheld('invalid-default-sha');
  const query = new URLSearchParams({
    branch: POLICY.branch, head_sha: branch.commit.sha, per_page: '100',
  });
  const runs = await api(`/actions/workflows/${POLICY.workflowId}/runs?${query}`);
  if (!Array.isArray(runs?.workflow_runs) ||
      runs.total_count !== runs.workflow_runs.length ||
      runs.workflow_runs.some((run) => !isId(run?.run_number) || !isId(run.id))) {
    return withheld('incomplete-runs');
  }
  // A newer failed or pending run must never fall back to an older success.
  const run = runs.workflow_runs.toSorted((a, b) => b.run_number - a.run_number)[0];
  if (!run) return withheld('no-current-ci-run');
  if (!trustedRun(run, branch.commit.sha)) return withheld('ci-run-not-successful');
  const jobs = await api(`/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`);
  const [latestBranch, latestRun] = await Promise.all([
    api(branchPath), api(`/actions/runs/${run.id}`),
  ]);
  return evaluateDeploymentReadiness(
    { repository, branch, workflow, run, jobs, latestBranch, latestRun }, state,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  try {
    if (![4, 6].includes(process.argv.length) || process.argv[2] !== '--state' ||
        (process.argv.length === 6 && process.argv[4] !== '--github-cli')) {
      throw new Error('Invalid readiness arguments');
    }
    const state = JSON.parse(readFileSync(process.argv[3], 'utf8'));
    const transport = process.argv.length === 6
      ? { fetchImpl: githubCliFetch(process.argv[5]) } : {};
    const result = await checkDeploymentReadiness(state, transport);
    console.log(JSON.stringify(result));
    if (result.reason === 'invalid-state') process.exitCode = 1;
  } catch {
    console.log(JSON.stringify(withheld('metadata-or-input-error')));
    console.error('Readiness unavailable; check state/arguments and GitHub availability or rate limits.');
    process.exitCode = 1;
  }
}

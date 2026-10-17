import { execFile } from 'node:child_process';
import { readFileSync, realpathSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, isAbsolute } from 'node:path';
import { promisify } from 'node:util';
import { pathToFileURL } from 'node:url';
function fieldsMatch(value, expected) {
  if (value == null) return false;
  for (const key of Object.keys(expected)) if (value[key] !== expected[key]) return false;
  return true;
}

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

const isSha = value => typeof value === 'string' && /^[0-9a-f]{40}$/.test(value);
const isId = value => Number.isSafeInteger(value) && value > 0;
const withheld = reason => ({ schemaVersion: 1, ready: false, reason, items: [] });
function validState(state) {
  if (!Array.isArray(state?.deployedShas)) return false;
  return state.deployedShas.every(isSha);
}
const trustedRepository = repo => fieldsMatch(repo, {
  id: POLICY.repositoryId, full_name: POLICY.repository, private: false,
});

export function githubCliFetch(binary, { execute = promisify(execFile) } = {}) {
  const deny = () => { throw new Error('Untrusted GitHub CLI'); };
  if (!isAbsolute(binary)) deny();
  if (realpathSync(binary) !== binary) deny();
  const file = statSync(binary);
  if (!file.isFile() || file.uid !== process.getuid() || (file.mode & 0o022) !== 0) deny();
  const environment = { HOME: homedir(), PATH: `${dirname(binary)}:/usr/bin:/bin`,
    LANG: 'C.UTF-8', GH_PROMPT_DISABLED: '1', GIT_TERMINAL_PROMPT: '0' };
  return async (target, options) => {
    const url = new URL(target);
    const prefix = `/repos/${POLICY.repository}`;
    const allowed = url.origin === 'https://api.github.com' && !url.username && !url.password && !url.hash
      && (url.pathname === prefix || url.pathname.startsWith(prefix + '/'))
      && fieldsMatch(options, { method: 'GET', redirect: 'error' });
    if (!allowed) throw new Error('Untrusted metadata request');
    const args = ['api', '--hostname', 'github.com', '--method', 'GET',
      '--header', 'Accept: application/vnd.github+json',
      '--header', 'X-GitHub-Api-Version: 2022-11-28', '--', url.pathname.slice(1) + url.search];
    try {
      const output = await execute(binary, args, { env: environment, encoding: 'utf8',
        timeout: 20_000, maxBuffer: 8 * 1024 * 1024 });
      JSON.parse(output.stdout);
      return new Response(output.stdout, { status: 200 });
    } catch {
      throw new Error('Authenticated GitHub metadata unavailable');
    }
  };
}

export function trustedBranch(branch) {
  if (!fieldsMatch(branch, { name: POLICY.branch, protected: true }) || !isSha(branch.commit?.sha)) return false;
  const checks = branch.protection?.required_status_checks;
  if (!fieldsMatch(checks, { enforcement_level: 'everyone' })) return false;
  if (!Array.isArray(checks.contexts) || !Array.isArray(checks.checks)) return false;
  const signed = new Set(checks.checks.filter(check => check?.app_id === POLICY.checkAppId).map(check => check.context));
  return POLICY.requiredChecks.every(name => checks.contexts.includes(name) && signed.has(name));
}

export function trustedRun(run, sha, branch = POLICY.branch) {
  if (!run || !isId(run.id) || !isId(run.run_attempt)) return false;
  if (![run.repository, run.head_repository].every(trustedRepository)) return false;
  return ['push', 'workflow_dispatch'].includes(run.event) && fieldsMatch(run, {
    workflow_id: POLICY.workflowId, path: POLICY.workflowPath, name: POLICY.workflowName,
    head_branch: branch, head_sha: sha, status: 'completed', conclusion: 'success',
  });
}

function jobFailure(jobs, run, sha) {
  if (!Array.isArray(jobs?.jobs) || jobs.total_count !== jobs.jobs.length) return 'incomplete-jobs';
  const grouped = new Map(POLICY.requiredChecks.map(name => [name, []]));
  for (const job of jobs.jobs) grouped.get(job?.name)?.push(job);
  for (const matches of grouped.values()) {
    if (matches.length !== 1) return 'required-check-missing-or-ambiguous';
    const job = matches[0];
    if (!isId(job.id) || !fieldsMatch(job, {
      run_id: run.id, run_attempt: run.run_attempt, head_sha: sha, head_branch: POLICY.branch,
      workflow_name: POLICY.workflowName, status: 'completed', conclusion: 'success',
    })) return 'required-check-not-successful';
  }
}

export function evaluateDeploymentReadiness(snapshot, state) {
  if (!validState(state)) return withheld('invalid-state');
  const { repository, branch, workflow, run, jobs, latestBranch, latestRun } = snapshot ?? {};
  const sha = branch?.commit?.sha;
  const gates = [
    ['untrusted-repository', () => trustedRepository(repository) && fieldsMatch(repository, {
      default_branch: POLICY.branch, archived: false, disabled: false })],
    ['branch-protection-mismatch', () => trustedBranch(branch) && trustedBranch(latestBranch)],
    ['untrusted-workflow', () => fieldsMatch(workflow, { id: POLICY.workflowId,
      path: POLICY.workflowPath, name: POLICY.workflowName, state: 'active' })],
    ['default-branch-moved', () => latestBranch.commit.sha === sha],
    ['ci-run-not-successful', () => trustedRun(run, sha)],
    ['run-changed', () => trustedRun(latestRun, sha) && fieldsMatch(latestRun, {
      id: run.id, run_attempt: run.run_attempt })],
  ];
  for (const [reason, accept] of gates) if (!accept()) return withheld(reason);
  const failure = jobFailure(jobs, run, sha);
  if (failure) return withheld(failure);
  if (state.deployedShas.includes(sha)) return withheld('already-deployed');
  return { schemaVersion: 1, ready: true, reason: 'ready', items: [{
    id: `${POLICY.repository}:${POLICY.branch}:${sha}`, repository: POLICY.repository,
    branch: POLICY.branch, sha, workflowId: POLICY.workflowId, runId: run.id, runAttempt: run.run_attempt,
  }] };
}

export async function checkDeploymentReadiness(state, { fetchImpl = fetch } = {}) {
  if (!validState(state)) return withheld('invalid-state');
  async function api(path) {
    const request = { method: 'GET', redirect: 'error', signal: AbortSignal.timeout(15_000),
      headers: { Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28',
        'User-Agent': 'panda-deployment-readiness', 'Cache-Control': 'no-cache' } };
    const result = await fetchImpl(`https://api.github.com/repos/${POLICY.repository}${path}`, request);
    if (result.ok) return result.json();
    throw new Error(`GitHub metadata request failed: HTTP ${result.status}`);
  }
  const branchPath = `/branches/${encodeURIComponent(POLICY.branch)}`;
  const workflowPath = `/actions/workflows/${POLICY.workflowId}`;
  const [repository, branch, workflow] = await Promise.all(['', branchPath, workflowPath].map(api));
  const sha = branch?.commit?.sha;
  if (!isSha(sha)) return withheld('invalid-default-sha');
  const query = new URLSearchParams({ branch: POLICY.branch, head_sha: sha, per_page: '100' });
  const runs = await api(`${workflowPath}/runs?${query}`);
  if (!Array.isArray(runs?.workflow_runs) || runs.total_count !== runs.workflow_runs.length)
    return withheld('incomplete-runs');
  let run;
  for (const item of runs.workflow_runs) {
    if (!isId(item?.run_number) || !isId(item?.id)) return withheld('incomplete-runs');
    if (!run || item.run_number > run.run_number) run = item;
  }
  if (!run) return withheld('no-current-ci-run');
  if (!trustedRun(run, sha)) return withheld('ci-run-not-successful');
  const jobs = await api(`/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`);
  const [latestBranch, latestRun] = await Promise.all([branchPath, `/actions/runs/${run.id}`].map(api));
  return evaluateDeploymentReadiness({ repository, branch, workflow, run, jobs, latestBranch, latestRun }, state);
}

export async function main(args = process.argv.slice(2)) {
  try {
    const validArgs = args[0] === '--state' && (args.length === 2 || (args.length === 4 && args[2] === '--github-cli'));
    if (!validArgs) throw new Error('Invalid readiness arguments');
    const state = JSON.parse(readFileSync(args[1], 'utf8'));
    const options = args.length === 4 ? { fetchImpl: githubCliFetch(args[3]) } : {};
    const result = await checkDeploymentReadiness(state, options);
    process.exitCode = result.reason === 'invalid-state' ? 1 : process.exitCode;
    console.log(JSON.stringify(result));
  } catch {
    console.error('Readiness unavailable; check state/arguments and GitHub availability or rate limits.');
    console.log(JSON.stringify(withheld('metadata-or-input-error')));
    process.exitCode = 1;
  }
}
if (process.argv[1] && pathToFileURL(realpathSync(process.argv[1])).href === import.meta.url) await main();

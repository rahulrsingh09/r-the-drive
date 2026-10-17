import { appendFileSync, realpathSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { POLICY, trustedBranch, trustedRun } from './fork-deploy-readiness.mjs';
import { syncFork } from './fork-sync.mjs';
import { demand, fieldsMatch, delay } from './operational-runtime.mjs';

const shaPattern = /^[0-9a-f]{40}$/;
const candidateBranch = sha => ['review', 'upstream', 'main', sha].join('/');
const branchPath = `/branches/${encodeURIComponent(POLICY.branch)}`;
const workflowPath = `/actions/workflows/${POLICY.workflowId}`;
const positiveId = value => Number.isSafeInteger(value) && value > 0;

export function githubApi(token, { fetchImpl = fetch } = {}) {
  demand(typeof token === 'string' && token.length > 0, 'missing-github-token');
  return async (path, body, method = body ? 'POST' : 'GET') => {
    demand(typeof path === 'string' && (path === '' || path[0] === '/')
      && !['..', '#', '\\'].some(part => path.includes(part)), 'untrusted-api-path');
    if (method === 'GET') demand(body === undefined, 'unexpected-get-body');
    else {
      const writes = {
        POST: () => path === `${workflowPath}/dispatches` && body && Object.keys(body).length === 1 && body.ref === POLICY.branch,
        PATCH: () => path === `/git/refs/heads/${POLICY.branch}` && body && Object.keys(body).length === 2
          && body.force === false && shaPattern.test(body.sha),
      };
      demand(Boolean(writes[method]?.()), 'untrusted-api-write');
    }
    const request = { method, redirect: 'error', signal: AbortSignal.timeout(20_000), headers: {
      Accept: 'application/vnd.github+json', Authorization: `Bearer ${token}`, 'Content-Type': 'application/json',
      'X-GitHub-Api-Version': '2022-11-28', 'User-Agent': 'panda-upstream-promotion', 'Cache-Control': 'no-cache',
    } };
    if (body) request.body = JSON.stringify(body);
    try {
      const response = await fetchImpl(`https://api.github.com/repos/${POLICY.repository}${path}`, request);
      demand(response.ok, 'github-request-failed');
      const data = await response.text();
      demand(data.length <= 2 * 1024 * 1024, 'github-response-too-large');
      return data.length ? JSON.parse(data) : null;
    } catch { throw new Error('github-request-unavailable-or-outcome-unknown'); }
  };
}

export function checkCandidateJobs(run, jobs, candidate) {
  const branch = candidateBranch(candidate);
  demand(trustedRun(run, candidate, branch) && run.event === 'push', 'candidate-ci-not-successful');
  demand(Array.isArray(jobs?.jobs) && jobs.total_count === jobs.jobs.length, 'incomplete-jobs');
  const grouped = new Map(POLICY.requiredChecks.map(name => [name, []]));
  for (const job of jobs.jobs) grouped.get(job?.name)?.push(job);
  for (const group of grouped.values()) {
    demand(group.length === 1, 'required-job-missing-or-ambiguous');
    demand(positiveId(group[0].id) && fieldsMatch(group[0], { run_id: run.id, run_attempt: run.run_attempt,
      head_sha: candidate, head_branch: branch, workflow_name: POLICY.workflowName,
      status: 'completed', conclusion: 'success' }), 'required-job-not-successful');
  }
}

class Promotion {
  constructor(api, prepare, now, sleep) { Object.assign(this, { api, prepare, now, sleep }); }
  async context(expected) {
    const [repo, branch, workflow] = await Promise.all(['', branchPath, workflowPath].map(path => this.api(path)));
    demand(fieldsMatch(repo, { id: POLICY.repositoryId, full_name: POLICY.repository, private: false,
      default_branch: POLICY.branch, archived: false, disabled: false }), 'repository-mismatch');
    demand(trustedBranch(branch) && branch.commit.sha === expected, 'protected-base-moved');
    demand(fieldsMatch(workflow, { id: POLICY.workflowId, path: POLICY.workflowPath,
      name: POLICY.workflowName, state: 'active' }), 'workflow-mismatch');
  }
  async candidate(base, sha) {
    const result = await this.prepare(base);
    const fixed = { source: 'main', status: 'candidate-ready', safeToValidate: true,
      base, candidate: sha, branch: candidateBranch(sha) };
    demand(fieldsMatch(result, fixed) && shaPattern.test(result.upstream)
      && ['changedControls', 'conflicts'].every(key => Array.isArray(result[key]) && result[key].length === 0),
    'candidate-requires-review-or-changed');
    const branch = candidateBranch(sha);
    const ref = await this.api(`/git/ref/heads/${branch}`);
    demand(fieldsMatch(ref, { ref: `refs/heads/${branch}` })
      && fieldsMatch(ref.object, { sha, type: 'commit' }), 'candidate-ref-moved');
  }
  async latest(branch, sha) {
    const query = new URLSearchParams({ branch, head_sha: sha, per_page: '100' });
    const list = await this.api(`${workflowPath}/runs?${query}`);
    demand(Array.isArray(list?.workflow_runs) && list.total_count === list.workflow_runs.length, 'incomplete-run-list');
    let newest;
    for (const run of list.workflow_runs) {
      demand(positiveId(run.id) && positiveId(run.run_number), 'incomplete-run-list');
      if (!newest || run.run_number > newest.run_number) newest = run;
    }
    return newest;
  }
  async defaultCi(sha) {
    await this.context(sha);
    const existing = await this.latest(POLICY.branch, sha);
    if (!existing) {
      await this.context(sha);
      await this.api(`${workflowPath}/dispatches`, { ref: POLICY.branch });
      return { dispatched: true };
    }
    demand(fieldsMatch(existing, { workflow_id: POLICY.workflowId, path: POLICY.workflowPath,
      head_sha: sha, head_branch: POLICY.branch }) && ['push', 'workflow_dispatch'].includes(existing.event), 'default-ci-mismatch');
    demand(existing.status !== 'completed' || existing.conclusion === 'success', 'default-ci-failed');
    return { dispatched: false, existingRunId: existing.id };
  }
  async completedCandidate(base, candidate) {
    const branch = candidateBranch(candidate);
    let run = await this.latest(branch, candidate);
    const deadline = this.now() + 55 * 60_000;
    for (;;) {
      if (run?.status === 'completed') break;
      demand(this.now() < deadline, 'candidate-ci-timeout');
      await this.sleep(15_000);
      await this.context(base);
      run = await this.latest(branch, candidate);
    }
    demand(trustedRun(run, candidate, branch) && run.event === 'push', 'candidate-ci-not-successful');
    const jobs = await this.api(`/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100`);
    checkCandidateJobs(run, jobs, candidate);
    return run;
  }
  async recheck(base, candidate, run) {
    await this.candidate(base, candidate);
    const fresh = await this.api(`/actions/runs/${run.id}`);
    const latest = await this.latest(candidateBranch(candidate), candidate);
    const identity = { id: run.id, run_attempt: run.run_attempt };
    demand(trustedRun(fresh, candidate, candidateBranch(candidate)) && fieldsMatch(fresh, identity)
      && fieldsMatch(latest, { ...identity, status: 'completed', conclusion: 'success' }), 'candidate-ci-changed');
    await this.context(base);
  }
}

export async function advanceUpstream(
  { base, candidate = '', source = 'main', status },
  { api, prepare = expectedBase => syncFork({ expectedBase, source: 'main' }), now = Date.now, sleep = delay },
) {
  demand(shaPattern.test(base) && source === 'main' && ['current', 'candidate-ready'].includes(status), 'upstream-review-required');
  const transaction = new Promotion(api, prepare, now, sleep);
  await transaction.context(base);
  if (status === 'current') {
    demand(candidate === '', 'unexpected-candidate');
    demand(fieldsMatch(await prepare(base), { status: 'current', base, source: 'main' }), 'upstream-changed');
    return { status: 'current', promoted: false, defaultCi: await transaction.defaultCi(base) };
  }
  demand(shaPattern.test(candidate) && candidate !== base, 'invalid-candidate');
  await transaction.candidate(base, candidate);
  const run = await transaction.completedCandidate(base, candidate);
  await transaction.recheck(base, candidate, run);
  await api(`/git/refs/heads/${POLICY.branch}`, { sha: candidate, force: false }, 'PATCH');
  await transaction.context(candidate);
  return { status: 'promoted', promoted: true, sha: candidate, candidateRunId: run.id,
    defaultCi: await transaction.defaultCi(candidate) };
}

export async function main(env = process.env) {
  let summary;
  try {
    demand(fieldsMatch(env, { GITHUB_REPOSITORY: POLICY.repository, GITHUB_REF: `refs/heads/${POLICY.branch}`,
      GITHUB_WORKFLOW_REF: `${POLICY.repository}/.github/workflows/fork-upstream-sync.yml@refs/heads/${POLICY.branch}`,
      GITHUB_SHA: env.FORK_SYNC_BASE }) && ['schedule', 'workflow_dispatch'].includes(env.GITHUB_EVENT_NAME), 'untrusted-workflow-context');
    const token = env.GITHUB_TOKEN;
    delete env.GITHUB_TOKEN;
    const result = await advanceUpstream({ base: env.FORK_SYNC_BASE, candidate: env.FORK_SYNC_CANDIDATE || '',
      source: env.FORK_SYNC_SOURCE || 'main', status: env.FORK_SYNC_STATUS }, { api: githubApi(token) });
    console.log(JSON.stringify(result));
    summary = `## Upstream automation\n\n\`\`\`json\n${JSON.stringify(result, null, 2)}\n\`\`\`\n\n` +
      'A promoted commit still requires protected-branch CI and the Northflank backup, deployment ' +
      'and verification workflow. No vehicle data or credentials are changed by this job.\n';
  } catch (error) {
    const message = error?.message;
    const reason = typeof message === 'string' && /^[a-z][a-z-]{1,100}$/.test(message) ? message : 'metadata-or-workflow-error';
    console.error(`Upstream automation stopped: ${reason}. Inspect sync metadata, CI and the protected branch before retrying.`);
    summary = `## Upstream automation stopped\n\nReason: \`${reason}\`.\n\nReview the sync report and candidate CI. If promotion ` +
      'already succeeded but dispatch did not, the next daily current-state check can resume CI. ' +
      'Do not force a merge or bypass required checks.\n';
    process.exitCode = 1;
  }
  if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, summary);
}
if (process.argv[1] && pathToFileURL(realpathSync(process.argv[1])).href === import.meta.url) await main();

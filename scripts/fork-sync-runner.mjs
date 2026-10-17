import { appendFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { syncFork as prepareFork, HARDENED_BRANCH, FORK_REPOSITORY } from './fork-sync.mjs';
import { direct } from './operational-runtime.mjs';

export function runSynchronization(process = globalThis.process, syncFork = prepareFork) {
  const env = process.env;
  try {
    const context = { GITHUB_REPOSITORY: FORK_REPOSITORY, GITHUB_REF: `refs/heads/${HARDENED_BRANCH}`,
      GITHUB_WORKFLOW_REF: `${FORK_REPOSITORY}/.github/workflows/fork-upstream-sync.yml@refs/heads/${HARDENED_BRANCH}` };
    if (Object.entries(context).some(([key, value]) => env[key] !== value) || !/^[0-9a-f]{40}$/.test(env.GITHUB_SHA || ''))
      throw new Error('Run synchronization using the installed trusted default-branch workflow');
    const sshKey = env.FORK_SYNC_SSH_KEY;
    delete process.env.FORK_SYNC_SSH_KEY;
    delete process.env.FORK_SYNC_TOKEN;
    if (!sshKey) throw new Error('The protected upstream automation environment needs its deploy key');
    const result = syncFork({ apply: true, source: env.GITHUB_EVENT_NAME === 'schedule' ? 'main' : env.FORK_SYNC_SOURCE || 'main',
      sshKey, expectedBase: env.GITHUB_SHA });
    return result;
  } catch (error) {
    return { status: 'error', safeToValidate: false, reason: error.message };
  }
}
export function main(env = process.env) {
  const result = runSynchronization({ env });
  const text = JSON.stringify(result, null, 2);
  const outputs = { candidate: result.candidate || '', base: result.base || '',
    safe: result.safeToValidate === true, status: result.status };
  console.log(text);
  writeFileSync(join(env.RUNNER_TEMP, 'fork-sync.json'), text + '\n');
  appendFileSync(env.GITHUB_OUTPUT, Object.entries(outputs).map(([key, value]) => `${key}=${value}\n`).join(''));
  const entities = { '&': '&amp;', '<': '&lt;', '>': '&gt;' };
  const safeText = text.replace(/[&<>]/g, char => entities[char]);
  appendFileSync(env.GITHUB_STEP_SUMMARY, `## Upstream candidate\n\n<pre>${safeText}</pre>\n\nNo promotion or deployment was performed.\n`);
  if (!new Set(['current', 'candidate-ready']).has(result.status)) process.exitCode = 1;
}
if (direct(import.meta.url)) main();

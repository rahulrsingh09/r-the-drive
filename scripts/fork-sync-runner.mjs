import { appendFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { syncFork, HARDENED_BRANCH, FORK_REPOSITORY } from './fork-sync.mjs';

let result;
try {
  if (
    process.env.GITHUB_REPOSITORY !== FORK_REPOSITORY ||
    process.env.GITHUB_REF !== `refs/heads/${HARDENED_BRANCH}` ||
    !/^[0-9a-f]{40}$/.test(process.env.GITHUB_SHA || '') ||
    process.env.GITHUB_WORKFLOW_REF !==
      `${FORK_REPOSITORY}/.github/workflows/fork-upstream-sync.yml@refs/heads/${HARDENED_BRANCH}`
  ) {
    throw new Error('Run synchronization using the installed trusted default-branch workflow');
  }
  const source =
    process.env.GITHUB_EVENT_NAME === 'schedule' ? 'main' : process.env.FORK_SYNC_SOURCE || 'main';
  const sshKey = process.env.FORK_SYNC_SSH_KEY;
  delete process.env.FORK_SYNC_SSH_KEY;
  delete process.env.FORK_SYNC_TOKEN;
  if (!sshKey) throw new Error('The protected upstream automation environment needs its deploy key');
  result = syncFork({ apply: true, source, sshKey, expectedBase: process.env.GITHUB_SHA });
} catch (error) {
  result = { status: 'error', safeToValidate: false, reason: error.message };
}
const json = JSON.stringify(result, null, 2);
console.log(json);
writeFileSync(join(process.env.RUNNER_TEMP, 'fork-sync.json'), `${json}\n`);
appendFileSync(
  process.env.GITHUB_OUTPUT,
  [
    `candidate=${result.candidate || ''}`,
    `base=${result.base || ''}`,
    `safe=${result.safeToValidate === true}`,
    `status=${result.status}`,
    '',
  ].join('\n')
);
const escaped = json.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
appendFileSync(
  process.env.GITHUB_STEP_SUMMARY,
  `## Upstream candidate\n\n<pre>${escaped}</pre>\n\nNo promotion or deployment was performed.\n`
);
if (!['current', 'candidate-ready'].includes(result.status)) process.exitCode = 1;

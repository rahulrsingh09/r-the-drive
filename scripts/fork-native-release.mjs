import { appendFileSync, realpathSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { checkDeploymentReadiness, POLICY } from './fork-deploy-readiness.mjs';

export async function queueRelease(input, { fetchImpl = fetch } = {}) {
  const { token, webhook, sha, runId } = input;
  const identifiers = /^[0-9a-f]{40}$/.test(sha) && /^[1-9][0-9]{0,19}$/.test(String(runId));
  if (!token || !identifiers) throw new Error('Invalid release input');
  const destination = new URL(webhook);
  const forbiddenParts = [destination.username, destination.password, destination.search, destination.hash];
  if (destination.origin !== 'https://webhooks.northflank.com' || forbiddenParts.some(Boolean)
    || !/^\/workflows\/[A-Za-z0-9_-]{32,256}$/.test(destination.pathname)) throw new Error('Invalid workflow webhook');
  const fetchMetadata = async (target, request) => {
    const location = new URL(target);
    const trusted = location.origin === 'https://api.github.com'
      && location.pathname.startsWith(`/repos/${POLICY.repository}`)
      && request.method === 'GET' && request.redirect === 'error';
    if (!trusted) throw new Error('Untrusted GitHub request');
    return fetchImpl(target, { ...request, headers: Object.assign({}, request.headers, { Authorization: `Bearer ${token}` }) });
  };
  const evidence = await checkDeploymentReadiness({ deployedShas: [] }, { fetchImpl: fetchMetadata });
  const exact = evidence.items?.find((item, index) => index === 0 && item.sha === sha && String(item.runId) === String(runId));
  if (!evidence.ready || !exact) throw new Error('The exact protected commit has not passed CI');
  for (const [key, value] of Object.entries({ sha, runId: String(runId), name: `GitHub ${sha.slice(0, 12)}` }))
    destination.searchParams.set(key, value);
  const request = { method: 'POST', redirect: 'error', signal: AbortSignal.timeout(20_000) };
  let response;
  try { response = await fetchImpl(destination, request); }
  catch { throw new Error('Northflank enqueue outcome unknown; inspect workflow runs before retrying'); }
  if (response.ok) return { status: 'queued', sha, runId: exact.runId };
  throw new Error('Northflank did not accept the release request');
}

export async function main(env = process.env) {
  try {
    const input = Object.fromEntries([['token', 'GITHUB_TOKEN'], ['webhook', 'NORTHFLANK_RELEASE_WEBHOOK'],
      ['sha', 'RELEASE_SHA'], ['runId', 'CI_RUN_ID']].map(([key, variable]) => [key, env[variable]]));
    const result = await queueRelease(input);
    console.log(JSON.stringify(result));
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY,
      `Northflank accepted release \`${result.sha}\`.\n\n` +
      'This job queues the release. Final backup, deployment, and verification results are in ' +
      '**Northflank → panda-private → Workflows → panda-verified-release**.\n');
  } catch {
    process.exitCode = 1;
    console.error('Release not confirmed. Inspect CI and Northflank workflow runs before retrying.');
  }
}
if (process.argv[1] && pathToFileURL(realpathSync(process.argv[1])).href === import.meta.url) await main();

import { appendFileSync, realpathSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { checkDeploymentReadiness, POLICY } from './fork-deploy-readiness.mjs';

export async function queueRelease({ token, webhook, sha, runId }, { fetchImpl = fetch } = {}) {
  if (!token || !/^[0-9a-f]{40}$/.test(sha) || !/^[1-9][0-9]{0,19}$/.test(String(runId))) {
    throw new Error('Invalid release input');
  }
  const url = new URL(webhook);
  if (url.origin !== 'https://webhooks.northflank.com' ||
      !/^\/workflows\/[A-Za-z0-9_-]{32,256}$/.test(url.pathname) ||
      url.username || url.password || url.search || url.hash) {
    throw new Error('Invalid workflow webhook');
  }
  const authenticated = (target, options) => {
    const parsed = new URL(target);
    if (parsed.origin !== 'https://api.github.com' ||
        !parsed.pathname.startsWith(`/repos/${POLICY.repository}`) ||
        options.method !== 'GET' || options.redirect !== 'error') {
      throw new Error('Untrusted GitHub request');
    }
    return fetchImpl(target, { ...options,
      headers: { ...options.headers, Authorization: `Bearer ${token}` },
    });
  };
  const result = await checkDeploymentReadiness({ deployedShas: [] }, { fetchImpl: authenticated });
  const item = result.items?.[0];
  if (!result.ready || item?.sha !== sha || String(item.runId) !== String(runId)) {
    throw new Error('The exact protected commit has not passed CI');
  }
  url.searchParams.set('sha', sha);
  url.searchParams.set('runId', String(runId));
  url.searchParams.set('name', `GitHub ${sha.slice(0, 12)}`);
  let response;
  try {
    response = await fetchImpl(url, {
      method: 'POST', redirect: 'error', signal: AbortSignal.timeout(20_000),
    });
  } catch {
    throw new Error('Northflank enqueue outcome unknown; inspect workflow runs before retrying');
  }
  if (!response.ok) throw new Error('Northflank did not accept the release request');
  return { status: 'queued', sha, runId: item.runId };
}

if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  try {
    const result = await queueRelease({
      token: process.env.GITHUB_TOKEN, webhook: process.env.NORTHFLANK_RELEASE_WEBHOOK,
      sha: process.env.RELEASE_SHA, runId: process.env.CI_RUN_ID,
    });
    console.log(JSON.stringify(result));
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY,
      `Northflank accepted release \`${result.sha}\`.\n\n` +
      'This job queues the release. Final backup, deployment, and verification results are in ' +
      '**Northflank → panda-private → Workflows → panda-verified-release**.\n');
  } catch {
    console.error('Release not confirmed. Inspect CI and Northflank workflow runs before retrying.');
    process.exitCode = 1;
  }
}

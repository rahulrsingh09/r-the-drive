import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { sshPushConfiguration } from './fork-push-transport.mjs';

export const FORK_REPOSITORY = 'your-org/r-the-drive';
export const HARDENED_BRANCH = 'mainline';
export const REPOSITORIES = {
  origin: `https://github.com/${FORK_REPOSITORY}.git`,
  upstream: 'https://upstream.panda.invalid/repository.git',
};

// An allowlist keeps credentials, injected Git config, and language hooks out of Git.
export function gitEnvironment(home) {
  return {
    PATH: process.env.PATH,
    HOME: home,
    TMPDIR: home,
    LANG: 'C.UTF-8',
    LC_ALL: 'C.UTF-8',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_ATTR_NOSYSTEM: '1',
    GIT_TERMINAL_PROMPT: '0',
    GIT_AUTHOR_NAME: 'Fork upstream automation',
    GIT_AUTHOR_EMAIL: 'fork-sync@example.invalid',
    GIT_COMMITTER_NAME: 'Fork upstream automation',
    GIT_COMMITTER_EMAIL: 'fork-sync@example.invalid',
  };
}

function git(cwd, args, { env = gitEnvironment(cwd), allowFailure = false, input } = {}) {
  const result = spawnSync(
    'git',
    [
      '-c',
      'core.hooksPath=/dev/null',
      '-c',
      'core.attributesFile=/dev/null',
      '-c',
      'credential.helper=',
      '-c',
      'commit.gpgSign=false',
      '-c',
      'http.followRedirects=false',
      ...args,
    ],
    { cwd, env, encoding: 'utf8', input, maxBuffer: 16 * 1024 * 1024 }
  );
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    // Subprocess diagnostics may contain credentials supplied to the push operation.
    throw new Error(`git ${args[0]} failed (exit ${result.status})`);
  }
  return { ok: result.status === 0, text: result.stdout.trim(), raw: result.stdout };
}

export function validateRemotes(cwd, repositories = REPOSITORIES) {
  for (const [remote, expected] of Object.entries(repositories)) {
    const accepted = [expected];
    if (expected.startsWith('https://github.com/')) {
      const repository = expected.slice('https://github.com/'.length).replace(/\.git$/, '');
      accepted.push(
        `https://github.com/${repository}`,
        `git@github.com:${repository}`,
        `git@github.com:${repository}.git`
      );
    }
    for (const direction of [[], ['--push']]) {
      const urls = git(cwd, ['remote', 'get-url', ...direction, '--all', remote]).text.split('\n');
      if (urls.length !== 1 || !accepted.includes(urls[0])) {
        throw new Error(`Unexpected ${remote} repository (${direction.length ? 'push' : 'fetch'})`);
      }
    }
  }
  if (process.env.GITHUB_REPOSITORY && process.env.GITHUB_REPOSITORY !== FORK_REPOSITORY) {
    throw new Error('This synchronization workflow belongs to the security fork');
  }
}

export function isControlPath(path) {
  return (
    /(^|\/)\.[^/]+/.test(path) ||
    /(^|\/)(patches|scripts|tools|config|compose)(\/|$)/i.test(path) ||
    /(^|\/)(action\.ya?ml|\.gitattributes|\.gitmodules|\.npmrc|\.pnpmfile\..*|package\.json|pnpm-.*\.yaml|Cargo\.(toml|lock)|rust-toolchain.*|Dockerfile.*|.*\.(config|policy)\.[^/]+)$/i.test(
      path
    ) ||
    /(^|\/)(security[^/]*|.*(?:auth|custody|session|token|permission|migration|enrollment|egress)[^/]*)(\/|$)/i.test(
      path
    ) ||
    /^apps\/(api|private-gateway)\//.test(path) ||
    /^packages\/hooks\//.test(path)
  );
}

function ancestor(cwd, older, newer) {
  return git(cwd, ['merge-base', '--is-ancestor', older, newer], { allowFailure: true }).ok;
}

function paths(cwd, older, newer) {
  return git(cwd, [
    'diff',
    '--no-ext-diff',
    '--no-textconv',
    '--no-renames',
    '--name-only',
    '-z',
    older,
    newer,
  ])
    .raw.split('\0')
    .filter(Boolean);
}

export function syncFork({
  cwd = process.cwd(),
  apply = false,
  source = 'main',
  token,
  sshKey,
  repositories = REPOSITORIES,
  expectedBase,
} = {}) {
  if (!['main', 'dev'].includes(source)) throw new Error('Source must be main or dev');
  if (token && sshKey) throw new Error('Choose one push credential');
  validateRemotes(cwd, repositories);
  if (git(cwd, ['status', '--porcelain']).text) {
    throw new Error('Use a clean checkout for upstream synchronization');
  }
  const isolated = mkdtempSync(join(tmpdir(), 'fork-sync-'));
  try {
    git(isolated, ['init', '--bare', '--template=', '.']);
    // Ignore tree-supplied merge drivers (including union) even without a worktree.
    mkdirSync(join(isolated, 'info'), { recursive: true });
    writeFileSync(join(isolated, 'info/attributes'), '* !merge\n');
    git(isolated, [
      'fetch',
      '--no-tags',
      repositories.origin,
      `refs/heads/${HARDENED_BRANCH}:refs/heads/base`,
      'refs/heads/main:refs/heads/mirror',
    ]);
    git(isolated, [
      'fetch',
      '--no-tags',
      repositories.upstream,
      'refs/heads/main:refs/heads/stable',
    ]);
    const resolve = (ref) => git(isolated, ['rev-parse', `${ref}^{commit}`]).text;
    const base = resolve('base');
    const stable = resolve('stable');
    const mirror = resolve('mirror');
    const devRef = git(
      isolated,
      ['ls-remote', '--exit-code', repositories.upstream, 'refs/heads/dev'],
      { allowFailure: true }
    );
    let dev = { status: 'unavailable', sha: null };
    if (devRef.ok) {
      git(isolated, ['fetch', '--no-tags', repositories.upstream, 'refs/heads/dev:refs/heads/dev']);
      const sha = resolve('dev');
      const common = git(isolated, ['merge-base', stable, sha], { allowFailure: true });
      dev = {
        status: sha === stable ? 'same-as-main' : 'differs-from-main',
        sha,
        relatedToMain: common.ok,
        relatedToBase: git(isolated, ['merge-base', base, sha], { allowFailure: true }).ok,
        commits: git(isolated, ['rev-list', '--left-right', '--count', `${stable}...${sha}`]).text,
      };
    }
    const upstream = source === 'main' ? stable : dev.sha;
    const result = {
      schemaVersion: 1,
      source,
      base,
      stable,
      upstream,
      mirror,
      dev,
      candidate: null,
      branch: null,
      applied: false,
      safeToValidate: false,
      changedControls: [],
      conflicts: [],
    };
    const block = (status, reason) => ({ ...result, status, reason, safeToValidate: false });
    if (expectedBase && expectedBase !== base) {
      return block(
        'base-moved',
        'The trusted default branch moved since this workflow started. Rerun from its new commit.'
      );
    }
    if (!ancestor(isolated, mirror, stable)) {
      return block(
        'mirror-diverged',
        'The main mirror diverged; review ancestry without force-pushing.'
      );
    }
    if (!upstream)
      return block('source-unavailable', 'The explicitly requested dev source is unavailable.');
    const common = git(isolated, ['merge-base', base, upstream], { allowFailure: true });
    if (!common.ok) {
      return block(
        'unrelated-history',
        'No shared ancestry with the hardened branch. Manual integration is required.'
      );
    }
    if (ancestor(isolated, upstream, base)) {
      // An up-to-date push also verifies the unattended credential without creating a CI event.
      if (apply)
        publish(isolated, repositories.origin, token, [[stable, 'main']], sshKey);
      return { ...result, status: 'current', applied: apply };
    }
    // Inspect upstream deltas too: a merge may otherwise hide a changed security control.
    const forkPaths = new Set(paths(isolated, common.text, base));
    result.changedControls = paths(isolated, common.text, upstream).filter(
      (path) => isControlPath(path) || forkPaths.has(path)
    );
    const merged = git(
      isolated,
      ['merge-tree', '--write-tree', '--name-only', '-z', base, upstream],
      { allowFailure: true }
    );
    if (!merged.ok) {
      const fields = merged.raw.split('\0');
      result.conflicts = fields.slice(1, fields.indexOf('', 1)).filter(Boolean);
      return block(
        'conflict',
        'Conflicting files require manual review, including security and migrations; no resolution was attempted.'
      );
    }
    const tree = merged.raw.split('\0')[0].trim();
    if (!/^[0-9a-f]{40}$/.test(tree)) throw new Error('Invalid merge tree');
    const timestamp = Math.max(
      ...[base, upstream].map((sha) =>
        Number(git(isolated, ['show', '-s', '--format=%ct', sha]).text)
      )
    );
    const candidate = git(isolated, ['commit-tree', tree, '-p', base, '-p', upstream], {
      env: {
        ...gitEnvironment(isolated),
        GIT_AUTHOR_DATE: `${timestamp} +0000`,
        GIT_COMMITTER_DATE: `${timestamp} +0000`,
      },
      input: `Merge upstream ${source} ${upstream} into hardened ${base}\n`,
    }).text;
    result.candidate = candidate;
    result.branch = `review/upstream/${source}/${candidate}`;
    result.changedControls = [
      ...new Set([
        ...result.changedControls,
        ...paths(isolated, base, candidate).filter(isControlPath),
      ]),
    ].sort();
    result.safeToValidate = result.changedControls.length === 0;
    if (apply && result.safeToValidate) {
      const existing = git(isolated, [
        'ls-remote',
        repositories.origin,
        `refs/heads/${result.branch}`,
      ]).text;
      if (existing && existing.split(/\s/)[0] !== candidate) {
        return block(
          'candidate-moved',
          'The immutable candidate branch already names a different commit.'
        );
      }
      const updates = [[candidate, result.branch]];
      if (mirror !== stable) updates.push([stable, 'main']);
      publish(isolated, repositories.origin, token, updates, sshKey);
      result.applied = true;
    }
    return {
      ...result,
      status: result.safeToValidate ? 'candidate-ready' : 'controls-review-required',
      reason: result.safeToValidate
        ? 'Candidate prepared; genuine CI and promotion checks are still required.'
        : 'Workflow, action, build, or security controls changed. Review before executing candidate code.',
    };
  } finally {
    rmSync(isolated, { recursive: true, force: true });
  }
}

function publish(cwd, origin, token, updates, sshKey) {
  let env = gitEnvironment(cwd);
  if (sshKey) ({ origin, env } = sshPushConfiguration(cwd, origin, sshKey, env));
  if (token) {
    env.GIT_CONFIG_COUNT = '1';
    env.GIT_CONFIG_KEY_0 = 'http.https://github.com/.extraheader';
    env.GIT_CONFIG_VALUE_0 = `AUTHORIZATION: basic ${Buffer.from(`x-access-token:${token}`).toString('base64')}`;
  }
  git(
    cwd,
    ['push', '--atomic', origin, ...updates.map(([sha, branch]) => `${sha}:refs/heads/${branch}`)],
    { env }
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2);
    if (args.some((arg) => !['--apply', '--json'].includes(arg))) {
      throw new Error('Usage: node scripts/fork-sync.mjs [--apply] [--json]');
    }
    const result = syncFork({
      apply: args.includes('--apply'),
      token: process.env.FORK_SYNC_TOKEN,
    });
    console.log(JSON.stringify(result, null, 2));
    if (!['current', 'candidate-ready'].includes(result.status)) process.exitCode = 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

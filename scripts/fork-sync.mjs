import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { sshPushConfiguration } from './fork-push-transport.mjs';
import { direct } from './operational-runtime.mjs';

export const FORK_REPOSITORY = 'your-org/r-the-drive';
export const HARDENED_BRANCH = 'mainline';
export const REPOSITORIES = { origin: `https://github.com/${FORK_REPOSITORY}.git`, upstream: 'https://upstream.panda.invalid/repository.git' };

export function gitEnvironment(home) {
  const environment = { PATH: process.env.PATH, HOME: home, TMPDIR: home, LANG: 'C.UTF-8', LC_ALL: 'C.UTF-8',
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', GIT_ATTR_NOSYSTEM: '1', GIT_TERMINAL_PROMPT: '0' };
  for (const role of ['AUTHOR', 'COMMITTER']) {
    environment[`GIT_${role}_NAME`] = 'Fork upstream automation';
    environment[`GIT_${role}_EMAIL`] = 'fork-sync@example.invalid';
  }
  return environment;
}

class GitWorkspace {
  constructor(directory) { this.directory = directory; }
  command(args, { env = gitEnvironment(this.directory), allowFailure = false, input } = {}) {
    const configuration = ['core.hooksPath=/dev/null', 'core.attributesFile=/dev/null',
      'credential.helper=', 'commit.gpgSign=false', 'http.followRedirects=false'];
    const argv = configuration.flatMap(value => ['-c', value]).concat(args);
    const output = spawnSync('git', argv, { cwd: this.directory, env, encoding: 'utf8', input, maxBuffer: 16 * 1024 * 1024 });
    if (output.error) throw output.error;
    const ok = output.status === 0;
    if (!ok && !allowFailure) throw new Error(`git ${args[0]} failed (exit ${output.status})`);
    return { ok, text: output.stdout.trim(), raw: output.stdout };
  }
  text(...args) { return this.command(args).text; }
  resolve(name) { return this.text('rev-parse', name + '^{commit}'); }
  ancestor(first, second) { return this.command(['merge-base', '--is-ancestor', first, second], { allowFailure: true }).ok; }
  common(first, second) { return this.command(['merge-base', first, second], { allowFailure: true }); }
  paths(first, second) {
    const result = this.command(['diff', '--no-ext-diff', '--no-textconv', '--no-renames', '--name-only', '-z', first, second]);
    return result.raw.split('\0').filter(name => name.length > 0);
  }
  fetch(repository, ...refs) { this.command(['fetch', '--no-tags', repository, ...refs]); }
  publish(origin, updates, token, sshKey) {
    let connection = { origin, env: gitEnvironment(this.directory) };
    if (sshKey) connection = sshPushConfiguration(this.directory, origin, sshKey, connection.env);
    if (token) Object.assign(connection.env, { GIT_CONFIG_COUNT: '1', GIT_CONFIG_KEY_0: 'http.https://github.com/.extraheader',
      GIT_CONFIG_VALUE_0: 'AUTHORIZATION: basic ' + Buffer.from('x-access-token:' + token).toString('base64') });
    const refs = updates.map(([sha, branch]) => sha + ':refs/heads/' + branch);
    this.command(['push', '--atomic', connection.origin, ...refs], { env: connection.env });
  }
  development(repository, stable, base) {
    const ref = this.command(['ls-remote', '--exit-code', repository, 'refs/heads/dev'], { allowFailure: true });
    if (!ref.ok) return { status: 'unavailable', sha: null };
    this.fetch(repository, 'refs/heads/dev:refs/heads/dev');
    const sha = this.resolve('dev');
    return { status: stable === sha ? 'same-as-main' : 'differs-from-main', sha,
      relatedToMain: this.common(stable, sha).ok, relatedToBase: this.common(base, sha).ok,
      commits: this.text('rev-list', '--left-right', '--count', `${stable}...${sha}`) };
  }
  candidate(base, upstream, source) {
    const merge = this.command(['merge-tree', '--write-tree', '--name-only', '-z', base, upstream], { allowFailure: true });
    const fields = merge.raw.split('\0');
    if (!merge.ok) return { conflicts: fields.slice(1, fields.indexOf('', 1)).filter(Boolean) };
    const tree = fields[0].trim();
    if (!/^[0-9a-f]{40}$/.test(tree)) throw new Error('Invalid merge tree');
    const timestamp = Math.max(Number(this.text('show', '-s', '--format=%ct', base)), Number(this.text('show', '-s', '--format=%ct', upstream)));
    const env = gitEnvironment(this.directory);
    for (const role of ['AUTHOR', 'COMMITTER']) env[`GIT_${role}_DATE`] = `${timestamp} +0000`;
    return { sha: this.command(['commit-tree', tree, '-p', base, '-p', upstream], {
      env, input: `Merge upstream ${source} ${upstream} into hardened ${base}\n`,
    }).text };
  }
}

export function validateRemotes(cwd, repositories = REPOSITORIES) {
  const repository = new GitWorkspace(cwd);
  for (const [name, expected] of Object.entries(repositories)) {
    const accepted = new Set([expected]);
    if (expected.startsWith('https://github.com/')) {
      const slug = expected.substring(19).replace(/\.git$/, '');
      for (const url of [`https://github.com/${slug}`, `git@github.com:${slug}`, `git@github.com:${slug}.git`]) accepted.add(url);
    }
    for (const direction of ['fetch', 'push']) {
      const flags = direction === 'push' ? ['--push'] : [];
      const urls = repository.text('remote', 'get-url', ...flags, '--all', name).split('\n');
      if (urls.length !== 1 || !accepted.has(urls[0])) throw new Error(`Unexpected ${name} repository (${direction})`);
    }
  }
  const context = process.env.GITHUB_REPOSITORY;
  if (context && context !== FORK_REPOSITORY) throw new Error('This synchronization workflow belongs to the security fork');
}

export function isControlPath(path) {
  const rules = [
    /(^|\/)\.[^/]+/,
    /(^|\/)(patches|scripts|tools|config|compose)(\/|$)/i,
    /(^|\/)(action\.ya?ml|\.gitattributes|\.gitmodules|\.npmrc|\.pnpmfile\..*|package\.json|pnpm-.*\.yaml|Cargo\.(toml|lock)|rust-toolchain.*|Dockerfile.*|.*\.(config|policy)\.[^/]+)$/i,
    /(^|\/)(security[^/]*|.*(?:auth|custody|session|token|permission|migration|enrollment|egress)[^/]*)(\/|$)/i,
    /^apps\/(api|private-gateway)\//, /^packages\/hooks\//,
  ];
  return rules.some(rule => rule.test(path));
}

function prepareCandidate(git, options) {
  const { repositories, source, expectedBase, apply, token, sshKey } = options;
  git.command(['init', '--bare', '--template=', '.']);
  mkdirSync(join(git.directory, 'info'), { recursive: true });
  writeFileSync(join(git.directory, 'info/attributes'), '* !merge\n');
  git.fetch(repositories.origin, `refs/heads/${HARDENED_BRANCH}:refs/heads/base`, 'refs/heads/main:refs/heads/mirror');
  git.fetch(repositories.upstream, 'refs/heads/main:refs/heads/stable');
  const [base, stable, mirror] = ['base', 'stable', 'mirror'].map(ref => git.resolve(ref));
  const dev = git.development(repositories.upstream, stable, base);
  const upstream = source === 'main' ? stable : dev.sha;
  const result = { schemaVersion: 1, source, base, stable, upstream, mirror, dev,
    candidate: null, branch: null, applied: false, safeToValidate: false, changedControls: [], conflicts: [] };
  const blocked = (status, reason) => Object.assign({}, result, { status, reason, safeToValidate: false });
  const gates = [
    [() => !expectedBase || expectedBase === base, 'base-moved', 'The trusted default branch moved since this workflow started. Rerun from its new commit.'],
    [() => git.ancestor(mirror, stable), 'mirror-diverged', 'The main mirror diverged; review ancestry without force-pushing.'],
    [() => Boolean(upstream), 'source-unavailable', 'The explicitly requested dev source is unavailable.'],
  ];
  for (const [accept, status, reason] of gates) if (!accept()) return blocked(status, reason);
  const common = git.common(base, upstream);
  if (!common.ok) return blocked('unrelated-history', 'No shared ancestry with the hardened branch. Manual integration is required.');
  if (git.ancestor(upstream, base)) {
    if (apply) git.publish(repositories.origin, [[stable, 'main']], token, sshKey);
    return { ...result, status: 'current', applied: apply };
  }
  const localChanges = new Set(git.paths(common.text, base));
  result.changedControls = git.paths(common.text, upstream).filter(path => localChanges.has(path) || isControlPath(path));
  const merge = git.candidate(base, upstream, source);
  if (merge.conflicts) {
    result.conflicts = merge.conflicts;
    return blocked('conflict', 'Conflicting files require manual review, including security and migrations; no resolution was attempted.');
  }
  result.candidate = merge.sha;
  result.branch = `review/upstream/${source}/${merge.sha}`;
  const controls = new Set(result.changedControls);
  for (const path of git.paths(base, merge.sha)) if (isControlPath(path)) controls.add(path);
  result.changedControls = Array.from(controls).sort();
  result.safeToValidate = controls.size === 0;
  if (apply && result.safeToValidate) {
    const existing = git.text('ls-remote', repositories.origin, `refs/heads/${result.branch}`);
    if (existing && existing.split(/\s/)[0] !== merge.sha)
      return blocked('candidate-moved', 'The immutable candidate branch already names a different commit.');
    const updates = [[merge.sha, result.branch]];
    if (stable !== mirror) updates.push([stable, 'main']);
    git.publish(repositories.origin, updates, token, sshKey);
    result.applied = true;
  }
  const outcome = result.safeToValidate ? ['candidate-ready', 'Candidate prepared; genuine CI and promotion checks are still required.']
    : ['controls-review-required', 'Workflow, action, build, or security controls changed. Review before executing candidate code.'];
  return { ...result, status: outcome[0], reason: outcome[1] };
}

export function syncFork({ cwd = process.cwd(), apply = false, source = 'main', token, sshKey,
  repositories = REPOSITORIES, expectedBase } = {}) {
  if (source !== 'main' && source !== 'dev') throw new Error('Source must be main or dev');
  if (token && sshKey) throw new Error('Choose one push credential');
  validateRemotes(cwd, repositories);
  if (new GitWorkspace(cwd).text('status', '--porcelain')) throw new Error('Use a clean checkout for upstream synchronization');
  const workspace = new GitWorkspace(mkdtempSync(join(tmpdir(), 'fork-sync-')));
  try { return prepareCandidate(workspace, { apply, source, token, sshKey, repositories, expectedBase }); }
  finally { rmSync(workspace.directory, { recursive: true, force: true }); }
}

export function main(args = process.argv.slice(2)) {
  try {
    const invalid = args.find(arg => arg !== '--apply' && arg !== '--json');
    if (invalid) throw new Error('Usage: node scripts/fork-sync.mjs [--apply] [--json]');
    const result = syncFork({ apply: args.includes('--apply'), token: process.env.FORK_SYNC_TOKEN });
    console.log(JSON.stringify(result, null, 2));
    if (result.status !== 'current' && result.status !== 'candidate-ready') process.exitCode = 1;
  } catch (error) { process.exitCode = 1; console.error(error.message); }
}
if (direct(import.meta.url)) main();

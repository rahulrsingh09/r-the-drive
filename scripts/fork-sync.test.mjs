import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { HARDENED_BRANCH, REPOSITORIES, syncFork as sync, gitEnvironment } from './fork-sync.mjs';

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'panda-fork-sync-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const run = (cwd, ...args) =>
    execFileSync('git', args, {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  const origin = join(root, 'origin.git');
  const upstream = join(root, 'upstream.git');
  const cwd = join(root, 'work');
  run(root, 'init', '--bare', origin);
  run(root, 'init', '--bare', upstream);
  mkdirSync(cwd);
  run(cwd, 'init', '-b', 'main');
  run(cwd, 'config', 'user.name', 'Fork sync test');
  run(cwd, 'config', 'user.email', 'test@example.invalid');
  const commit = (name, content) => {
    const file = join(cwd, name);
    mkdirSync(join(file, '..'), { recursive: true });
    writeFileSync(file, content);
    run(cwd, 'add', name);
    run(cwd, 'commit', '-m', `Update ${name}`);
    return run(cwd, 'rev-parse', 'HEAD');
  };
  commit('README.md', 'upstream\n');
  run(cwd, 'remote', 'add', 'origin', origin);
  run(cwd, 'remote', 'add', 'upstream', upstream);
  run(cwd, 'push', 'origin', 'main');
  run(cwd, 'push', 'upstream', 'main');
  run(cwd, 'checkout', '-b', HARDENED_BRANCH);
  commit('SECURITY-FORK.txt', 'preserve this security patch\n');
  run(cwd, 'push', 'origin', HARDENED_BRANCH);
  const hardened = run(cwd, 'rev-parse', 'HEAD');
  const advance = (name = 'upstream-fix.txt', content = 'bug fix\n') => {
    run(cwd, 'checkout', 'main');
    const sha = commit(name, content);
    run(cwd, 'push', 'upstream', 'main');
    run(cwd, 'checkout', HARDENED_BRANCH);
    return sha;
  };
  const syncFork = (options = {}) => sync({ cwd, repositories: { origin, upstream }, ...options });
  return { root, cwd, run, origin, upstream, commit, advance, hardened, syncFork };
}

test('dry run leaves remote branches unchanged', (t) => {
  const f = fixture(t);
  const before = f.run(f.cwd, 'ls-remote', 'origin');
  f.advance();
  assert.equal(f.syncFork({}).status, 'candidate-ready');
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin'), before);
});

test('sync preserves hardened history and publishes an unmerged review branch', (t) => {
  const f = fixture(t);
  const upstream = f.advance();
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'candidate-ready');
  assert.equal(result.safeToValidate, true);
  assert.equal(f.run(f.cwd, 'rev-parse', `origin/${HARDENED_BRANCH}`), f.hardened);
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin', 'refs/heads/main').split(/\s/)[0], upstream);
  f.run(f.cwd, 'fetch', 'origin', result.branch);
  assert.equal(
    f.run(f.cwd, 'show', `${result.candidate}:SECURITY-FORK.txt`),
    'preserve this security patch'
  );
  assert.equal(f.run(f.cwd, 'symbolic-ref', '--short', 'HEAD'), HARDENED_BRANCH);
  assert.equal(f.syncFork({ apply: true }).candidate, result.candidate);
});

test('upstream changes to fork workflow controls require manual review', (t) => {
  const f = fixture(t);
  f.advance('.github/workflows/fork-ci.yml', 'unreviewed workflow\n');
  const result = f.syncFork({ apply: true });
  assert.equal(result.safeToValidate, false);
  assert.deepEqual(result.changedControls, ['.github/workflows/fork-ci.yml']);
});

test('a clean text merge touching an existing fork patch still requires review', (t) => {
  const f = fixture(t);
  const file = 'apps/web/src/components/widget.tsx';
  const lines = Array.from({ length: 20 }, (_, i) => `line ${i}`).join('\n') + '\n';
  f.run(f.cwd, 'checkout', 'main');
  f.commit(file, lines);
  f.run(f.cwd, 'push', 'upstream', 'main');
  f.run(f.cwd, 'checkout', HARDENED_BRANCH);
  f.run(f.cwd, 'merge', '--no-edit', 'main');
  f.commit(file, lines.replace('line 0\n', 'private control\n'));
  f.run(f.cwd, 'push', 'origin', HARDENED_BRANCH);
  f.advance(file, lines.replace('line 19\n', 'upstream change\n'));
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'controls-review-required');
  assert.deepEqual(result.conflicts, []);
  assert.deepEqual(result.changedControls, [file]);
  assert.equal(result.applied, false);
});

test('merge conflicts keep the hardened branch intact and leave no partial merge', (t) => {
  const f = fixture(t);
  f.commit('README.md', 'security-specific change\n');
  f.run(f.cwd, 'push', 'origin', HARDENED_BRANCH);
  const before = f.run(f.cwd, 'rev-parse', 'HEAD');
  f.advance('README.md', 'conflicting upstream change\n');
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'conflict');
  assert.deepEqual(result.conflicts, ['README.md']);
  assert.equal(f.run(f.cwd, 'rev-parse', 'HEAD'), before);
  assert.equal(f.run(f.cwd, 'status', '--porcelain'), '');
  assert.equal(f.run(f.cwd, 'ls-remote', '--heads', 'origin', 'review/upstream/*'), '');
});

test('a divergent mirror is never force pushed', (t) => {
  const f = fixture(t);
  f.run(f.cwd, 'checkout', 'main');
  f.commit('fork-only-main.txt', 'unexpected mirror modification\n');
  f.run(f.cwd, 'push', 'origin', 'main');
  f.run(f.cwd, 'checkout', HARDENED_BRANCH);
  assert.equal(f.syncFork({ apply: true }).status, 'mirror-diverged');
});

test('no upstream change creates no candidate or remote update', (t) => {
  const f = fixture(t);
  const before = f.run(f.cwd, 'ls-remote', 'origin');
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'current');
  assert.equal(result.candidate, null);
  assert.equal(result.safeToValidate, false);
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin'), before);
});

test('candidate SHA and parents remain immutable across retries and new upstream commits', (t) => {
  const f = fixture(t);
  const upstream = f.advance();
  const first = f.syncFork({ apply: true });
  const retry = f.syncFork({ apply: true });
  assert.equal(retry.candidate, first.candidate);
  f.run(f.cwd, 'fetch', 'origin', first.branch);
  assert.equal(
    f.run(f.cwd, 'show', '-s', '--format=%P', first.candidate),
    `${f.hardened} ${upstream}`
  );
  f.advance('another.txt');
  const next = f.syncFork({ apply: true });
  assert.notEqual(next.candidate, first.candidate);
  assert.notEqual(next.branch, first.branch);
  assert.equal(
    f.run(f.cwd, 'ls-remote', 'origin', `refs/heads/${first.branch}`).split(/\s/)[0],
    first.candidate
  );
});

test('all workflow, local action, install, hook, security and migration changes block execution', (t) => {
  const f = fixture(t);
  const files = [
    '.github/workflows/innocent.yaml',
    '.github/actions/local/action.yml',
    'custom/action.yaml',
    '.gitattributes',
    '.gitmodules',
    '.githooks/pre-push',
    '.npmrc',
    '.pnpmfile.cjs',
    'packages/ui/package.json',
    'pnpm-lock.yaml',
    'scripts/unrelated-name.mjs',
    'tools/security/check.mjs',
    'apps/api/src/keys.rs',
    'apps/api/migrations/new.sql',
    'apps/private-gateway/src/worker.ts',
    'packages/hooks/src/useAuth.ts',
    '.cargo/config.toml',
    '.gitleaks.toml',
    '.semgrep.yml',
    '.trivyignore',
  ];
  f.run(f.cwd, 'checkout', 'main');
  for (const file of files) f.commit(file, 'changed control\n');
  f.run(f.cwd, 'push', 'upstream', 'main');
  f.run(f.cwd, 'checkout', HARDENED_BRANCH);
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'controls-review-required');
  assert.equal(result.safeToValidate, false);
  assert.deepEqual(result.changedControls, files.sort());
  assert.ok(result.candidate);
  assert.equal(result.applied, false);
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin', `refs/heads/${result.branch}`), '');
});

test('remote rejection happens before fetch or push, including alternate push URLs', (t) => {
  const f = fixture(t);
  const before = f.run(f.cwd, 'ls-remote', 'origin');
  assert.throws(() => sync({ cwd: f.cwd, apply: true }), /Unexpected origin repository/);
  f.run(f.cwd, 'config', 'remote.origin.pushurl', f.upstream);
  assert.throws(() => f.syncFork({ apply: true }), /Unexpected origin repository \(push\)/);
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin'), before);
});

test('security and migration conflicts stay unresolved even with a union merge attribute', (t) => {
  const f = fixture(t);
  const file = 'apps/api/migrations/0001.sql';
  f.run(f.cwd, 'checkout', 'main');
  f.commit('.gitattributes', '*.sql merge=union\n');
  f.commit(file, 'original migration\n');
  f.run(f.cwd, 'push', 'upstream', 'main');
  f.run(f.cwd, 'checkout', HARDENED_BRANCH);
  f.run(f.cwd, 'merge', '--no-edit', 'main');
  f.commit(file, 'fork migration\n');
  f.run(f.cwd, 'push', 'origin', HARDENED_BRANCH);
  f.advance(file, 'upstream migration\n');
  const before = f.run(f.cwd, 'ls-remote', 'origin');
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'conflict');
  assert.ok(result.conflicts.includes(file));
  assert.equal(result.candidate, null);
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin'), before);
});

test('dev is awareness only unless explicitly selected and never replaces the stable mirror', (t) => {
  const f = fixture(t);
  f.run(f.cwd, 'checkout', '-b', 'dev', 'main');
  const dev = f.commit('dev-only.txt', 'experimental\n');
  f.run(f.cwd, 'push', 'upstream', 'dev');
  f.run(f.cwd, 'checkout', HARDENED_BRANCH);
  const mirror = f.run(f.cwd, 'ls-remote', 'origin', 'refs/heads/main');
  const daily = f.syncFork({ apply: true });
  assert.equal(daily.status, 'current');
  assert.equal(daily.dev.sha, dev);
  assert.equal(daily.candidate, null);
  const manual = f.syncFork({ apply: true, source: 'dev' });
  assert.equal(manual.upstream, dev);
  assert.equal(manual.status, 'candidate-ready');
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin', 'refs/heads/main'), mirror);
});

test('unrelated dev ancestry is reported and explicit dev integration blocks', (t) => {
  const f = fixture(t);
  f.run(f.cwd, 'checkout', '--orphan', 'dev');
  f.run(f.cwd, 'rm', '-rf', '.');
  f.commit('new-history.txt', 'unrelated\n');
  f.run(f.cwd, 'push', 'upstream', 'dev');
  f.run(f.cwd, 'checkout', HARDENED_BRANCH);
  assert.equal(f.syncFork().dev.relatedToBase, false);
  const result = f.syncFork({ apply: true, source: 'dev' });
  assert.equal(result.status, 'unrelated-history');
  assert.equal(result.candidate, null);
});

test('Git environment excludes inherited tokens and executable configuration', () => {
  const before = { ...process.env };
  try {
    Object.assign(process.env, {
      GH_TOKEN: 'synthetic-write-token',
      GITHUB_TOKEN: 'synthetic-write-token',
      FORK_SYNC_TOKEN: 'synthetic-write-token',
      GIT_CONFIG_COUNT: '1',
      GIT_CONFIG_KEY_0: 'core.sshCommand',
      GIT_CONFIG_VALUE_0: 'execute-candidate',
      NODE_OPTIONS: '--import=execute-candidate',
      AWS_ACCESS_KEY_ID: 'synthetic',
    });
    const env = gitEnvironment('/isolated');
    assert.equal(JSON.stringify(env).includes('synthetic'), false);
    assert.equal(JSON.stringify(env).includes('execute-candidate'), false);
    assert.equal(env.GIT_CONFIG_COUNT, undefined);
  } finally {
    for (const key of Object.keys(process.env)) if (!(key in before)) delete process.env[key];
    Object.assign(process.env, before);
  }
});

test('a moved trusted base or reused candidate name blocks publication', (t) => {
  const f = fixture(t);
  f.advance();
  const before = f.run(f.cwd, 'ls-remote', 'origin');
  assert.equal(f.syncFork({ apply: true, expectedBase: '0'.repeat(40) }).status, 'base-moved');
  assert.equal(f.run(f.cwd, 'ls-remote', 'origin'), before);
  const preview = f.syncFork();
  f.run(f.cwd, 'push', 'origin', `${f.hardened}:refs/heads/${preview.branch}`);
  const result = f.syncFork({ apply: true });
  assert.equal(result.status, 'candidate-moved');
  assert.equal(result.safeToValidate, false);
  assert.equal(
    f.run(f.cwd, 'ls-remote', 'origin', `refs/heads/${preview.branch}`).split(/\s/)[0],
    f.hardened
  );
});

test('actual Git subprocesses only receive the write credential for push and never run hooks', (t) => {
  const f = fixture(t);
  f.advance();
  const realGit = execFileSync('sh', ['-c', 'command -v git'], { encoding: 'utf8' }).trim();
  const bin = join(f.root, 'bin');
  const trace = join(f.root, 'trace.jsonl');
  const sentinel = join(f.root, 'hook-ran');
  mkdirSync(bin);
  writeFileSync(
    join(bin, 'git'),
    `#!${process.execPath}
const { appendFileSync } = require('node:fs');
const { spawnSync } = require('node:child_process');
appendFileSync(${JSON.stringify(trace)}, JSON.stringify({
  args: process.argv.slice(2),
  credential: Object.values(process.env).some(v => v.includes('synthetic-token') || v.includes('${Buffer.from('x-access-token:synthetic-token').toString('base64')}'))
}) + '\\n');
const child = spawnSync(${JSON.stringify(realGit)}, process.argv.slice(2), { stdio: 'inherit' });
process.exit(child.status ?? 1);
`
  );
  chmodSync(join(bin, 'git'), 0o755);
  for (const hook of ['post-checkout', 'pre-merge-commit', 'pre-push']) {
    const file = join(f.cwd, '.git/hooks', hook);
    writeFileSync(file, `#!/bin/sh\ntouch '${sentinel}'\n`);
    chmodSync(file, 0o755);
  }
  const previousPath = process.env.PATH;
  const previousToken = process.env.GH_TOKEN;
  try {
    process.env.PATH = `${bin}:${previousPath}`;
    process.env.GH_TOKEN = 'synthetic-token';
    assert.equal(f.syncFork({ apply: true, token: 'synthetic-token' }).status, 'candidate-ready');
  } finally {
    process.env.PATH = previousPath;
    if (previousToken === undefined) delete process.env.GH_TOKEN;
    else process.env.GH_TOKEN = previousToken;
  }
  const calls = readFileSync(trace, 'utf8').trim().split('\n').map(JSON.parse);
  const privileged = calls.filter((call) => call.credential);
  assert.equal(privileged.length, 1);
  assert.ok(privileged[0].args.includes('push'));
  assert.ok(calls.some((call) => call.args.includes('merge-tree')));
  assert.equal(
    calls.some((call) => call.args.includes('checkout')),
    false
  );
  assert.equal(existsSync(sentinel), false);
});

test('SSH key files exist only for fixed-repository publication and are removed afterward', (t) => {
  const f = fixture(t);
  f.advance();
  f.run(f.cwd, 'remote', 'set-url', 'origin', REPOSITORIES.origin);
  f.run(f.cwd, 'remote', 'set-url', 'upstream', REPOSITORIES.upstream);
  const realGit = execFileSync('sh', ['-c', 'command -v git'], { encoding: 'utf8' }).trim();
  const bin = join(f.root, 'bin');
  const trace = join(f.root, 'trace.jsonl');
  mkdirSync(bin);
  writeFileSync(join(bin, 'git'), `#!${process.execPath}
const { appendFileSync, existsSync } = require('node:fs');
const { spawnSync } = require('node:child_process');
const args = process.argv.slice(2);
appendFileSync(${JSON.stringify(trace)}, JSON.stringify({
  args, cwd: process.cwd(), ssh: !!process.env.GIT_SSH_COMMAND,
  rawKey: Object.values(process.env).some(v => v.includes('BEGIN OPENSSH PRIVATE KEY')),
  agent: !!process.env.SSH_AUTH_SOCK,
  keyFile: existsSync(process.cwd() + '/upstream-key')
}) + '\\n');
const mapped = args.map(v => v === ${JSON.stringify(REPOSITORIES.origin)} ||
  v === 'ssh://git@ssh.github.com:443/your-org/r-the-drive.git' ? ${JSON.stringify(f.origin)} :
  v === ${JSON.stringify(REPOSITORIES.upstream)} ? ${JSON.stringify(f.upstream)} : v);
const child = spawnSync(${JSON.stringify(realGit)}, mapped, { stdio: 'inherit' });
process.exit(child.status ?? 1);
`);
  chmodSync(join(bin, 'git'), 0o755);
  const previousPath = process.env.PATH;
  const previousAgent = process.env.SSH_AUTH_SOCK;
  try {
    process.env.PATH = `${bin}:${previousPath}`;
    process.env.SSH_AUTH_SOCK = 'synthetic-agent';
    assert.equal(f.syncFork({
      repositories: REPOSITORIES, apply: true,
      sshKey: '-----BEGIN OPENSSH PRIVATE KEY-----\nc3ludGhldGlj\n-----END OPENSSH PRIVATE KEY-----\n',
    }).status, 'candidate-ready');
  } finally {
    process.env.PATH = previousPath;
    if (previousAgent === undefined) delete process.env.SSH_AUTH_SOCK;
    else process.env.SSH_AUTH_SOCK = previousAgent;
  }
  const calls = readFileSync(trace, 'utf8').trim().split('\n').map(JSON.parse);
  assert.ok(calls.every((c) => !c.rawKey && !c.agent));
  const pushes = calls.filter((c) => c.ssh || c.keyFile);
  assert.equal(pushes.length, 1);
  assert.ok(pushes[0].args.includes('push'));
  assert.ok(pushes[0].ssh && pushes[0].keyFile);
  assert.ok(pushes[0].args.includes('ssh://git@ssh.github.com:443/your-org/r-the-drive.git'));
  assert.equal(existsSync(pushes[0].cwd), false);
});

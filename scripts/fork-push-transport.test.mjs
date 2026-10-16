import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { sshPushConfiguration } from './fork-push-transport.mjs';
import { gitEnvironment, REPOSITORIES, syncFork } from './fork-sync.mjs';

const key = '-----BEGIN OPENSSH PRIVATE KEY-----\nc3ludGhldGljLW9ubHk=\n-----END OPENSSH PRIVATE KEY-----\n';
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'fork-ssh-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

test('SSH transport confines credentials to private files and pins the GitHub host', (t) => {
  const root = fixture(t);
  const configuration = sshPushConfiguration(root, REPOSITORIES.origin, key, gitEnvironment(root));
  assert.equal(configuration.origin, 'ssh://git@ssh.github.com:443/your-org/r-the-drive.git');
  assert.equal(statSync(join(root, 'upstream-key')).mode & 0o777, 0o600);
  assert.equal(statSync(join(root, 'upstream-hosts')).mode & 0o777, 0o600);
  assert.equal(readFileSync(join(root, 'upstream-key'), 'utf8'), key);
  assert.match(readFileSync(join(root, 'upstream-hosts'), 'utf8'), /^\[ssh.github.com\]:443 ssh-ed25519 /);
  assert.ok(!JSON.stringify(configuration).includes(key));
  for (const option of ['IdentitiesOnly=yes', 'IdentityAgent=none', 'StrictHostKeyChecking=yes',
    'HostKeyAlgorithms=ssh-ed25519', 'BatchMode=yes', 'PasswordAuthentication=no',
    'KbdInteractiveAuthentication=no', 'GlobalKnownHostsFile=/dev/null', 'ConnectTimeout=15'])
    assert.ok(configuration.env.GIT_SSH_COMMAND.includes(option));
  assert.ok(configuration.env.GIT_SSH_COMMAND.startsWith("'/usr/bin/ssh' '-F' '/dev/null'"));
});

test('shell metacharacters in a temporary path remain inert single arguments', (t) => {
  const root = fixture(t);
  const directory = join(root, "key's $(touch injected)");
  mkdirSync(directory);
  const { env } = sshPushConfiguration(directory, REPOSITORIES.origin, key, gitEnvironment(root));
  const args = env.GIT_SSH_COMMAND.replace(/^'\/usr\/bin\/ssh' /, '');
  const result = spawnSync('/bin/sh', ['-c', `set -- ${args}; printf '%s\\0' "$@"`], {
    cwd: root, env: gitEnvironment(root), encoding: 'utf8',
  });
  assert.equal(result.status, 0);
  assert.ok(result.stdout.split('\0').includes(join(directory, 'upstream-key')));
  assert.ok(result.stdout.split('\0').includes(`UserKnownHostsFile=${join(directory, 'upstream-hosts')}`));
  assert.equal(existsSync(join(root, 'injected')), false);
});

test('foreign repositories and malformed or oversized keys fail before writing a key', (t) => {
  for (const [origin, value] of [
    ['https://attacker.invalid/repo.git', key],
    ['https://github.com/other/repo.git', key],
    [REPOSITORIES.origin, 'synthetic-secret'],
    [REPOSITORIES.origin, key + 'extra-data'],
    [REPOSITORIES.origin, key.repeat(200)],
    [REPOSITORIES.origin, undefined],
  ]) {
    const root = fixture(t);
    assert.throws(() => sshPushConfiguration(root, origin, value, gitEnvironment(root)));
    assert.equal(existsSync(join(root, 'upstream-key')), false);
  }
});

test('a preexisting private-key path is never overwritten', (t) => {
  const root = fixture(t);
  sshPushConfiguration(root, REPOSITORIES.origin, key, gitEnvironment(root));
  assert.throws(() => sshPushConfiguration(root, REPOSITORIES.origin, key, gitEnvironment(root)));
  assert.equal(readFileSync(join(root, 'upstream-key'), 'utf8'), key);
});

test('push credential types cannot be combined', () => {
  assert.throws(() => syncFork({ token: 'synthetic-token', sshKey: key }), /Choose one push credential/);
});

test('workflow restricts the SSH secret to the trusted preparation environment', () => {
  const workflow = readFileSync(new URL('../.github/workflows/fork-upstream-sync.yml', import.meta.url), 'utf8');
  const prepare = workflow.split('  propose:')[1].split('  automate:')[0];
  assert.match(prepare, /environment: upstream-automation/);
  assert.match(prepare, /contents: read/);
  assert.match(prepare, /FORK_SYNC_SSH_KEY: \$\{\{ secrets.UPSTREAM_PUSH_KEY \}\}/);
  assert.ok(!workflow.split('  automate:')[1].includes('UPSTREAM_PUSH_KEY'));
  const runner = readFileSync(new URL('./fork-sync-runner.mjs', import.meta.url), 'utf8');
  assert.ok(runner.indexOf('delete process.env.FORK_SYNC_SSH_KEY') < runner.indexOf('result = syncFork'));
});

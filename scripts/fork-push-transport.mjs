import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

const repository = 'your-org/r-the-drive';
const hostKey = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl';
const shellWord = value => "'" + value.split("'").join("'\\''") + "'";

export function sshPushConfiguration(directory, origin, key, environment) {
  if (origin !== `https://github.com/${repository}.git`) throw new Error('untrusted-ssh-repository');
  const keyPattern = /^-----BEGIN OPENSSH PRIVATE KEY-----\n[A-Za-z0-9+/\n=]+\n-----END OPENSSH PRIVATE KEY-----\n?$/;
  if (!(typeof key === 'string' && key.length <= 8192 && keyPattern.test(key)))
    throw new Error('invalid-upstream-deploy-key');
  const files = { identity: join(directory, 'upstream-key'), hosts: join(directory, 'upstream-hosts') };
  const contents = [[files.identity, key.endsWith('\n') ? key : key + '\n'],
    [files.hosts, `[ssh.github.com]:443 ${hostKey}\n`]];
  for (const [file, text] of contents) writeFileSync(file, text, { flag: 'wx', mode: 0o600 });
  const settings = ['IdentitiesOnly=yes', 'IdentityAgent=none', 'StrictHostKeyChecking=yes',
    `UserKnownHostsFile=${files.hosts}`, 'GlobalKnownHostsFile=/dev/null',
    'HostKeyAlgorithms=ssh-ed25519', 'BatchMode=yes', 'PasswordAuthentication=no',
    'KbdInteractiveAuthentication=no', 'ConnectTimeout=15'];
  const argv = ['/usr/bin/ssh', '-F', '/dev/null', '-i', files.identity];
  for (const setting of settings) argv.push('-o', setting);
  return { origin: `ssh://git@ssh.github.com:443/${repository}.git`, env: {
    ...environment, GIT_SSH_VARIANT: 'ssh', GIT_SSH_COMMAND: argv.map(shellWord).join(' '),
  } };
}

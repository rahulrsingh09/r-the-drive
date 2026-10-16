import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

const repository = 'your-org/r-the-drive';
const hostKey =
  'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl';
const quote = (value) => `'${value.replaceAll("'", "'\\''")}'`;

export function sshPushConfiguration(directory, origin, key, environment) {
  if (origin !== `https://github.com/${repository}.git`)
    throw new Error('untrusted-ssh-repository');
  if (typeof key !== 'string' || key.length > 8192 ||
    !/^-----BEGIN OPENSSH PRIVATE KEY-----\n[A-Za-z0-9+/\n=]+\n-----END OPENSSH PRIVATE KEY-----\n?$/.test(key))
    throw new Error('invalid-upstream-deploy-key');
  const identity = join(directory, 'upstream-key');
  const knownHosts = join(directory, 'upstream-hosts');
  writeFileSync(identity, key.endsWith('\n') ? key : `${key}\n`, { mode: 0o600, flag: 'wx' });
  writeFileSync(knownHosts, `[ssh.github.com]:443 ${hostKey}\n`, { mode: 0o600, flag: 'wx' });
  return {
    origin: `ssh://git@ssh.github.com:443/${repository}.git`,
    env: {
      ...environment,
      GIT_SSH_VARIANT: 'ssh',
      GIT_SSH_COMMAND: [
        '/usr/bin/ssh', '-F', '/dev/null', '-i', identity,
        '-o', 'IdentitiesOnly=yes', '-o', 'IdentityAgent=none',
        '-o', 'StrictHostKeyChecking=yes', '-o', `UserKnownHostsFile=${knownHosts}`,
        '-o', 'GlobalKnownHostsFile=/dev/null', '-o', 'HostKeyAlgorithms=ssh-ed25519',
        '-o', 'BatchMode=yes', '-o', 'PasswordAuthentication=no',
        '-o', 'KbdInteractiveAuthentication=no', '-o', 'ConnectTimeout=15',
      ].map(quote).join(' '),
    },
  };
}

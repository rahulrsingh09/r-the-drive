#!/usr/bin/env node
// Probe the production COPY instructions using only an already-local base image.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
assert(args.length === 2 && args[0] === '--base-image',
  'Usage: node scripts/verify-nginx-config-permissions.mjs --base-image <local-image-with-sh-and-stat>');
const baseImage = args[1];
assert.match(baseImage, /^[a-z0-9][a-z0-9._/:@-]*$/i, 'Expected a local image reference');
execFileSync('docker', ['image', 'inspect', baseImage], { stdio: 'ignore' });
const dockerfile = readFileSync(join(root, 'compose/Dockerfile'), 'utf8');
const copies = ['nginx.conf', 'security-headers.conf'].map((name) => {
  const source = `compose/nginx/${name}`;
  const destination = `/etc/nginx/${name}`;
  const matches = dockerfile.split('\n').filter((line) =>
    line.startsWith('COPY ') && line.trim().endsWith(`${source} ${destination}`));
  assert.equal(matches.length, 1, `Expected one production COPY for ${name}`);
  return matches[0];
});
assert.match(dockerfile, /^USER 1001:1001$/m, 'Production must retain UID/GID 1001');

const context = mkdtempSync(join(tmpdir(), 'panda-nginx-permissions-'));
const image = `panda-nginx-permissions:${process.pid}-${Date.now()}`;
const docker = (command) => execFileSync('docker', command, { stdio: 'inherit' });
try {
  mkdirSync(join(context, 'compose/nginx'), { recursive: true });
  for (const name of ['nginx.conf', 'security-headers.conf']) {
    const file = join(context, 'compose/nginx', name);
    writeFileSync(file, readFileSync(join(root, 'compose/nginx', name)));
    chmodSync(file, 0o600);
  }
  writeFileSync(join(context, 'Dockerfile'),
    [`FROM ${baseImage}`, 'USER 0:0', ...copies, 'USER 1001:1001', ''].join('\n'));
  docker(['build', '--pull=false', '--network=none', '--tag', image, context]);
  docker(['run', '--rm', '--network=none', '--read-only', '--cap-drop=ALL',
    '--security-opt=no-new-privileges', '--entrypoint=/bin/sh', image, '-ec', `
      test "$(id -u):$(id -g)" = 1001:1001
      cat /etc/nginx/nginx.conf /etc/nginx/security-headers.conf >/dev/null
      for config in /etc/nginx/nginx.conf /etc/nginx/security-headers.conf; do
        test "$(stat -c '%a:%u:%g' "$config")" = 644:0:0
        test ! -w "$config"
      done
    `]);
  console.log('nginx configuration permissions passed: 0600 inputs become root-owned 0644 files readable by UID 1001');
} finally {
  try { execFileSync('docker', ['image', 'rm', '--force', image], { stdio: 'ignore' }); } catch {}
  rmSync(context, { recursive: true, force: true });
}

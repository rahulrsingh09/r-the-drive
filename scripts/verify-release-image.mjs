#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { childCompletion, direct, option } from './operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export function imageBuildPlan(args, now = Date.now()) {
  const read = (name, fallback) => option(args, '--' + name, fallback);
  const platforms = args.includes('--all-platforms') ? 'linux/amd64,linux/arm64' : read('platforms', 'linux/amd64');
  const timeoutMinutes = Number(read('timeout-minutes', '45'));
  if (!(Number.isFinite(timeoutMinutes) && timeoutMinutes > 0)) throw new Error('--timeout-minutes must be a positive number.');
  const output = join(tmpdir(), `panda-release-image-${now}.oci`);
  const flags = [['--progress=plain'], ['--platform', platforms], ['--output', `type=oci,dest=${output}`],
    ['--file', './compose/Dockerfile']];
  const logs = [`Building release image for ${platforms}.`];
  const cache = read('cache-ref');
  const jobs = read('cargo-build-jobs');
  if (cache) flags.push(['--cache-from', `type=registry,ref=${cache}`]);
  if (jobs) flags.push(['--build-arg', `CARGO_BUILD_JOBS=${jobs}`]);
  if (args.includes('--no-cache')) { flags.push(['--no-cache']); logs.push('Build cache is disabled.'); }
  if (cache) logs.push(`Importing the GitHub-compatible registry cache from ${cache}.`);
  if (jobs) logs.push(`Using ${jobs} Cargo build jobs.`);
  logs.push(`The build will time out after ${timeoutMinutes} minutes.`);
  return { platforms, timeoutMinutes, output, args: ['buildx', 'build', ...flags.flat(), '.'], logs };
}
export async function main(args = process.argv.slice(2), launch = spawn) {
  const plan = imageBuildPlan(args);
  plan.logs.forEach(message => console.log(message));
  const start = Date.now();
  const child = launch('docker', plan.args, { cwd: root, stdio: 'inherit', windowsHide: true });
  const timer = setTimeout(() => {
    console.error(`Release image build exceeded ${plan.timeoutMinutes} minutes.`);
    child.kill();
  }, plan.timeoutMinutes * 60_000);
  try {
    const result = await childCompletion(child, 'close');
    if (result.code !== 0) throw new Error(`docker buildx exited with code ${result.code}.`);
    console.log(`Release image build passed for ${plan.platforms} in ${Math.round((Date.now() - start) / 1000)} seconds.`);
  } finally { clearTimeout(timer); rmSync(plan.output, { force: true }); }
}
if (direct(import.meta.url)) await main();

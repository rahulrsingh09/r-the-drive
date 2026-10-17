#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Commands, direct, option, pollUntil } from './operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export function restorePlan(args, environment = process.env) {
  const packageArgument = option(args, '--package');
  if (!packageArgument) throw new Error('--package is required.');
  const files = { packagePath: resolve(packageArgument), envFile: resolve(option(args, '--env-file', '.env')),
    composeFile: resolve(root, option(args, '--compose-file', 'compose/docker-compose.yml')) };
  for (const [key, label] of [['packagePath', 'Recovery package'], ['envFile', 'Compose env file'], ['composeFile', 'Compose file']])
    if (!existsSync(files[key])) throw new Error(`${label} does not exist: ${files[key]}`);
  const project = option(args, '--project', `panda-restore-${Date.now().toString(36)}`);
  const sourceBuild = args.includes('--source-build');
  const compose = ['compose', '-p', project, '--env-file', files.envFile, '-f', files.composeFile];
  if (sourceBuild) compose.push('-f', 'compose/docker-compose.build.yml');
  return { ...files, project, compose, force: args.includes('--force'),
    build: sourceBuild && !args.includes('--skip-build'), env: { ...environment, PANDA_ENV_FILE: files.envFile } };
}

export function restoreEnvironment(file, name) {
  for (const line of readFileSync(file, 'utf8').split(/\r?\n/)) {
    if (line.trim().startsWith(name + '=')) return line.substring(line.indexOf('=') + 1).trim().replace(/^['"]|['"]$/g, '');
  }
}

export async function restore(plan, { commands = new Commands(root, plan.env), probe = spawnSync,
  fetchImpl = fetch, poll = pollUntil } = {}) {
  const docker = (...args) => commands.invoke('docker', plan.compose.concat(args));
  const engine = action => {
    const arguments_ = ['run', '--rm', '--no-deps', '--entrypoint', '/app/panda-restore-agent'];
    if (action === 'host-restore') arguments_.push('-v', `${plan.packagePath}:/restore-package:ro`);
    arguments_.push('panda', action);
    if (action === 'host-restore') arguments_.push('/restore-package', ...(plan.force ? ['--force'] : []));
    return docker(...arguments_);
  };
  let state = 'unprepared';
  try {
    if (plan.build) docker('build', 'panda');
    docker('up', '-d', 'timescaledb');
    const user = restoreEnvironment(plan.envFile, 'POSTGRES_USER') ?? 'panda';
    let streak = 0;
    await poll(() => {
      const result = probe('docker', [...plan.compose, 'exec', '-T', 'timescaledb', 'psql',
        '-U', user, '-d', 'panda', '-At', '-c', 'SELECT 1'], { cwd: root, stdio: 'ignore', env: plan.env });
      if (result.status !== 0) streak = 0;
      else streak += 1;
      return streak >= 5;
    }, { timeout: 120_000, message: 'Timed out waiting for stable PostgreSQL readiness.' });
    engine('host-restore');
    state = 'prepared';
    docker('stop', 'panda');
    engine('host-activate');
    state = 'activated';
    docker('up', '-d', 'panda');
    const host = restoreEnvironment(plan.envFile, 'PANDA_HEALTH_HOST') ?? 'localhost';
    const port = restoreEnvironment(plan.envFile, 'PANDA_ORIGIN_PORT') ?? '8080';
    const origin = `http://${host}:${port}`;
    await poll(async () => {
      try { return (await fetchImpl(origin + '/health')).ok; } catch { return false; }
    }, { timeout: 180_000, message: `Timed out waiting for ${origin}/health` });
    const response = await fetchImpl(origin + '/v1/auth/setup');
    if ((await response.json()).setup_required) throw new Error('Restored application unexpectedly requires setup.');
    engine('host-finalize');
    state = 'complete';
    console.log(`Restore completed into Compose project ${plan.project}.`);
    console.log('Provider credentials and live sessions were intentionally excluded and require reauthentication.');
  } catch (failure) {
    if (state === 'prepared' || state === 'activated') {
      probe('docker', [...plan.compose, 'stop', 'panda'], { cwd: root, stdio: 'inherit', env: plan.env });
      try {
        engine('host-rollback');
        if (state === 'activated') docker('up', '-d', 'panda');
        console.error(state === 'activated'
          ? 'Restore verification failed; the previous database and artwork were restored.'
          : 'Restore preparation failed; the isolated candidate was discarded.');
      } catch (rollbackError) {
        console.error('Automatic rollback failed. The retained restore state must be recovered before retrying.', rollbackError);
      }
    }
    throw failure;
  }
}
if (direct(import.meta.url)) await restore(restorePlan(process.argv.slice(2)));

#!/usr/bin/env node
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertDisposableUpgradeDatabase } from './lib/database-reset-guard.mjs';
import { Commands, direct, pollUntil } from './operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const apiDir = resolve(root, 'apps/api');
const payloadCount = 250_000;
const expectedIdentityCount = 200_000;
export function parseStats(output) {
  const fields = /status=(\S+) pending=(\d+) payloads=(\d+) identities=(\d+)/.exec(output);
  if (!fields) throw new Error(`Could not parse fixture stats: ${output}`);
  return Object.fromEntries(['status', 'pending', 'payloads', 'identities'].map((key, index) =>
    [key, index === 0 ? fields[index + 1] : Number(fields[index + 1])]));
}
export class PopulatedUpgrade {
  constructor(env = process.env, { execute = execFileSync, launch = spawn, fetchImpl = fetch, poll = pollUntil } = {}) {
    this.databaseUrl = assertDisposableUpgradeDatabase(env);
    this.port = Number(env.UPGRADE_API_PORT ?? 3121);
    if (!Number.isInteger(this.port) || this.port < 1024 || this.port > 65535)
      throw new Error('UPGRADE_API_PORT must be an integer between 1024 and 65535.');
    Object.assign(this, { env, execute, launch, fetchImpl, poll });
    this.baseUrl = `http://127.0.0.1:${this.port}`;
    this.commands = new Commands(apiDir, { ...env, DATABASE_URL: this.databaseUrl, PGOPTIONS: '-c search_path=public' }, execute);
    this.container = `panda-populated-upgrade-${process.pid}`;
    const suffix = process.platform === 'win32' ? '.exe' : '';
    this.apiBinary = resolve(apiDir, 'target/debug', 'panda-api' + suffix);
    this.fixtureBinary = resolve(apiDir, 'target/debug', 'charge_identity_upgrade_fixture' + suffix);
    this.output = '';
  }
  fixture(mode) {
    try { return this.execute(this.fixtureBinary, [mode], { cwd: apiDir, env: this.commands.env, encoding: 'utf8', windowsHide: true }); }
    catch (error) {
      const text = [error.stdout, error.stderr].filter(Boolean).join('');
      if (text) process.stdout.write(text);
      return null;
    }
  }
  stats() {
    const output = this.fixture('--stats');
    if (!output) throw new Error('Fixture stats command failed.');
    process.stdout.write(output);
    return parseStats(output);
  }
  initialize() {
    this.directory = mkdtempSync(join(tmpdir(), 'panda-populated-upgrade-'));
    this.key = join(this.directory, 'restore-agent-key');
    writeFileSync(this.key, 'populated-upgrade-fixture-key\n', { mode: 0o600 });
    const migrationRoot = resolve(apiDir, 'migrations');
    const migrations = readdirSync(migrationRoot).filter(file => /^000[1-6]_.*\.sql$/.test(file)).sort();
    if (migrations.length !== 6) throw new Error(`Expected exactly six pre-0007 migrations, found ${migrations.length}.`);
    for (const name of migrations) copyFileSync(join(migrationRoot, name), join(this.directory, name));
    try { this.commands.invoke('cargo', ['sqlx', 'database', 'drop', '-y', '--force'], { stdio: 'pipe', windowsHide: true }); }
    catch (error) { if (!/does not exist/i.test([error.stdout, error.stderr].filter(Boolean).join(''))) throw error; }
    const steps = [['sqlx', 'database', 'create'], ['sqlx', 'migrate', 'run', '--source', this.directory],
      ['build', '--bin', 'panda-api', '--bin', 'charge_identity_upgrade_fixture']];
    for (const args of steps) this.commands.invoke('cargo', args, { windowsHide: true });
    const output = this.fixture('--seed');
    if (!output) throw new Error('Synthetic charge payload fixture seeding failed.');
    process.stdout.write(output);
    if (![this.apiBinary, this.fixtureBinary].every(existsSync)) throw new Error('Expected candidate API and fixture binaries were not built.');
  }
  runtime() {
    const url = new URL(this.databaseUrl);
    url.search = '?options=-c%20search_path%3Dpanda,timeseries,public';
    return { ...this.commands.env, DATABASE_URL: url.toString(), PANDA_ENV: 'development', PORT: String(this.port),
      PANDA_BIND_ADDRESS: '127.0.0.1', ALLOWED_ORIGINS: `${this.baseUrl},http://localhost:${this.port}`,
      REDIS_URL: this.env.REDIS_URL ?? 'redis://127.0.0.1:6379', BACKUP_ARTIFACT_DIR: this.directory,
      VEHICLE_IMAGE_CACHE_DIR: join(this.directory, 'vehicle-images'), RESTORE_AGENT_KEY_FILE: this.key,
      CHARGE_IDENTITY_BACKFILL_BATCH_SIZE: '1000', CHARGE_IDENTITY_BACKFILL_PAUSE_MS: '250' };
  }
  launchPlan() {
    const runtime = this.runtime();
    if (!this.env.UPGRADE_IMAGE) return { command: this.apiBinary, args: [], cwd: apiDir, env: runtime };
    const url = new URL(runtime.DATABASE_URL);
    url.hostname = 'host.docker.internal';
    const variables = { DATABASE_URL: url.toString(), REDIS_URL: runtime.REDIS_URL.replace('127.0.0.1', 'host.docker.internal'),
      PANDA_ENV: 'development', PORT: '3001', PANDA_BIND_ADDRESS: '127.0.0.1', ALLOWED_ORIGINS: this.baseUrl, COOKIE_INSECURE: 'true',
      BACKUP_ARTIFACT_DIR: '/backups', RESTORE_AGENT_KEY_FILE: '/backups/.restore-agent-key',
      CHARGE_IDENTITY_BACKFILL_BATCH_SIZE: '1000', CHARGE_IDENTITY_BACKFILL_PAUSE_MS: '250' };
    const args = ['run', '--rm', '--name', this.container, '--platform', this.env.UPGRADE_PLATFORM ?? 'linux/amd64',
      '--add-host', 'host.docker.internal:host-gateway', '-p', `${this.port}:8080`];
    for (const [key, value] of Object.entries(variables)) args.push('-e', `${key}=${value}`);
    args.push('-v', `${this.key}:/backups/.restore-agent-key:ro`, this.env.UPGRADE_IMAGE);
    return { command: 'docker', args, cwd: root, env: this.env };
  }
  start() {
    this.output = '';
    const { command, args, ...options } = this.launchPlan();
    this.child = this.launch(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    for (const stream of ['stdout', 'stderr']) this.child[stream].on('data', chunk => {
      const text = chunk.toString(); this.output += text; process[stream].write(`[upgrade-api] ${text}`);
    });
  }
  async healthy() {
    await this.poll(async () => {
      if (this.child?.exitCode != null) throw new Error(`API exited with code ${this.child.exitCode} before becoming healthy.\n${this.output}`);
      try { return (await this.fetchImpl(this.baseUrl + '/health')).ok; } catch { return false; }
    }, { timeout: 120_000, interval: 500, message: `Timed out waiting for ${this.baseUrl}/health.\n${this.output}` });
  }
  async stop() {
    const child = this.child;
    this.child = undefined;
    if (!child) return;
    if (child.exitCode === null) child.kill('SIGTERM');
    await new Promise(done => {
      const timer = setTimeout(() => { if (child.exitCode === null) child.kill('SIGKILL'); done(); }, 10_000);
      child.once('close', () => { clearTimeout(timer); done(); });
    });
    if (this.env.UPGRADE_IMAGE) {
      try { this.execute('docker', ['rm', '--force', this.container], { cwd: root, stdio: 'ignore', windowsHide: true }); }
      catch { /* Docker --rm normally completed cleanup. */ }
    }
  }
  async main() {
    try {
      console.log(`Using disposable upgrade database ${new URL(this.databaseUrl).pathname.slice(1)}.`);
      this.initialize();
      this.start(); await this.healthy();
      const partial = await this.poll(() => {
        const stats = this.stats(); return stats.pending > 0 && stats.pending < payloadCount ? stats : false;
      }, { timeout: 120_000, message: 'The API did not demonstrate partial backfill progress before the timeout.' });
      if (partial.pending <= 0 || partial.pending >= payloadCount) throw new Error('Health did not precede a still-pending backfill.');
      console.log('Health-first startup confirmed while the populated backfill was still pending.');
      await this.stop();
      const interrupted = this.stats();
      if (interrupted.pending <= 0) throw new Error('The interruption checkpoint unexpectedly had no pending rows.');
      console.log(`Interrupted with ${interrupted.pending} pending rows; restarting the candidate API.`);
      this.start(); await this.healthy();
      const completed = await this.poll(() => {
        const output = this.fixture('--check-complete');
        if (!output) return false;
        process.stdout.write(output); return parseStats(output);
      }, { timeout: 600_000, interval: 1500, message: 'The populated charge identity backfill did not complete before the timeout.' });
      if (completed.pending !== 0 || completed.payloads !== payloadCount || completed.identities !== expectedIdentityCount)
        throw new Error(`Unexpected completed fixture state: ${JSON.stringify(completed)}`);
      await this.stop();
      this.start(); await this.healthy();
      const output = this.fixture('--check-complete');
      if (!output) throw new Error('Completed fixture failed after the second API restart.');
      process.stdout.write(output);
      console.log('Restart-safe completion and duplicate-free identity count confirmed.');
      await this.stop();
      console.log('Populated charge identity upgrade verification passed.');
    } finally {
      await this.stop();
      if (this.directory) rmSync(this.directory, { recursive: true, force: true });
    }
  }
}
if (direct(import.meta.url)) await new PopulatedUpgrade().main();

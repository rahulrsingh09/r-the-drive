#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Commands, direct, option, pollUntil } from './operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export function readEnvValue(file, name) {
  for (const line of readFileSync(file, 'utf8').split(/\r?\n/)) {
    const entry = line.trim();
    if (!entry.startsWith(name + '=')) continue;
    const value = entry.slice(name.length + 1).trim();
    const quoted = value.length >= 2 && ['"', "'"].includes(value[0]) && value.at(-1) === value[0];
    return quoted ? value.slice(1, -1) : value;
  }
}
export function readSetupToken(file) {
  const inline = readEnvValue(file, 'PANDA_SETUP_TOKEN');
  if (inline) return inline;
  const path = readEnvValue(file, 'PANDA_SETUP_TOKEN_FILE');
  if (!path || !existsSync(path)) return undefined;
  return readFileSync(path, 'utf8').trimEnd();
}
export function sortJson(value) {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) return Array.from(value, sortJson);
  const result = {};
  for (const key of Object.keys(value).sort()) result[key] = sortJson(value[key]);
  return result;
}

export class FreshInstall {
  constructor(args, { commands = new Commands(root), probe = spawnSync, fetchImpl = fetch, poll = pollUntil, env = process.env } = {}) {
    Object.assign(this, { commands, probe, fetchImpl, poll, env });
    const fields = ['mode', 'production-env', 'image-tag', 'image-ref', 'keys-source'];
    this.options = Object.fromEntries(fields.map(name => [name, option(args, '--' + name)]));
    this.options.mode ??= 'all';
    this.sourceBuild = args.includes('--source-build');
    this.project = `panda-fresh-${Date.now().toString(36)}`;
    this.port = String(18080 + Math.floor(Math.random() * 1000));
    this.compose = ['compose', '-p', this.project, '-f', 'compose/docker-compose.yml'];
    if (this.sourceBuild) this.compose.push('-f', 'compose/docker-compose.build.yml');
    if (this.options['keys-source']) this.compose.push('-f', 'compose/docker-compose.keys.yml');
    this.started = false;
  }
  command(args, options = {}) {
    return this.commands.invoke('docker', this.compose.concat(['--env-file', this.options['production-env']], args),
      { env: this.environment, ...options });
  }
  async healthy(url) {
    await this.poll(async () => { try { return (await this.fetchImpl(url)).ok; } catch { return false; } },
      { timeout: 120000, message: `Timed out waiting for ${url}` });
  }
  async dashboards(origin, token) {
    const response = await this.fetchImpl(origin + '/v1/dashboards', { headers: { authorization: `Bearer ${token}` } });
    if (!response.ok) throw new Error(`Fresh dashboard list failed with ${response.status}.`);
    const rows = await response.json();
    const names = ['dashboard', 'battery', 'efficiency', 'charging', 'trips'];
    const defaults = rows.filter(row => row.owner_id == null && row.is_default === true);
    if (defaults.length !== names.length) throw new Error('Fresh install did not seed exactly five system dashboards.');
    for (const name of names) {
      const expected = JSON.parse(readFileSync(resolve(root, 'packages/dashboards/src/defaults', name + '.json'), 'utf8'));
      const actual = rows.find(row => row.owner_id == null && row.slug === expected.slug);
      if (!actual) throw new Error(`Fresh install is missing the ${expected.slug} system dashboard.`);
      const equivalent = JSON.stringify(sortJson(actual.config)) === JSON.stringify(sortJson(expected));
      if (!equivalent) throw new Error(`Fresh install ${expected.slug} dashboard does not match its bundled baseline.`);
    }
  }
  async owner(origin) {
    await this.healthy(origin + '/health');
    const setup = await (await this.fetchImpl(origin + '/v1/auth/setup')).json();
    if (!setup.setup_required) throw new Error('Fresh stack unexpectedly already has a user.');
    const proof = setup.setup_proof_required ? readSetupToken(this.options['production-env']) : undefined;
    if (setup.setup_proof_required && !proof) throw new Error('Production first-owner verification requires PANDA_SETUP_TOKEN in the supplied env file.');
    const body = { email: 'owner@example.test', password: 'fresh-install-password' };
    if (proof) body.setup_token = proof;
    const register = value => this.fetchImpl(origin + '/v1/auth/register', {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(value),
    });
    const response = await register(body);
    if (response.status !== 201) throw new Error(`First owner registration failed with ${response.status}.`);
    const token = (await response.json()).access_token;
    if (!token) throw new Error('First owner registration did not return an access token.');
    await this.dashboards(origin, token);
    const closed = await register({ email: 'second@example.test', password: 'fresh-install-password' });
    if (closed.status !== 403) throw new Error(`Registration remained open after owner setup (status ${closed.status}).`);
    return token;
  }
  async production() {
    const file = this.options['production-env'];
    if (!file || !existsSync(file)) throw new Error('--production-env must point to a valid, ephemeral production env file.');
    const tag = this.options['image-tag'], ref = this.options['image-ref'], keys = this.options['keys-source'];
    if (tag && ref) throw new Error('--image-tag and --image-ref are mutually exclusive.');
    this.dataRoot = mkdtempSync(join(tmpdir(), 'panda-fresh-data-'));
    this.environment = { ...this.env, PANDA_ORIGIN_PORT: this.port, PANDA_ENV_FILE: resolve(file), PANDA_DATA_DIR: this.dataRoot.split('\\').join('/') };
    const image = this.sourceBuild ? 'panda:local' : ref || (tag ? `${this.env.PANDA_IMAGE_REGISTRY ?? 'ghcr.io/your-org'}/panda:${tag}` : undefined);
    if (image) this.environment.PANDA_IMAGE = image;
    if (keys) this.environment.PANDA_KEYS_SOURCE = resolve(keys);
    this.command(['config', '--quiet']);
    this.command(['up', ...(this.sourceBuild ? ['--build'] : []), '-d']);
    this.started = true;
    const origin = `http://localhost:${this.port}`;
    const token = await this.owner(origin);
    this.command(['restart', 'panda']);
    await this.healthy(origin + '/health');
    if ((await (await this.fetchImpl(origin + '/v1/auth/setup')).json()).setup_required)
      throw new Error('compose/docker-compose.yml restart did not preserve the first owner.');
    await this.dashboards(origin, token);
  }
  logs() {
    if (!this.started) return;
    console.error('Published production smoke-test container logs:');
    this.probe('docker', [...this.compose, '--env-file', this.options['production-env'], 'logs', '--no-color', '--tail', '200', 'panda'],
      { cwd: root, stdio: 'inherit', env: this.environment });
  }
  cleanup() {
    if (this.started) this.probe('docker', [...this.compose, '--env-file', this.options['production-env'], 'down', '-v', '--remove-orphans'],
      { cwd: root, stdio: 'inherit', env: this.environment });
    if (!this.dataRoot) return;
    try { rmSync(this.dataRoot, { recursive: true, force: true }); }
    catch (error) {
      if (!['EACCES', 'EPERM'].includes(error?.code)) throw error;
      const result = this.probe('docker', ['run', '--rm', '--user', '0:0', '--mount', `type=bind,source=${this.dataRoot},target=/cleanup`,
        'alpine:3.22.1', 'sh', '-c', 'rm -rf /cleanup/* /cleanup/.[!.]* /cleanup/..?*'], { cwd: root, stdio: 'inherit' });
      if (result.status !== 0) throw error;
      rmSync(this.dataRoot, { recursive: true, force: true });
    }
  }
  async main() {
    try {
      if (this.commands.invoke('git', ['status', '--porcelain'], { capture: true }))
        throw new Error('Fresh-install verification requires a clean worktree. Run it from an isolated worktree or checkout.');
      const modes = { all: ['dev', 'production'], standard: ['production'], production: ['production'], dev: ['dev'] };
      const steps = modes[this.options.mode];
      if (!steps) throw new Error('--mode must be all, standard, production, or dev.');
      for (const step of steps) {
        if (step === 'production') await this.production();
        else this.commands.invoke('pnpm', ['run', 'dev:stack', '--', '--once'], {
          env: { ...this.env, DEV_COMPOSE_PROJECT_NAME: this.project + '-dev' }, shell: process.platform === 'win32' });
      }
      console.log('Fresh-install verification passed.');
    } catch (error) { this.logs(); throw error; }
    finally { this.cleanup(); }
  }
}
if (direct(import.meta.url)) await new FreshInstall(process.argv.slice(2)).main();

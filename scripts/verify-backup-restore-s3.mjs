#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Commands, direct, pollUntil } from './operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const accessKey = 'GKdeadbeef0000000000000000000000';
const secretKey = 'deadbeef0000000000000000000000000000000000000000000000000000cafe';
export class S3RestoreDrill {
  constructor(args = [], { commands = new Commands(root), probe = spawnSync, fetchImpl = fetch, poll = pollUntil } = {}) {
    Object.assign(this, { commands, probe, fetchImpl, poll });
    this.nonce = Date.now().toString(36);
    this.sourceBuild = args.includes('--source-build');
    this.project = 'panda-s3-drill-' + this.nonce;
    this.port = 23900 + Math.floor(Math.random() * 500);
    this.directory = mkdtempSync(join(tmpdir(), 'panda-s3-drill-'));
    this.projects = [];
    this.reportPath = join(root, 'tools/restore-lab/local/reports', `s3-${this.nonce}.json`);
    this.report = { status: 'failed', created_at: new Date().toISOString() };
  }
  compose(project, file) {
    const args = ['compose', '-p', project, '-f', 'compose/docker-compose.yml', '-f', 'compose/docker-compose.backup-drill.yml'];
    if (this.sourceBuild) args.push('-f', 'compose/docker-compose.build.yml');
    return args.concat('--env-file', file);
  }
  garageArgs(...args) { return ['compose', '-p', this.project, '-f', 'compose/docker-compose.dev.yml', ...args]; }
  garageEnvironment() { return { ...process.env, DEV_GARAGE_PORT: String(this.port), DEV_GARAGE_ADMIN_PORT: String(this.port + 3) }; }
  garage() {
    this.commands.invoke('docker', this.garageArgs('up', '-d', '--wait', 'garage'), { env: this.garageEnvironment() });
    const prefix = this.garageArgs('exec', '-T', 'garage', '/garage', '-c', '/config/garage.toml');
    const id = this.commands.invoke('docker', [...prefix, 'node', 'id'], { capture: true }).split(/\s+/)[0];
    const stages = [['layout', 'assign', id, '-z', 'dc1', '-c', '1G'], ['layout', 'apply', '--version', '1'],
      ['key', 'import', '--yes', '-n', 'dev-key', accessKey, secretKey], ['bucket', 'create', 'panda'],
      ['bucket', 'allow', '--read', '--write', '--owner', 'panda', '--key', accessKey]];
    for (const stage of stages) this.commands.invoke('docker', prefix.concat(stage));
  }
  settings() {
    const destination = { endpoint: `http://host.docker.internal:${this.port}`, region: 'garage', bucket: 'panda',
      prefix: 'drill/' + this.nonce, access_key: accessKey, secret_key: secretKey };
    return { enabled: false, frequency: 'daily', run_at: '03:00', timezone: 'UTC', day_of_week: null, day_of_month: null,
      retention_count: 3, local_enabled: true, s3_enabled: true, target_type: 's3', ...destination };
  }
  dataCommand(directory, script) {
    const mount = `type=bind,source=${directory},target=/data`;
    return this.commands.invoke('docker', ['run', '--rm', '--user', '0:0', '--mount', mount, 'alpine:3.22.1', 'sh', '-ceu', script]);
  }
  stack(name, offset) {
    const port = this.port + offset;
    const envFile = join(this.directory, name + '.env');
    const data = { DATABASE_URL: `postgresql://panda:${name}-db-password@timescaledb:5432/panda`, POSTGRES_PASSWORD: name + '-db-password',
      REDIS_PASSWORD: name + '-redis-password', REDIS_URL: `redis://default:${name}-redis-password@redis:6379`,
      ALLOWED_ORIGINS: `http://localhost:${port}`, PANDA_ENV: 'development', S3_ENDPOINT: `http://host.docker.internal:${this.port}`,
      S3_ALLOW_DEVELOPMENT_GARAGE: 'true', COOKIE_INSECURE: 'true', BACKUP_ARTIFACT_DIR: '/backups',
      VEHICLE_IMAGE_CACHE_DIR: '/data/cache/panda/vehicle-images', RUST_LOG: 'panda_api=info,panda_restore_agent=error,tower_http=info' };
    writeFileSync(envFile, Object.entries(data).map(([key, value]) => `${key}=${value}\n`).join(''));
    const directory = join(this.directory, name);
    mkdirSync(directory, { recursive: true });
    this.dataCommand(directory, 'mkdir -p /data/backups /data/cache && (chown 1001:1001 /data/backups /data/cache && chmod 0770 /data/backups /data/cache || chmod 0777 /data/backups /data/cache)');
    const env = { ...process.env, PANDA_DATA_DIR: directory.split('\\').join('/'), PANDA_ENV_FILE: envFile, PANDA_ORIGIN_PORT: String(port) };
    if (this.sourceBuild) env.PANDA_IMAGE = 'panda:local';
    const stack = { project: this.project + '-' + name, envFile, env, directory, origin: `http://localhost:${port}`,
      email: `${name}-owner@example.test`, password: `${name}-${this.nonce}-password` };
    this.projects.push(stack);
    this.commands.invoke('docker', [...this.compose(stack.project, envFile), 'up', ...(this.sourceBuild ? ['--build'] : []), '-d'], { env });
    return stack;
  }
  async health(stack) {
    const url = stack.origin + '/health';
    await this.poll(async () => { try { return (await this.fetchImpl(url)).ok; } catch { return false; } },
      { timeout: 240000, message: `Timed out waiting for ${url}` });
  }
  async request(stack, path, { token, method = 'GET', body } = {}) {
    const headers = {};
    if (token) headers.authorization = `Bearer ${token}`;
    if (body) headers['content-type'] = 'application/json';
    const response = await this.fetchImpl(stack.origin + path, { method, headers, body: body ? JSON.stringify(body) : undefined });
    const text = await response.text();
    const payload = text ? JSON.parse(text) : null;
    if (response.ok) return payload;
    throw new Error(`${method} ${path} failed (${response.status}): ${text}`);
  }
  async authenticate(stack, action, identity = stack) {
    const response = await this.request(stack, '/v1/auth/' + action, { method: 'POST', body: { email: identity.email, password: identity.password } });
    if (!response.access_token) throw new Error((action === 'register' ? 'Registration' : 'Login') + ' did not return an access token.');
    return response.access_token;
  }
  async configure(stack, token, test = false) {
    await this.request(stack, '/v1/admin/backups/settings', { token, method: 'PUT', body: this.settings() });
    if (test) await this.request(stack, '/v1/admin/backups/s3/test', { token, method: 'POST', body: this.settings() });
  }
  async backup(stack, token, id) {
    return this.poll(async () => {
      const overview = await this.request(stack, '/v1/admin/backups?per_page=100', { token });
      const run = overview.recent_runs?.find(row => row.id === id);
      if (['failed', 'canceled'].includes(run?.status)) throw new Error(`Backup run ${id} ${run.status}: ${run.error_message || 'no error message'}`);
      if (run?.status !== 'succeeded') return false;
      const artifacts = overview.artifacts?.filter(row => row.run_id === id) ?? [];
      const stores = new Set(artifacts.map(row => row.storage_type));
      if (!stores.has('local') || !stores.has('s3')) throw new Error(`Backup run ${id} succeeded without both local and S3 artifacts.`);
      return { run, artifacts };
    }, { timeout: 360000, message: `Backup run ${id} did not complete before the timeout.` });
  }
  async symlinkProbe(stack, token, artifact) {
    this.dataCommand(stack.directory, 'mkdir -p /data/cache/workspace-escape; chown 1001:1001 /data/cache/workspace-escape; ln -s /data/cache/workspace-escape /data/backups/.recovery-work');
    try {
      const response = await this.fetchImpl(stack.origin + '/v1/admin/backups/restores/preflight', {
        method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify({ artifact_id: artifact }),
      });
      await response.arrayBuffer();
      if (response.ok) throw new Error('Preflight followed a symlinked recovery workspace.');
      this.dataCommand(stack.directory, 'test -z "$(ls -A /data/cache/workspace-escape)"; test -z "$(ls -A /data/backups/.remote-staging)"');
    } finally { this.dataCommand(stack.directory, 'rm /data/backups/.recovery-work; rmdir /data/cache/workspace-escape'); }
  }
  async restored(stack, started) {
    let phase;
    try {
      await this.poll(async () => {
        let job;
        try {
          const response = await this.fetchImpl(`${stack.origin}/v1/restore-runtime/jobs/${started.job.id}`, {
            headers: { 'x-panda-restore-token': started.capability_token },
          });
          if (!response.ok) return false;
          job = await response.json();
        } catch { return false; }
        phase = job.phase;
        if (phase === 'failed') throw new Error(`Remote restore failed: ${job.error_message || 'restore agent returned no error message'}`);
        return phase === 'completed';
      }, { timeout: 360000, message: 'restore-poll-timeout' });
    } catch (error) {
      if (error.message === 'restore-poll-timeout') throw new Error(`Remote restore did not complete; final phase was ${phase ?? 'unknown'}.`);
      throw error;
    }
  }
  image(stack) {
    const id = this.commands.invoke('docker', [...this.compose(stack.project, stack.envFile), 'ps', '-q', 'panda'], { capture: true, env: stack.env });
    return this.commands.invoke('docker', ['inspect', '--format', '{{.Image}}', id], { capture: true });
  }
  async exercise() {
    this.garage();
    const source = this.stack('source', 1000);
    await this.health(source);
    const sourceToken = await this.authenticate(source, 'register');
    this.dataCommand(source.directory, `mkdir -p /data/cache/panda/vehicle-images/drill && printf %s '${this.nonce}' > /data/cache/panda/vehicle-images/drill/sentinel.txt`);
    await this.configure(source, sourceToken, true);
    const backup = await this.request(source, '/v1/admin/backups/run', { token: sourceToken, method: 'POST' });
    await this.backup(source, sourceToken, backup.run.id);
    this.dataCommand(source.directory, "find /data/backups -type f -name '*.rma.tar.gz' -delete");
    const target = this.stack('target', 2000);
    await this.health(target);
    const targetToken = await this.authenticate(target, 'register');
    await this.configure(target, targetToken);
    const overview = await this.request(target, '/v1/admin/backups', { token: targetToken });
    const remote = overview.artifacts?.find(row => row.storage_type === 's3');
    if (!remote) throw new Error('Clean target did not discover the Garage recovery package.');
    await this.symlinkProbe(target, targetToken, remote.id);
    const preflight = await this.request(target, '/v1/admin/backups/restores/preflight', { token: targetToken, method: 'POST', body: { artifact_id: remote.id } });
    if (!preflight.plan?.compatible) throw new Error(`Remote restore preflight was blocked: ${JSON.stringify(preflight.plan?.blocking_errors ?? [])}`);
    const started = await this.request(target, '/v1/admin/backups/restores', { token: targetToken, method: 'POST', body: {
      artifact_id: remote.id, confirmation_phrase: 'RESTORE', notes: 'Automated S3 drill',
      plan_id: preflight.plan.plan_id, package_checksum_sha256: preflight.plan.package_checksum_sha256,
    } });
    await this.restored(target, started); await this.health(target);
    const old = await this.fetchImpl(target.origin + '/v1/auth/me', { headers: { authorization: `Bearer ${targetToken}` } });
    if (![401, 403].includes(old.status)) throw new Error('The previous target session remained usable after restore.');
    const token = await this.authenticate(target, 'login', source);
    const dashboards = await this.request(target, '/v1/dashboards', { token });
    if (!Array.isArray(dashboards) || dashboards.length < 5) throw new Error('Restored source dashboards were not available.');
    this.dataCommand(target.directory, `test "$(cat /data/cache/panda/vehicle-images/drill/sentinel.txt)" = '${this.nonce}'`);
    const restored = await this.request(target, '/v1/admin/backups', { token });
    if (restored.settings.has_secret_key) throw new Error('The S3 secret key was unexpectedly present after restore.');
    await this.configure(target, token, true);
    const sourceImage = this.image(source), targetImage = this.image(target);
    if (sourceImage !== targetImage) throw new Error('Source and target used different application images.');
    const evidence = ['local_source_removed', 'remote_discovered', 'preflight_compatible', 'workspace_symlink_rejected',
      'in_app_restore_completed', 'previous_session_rejected', 'restored_password_login', 'artwork_restored', 's3_secret_discarded', 's3_reconfigured_and_tested'];
    this.report = { status: 'passed', created_at: new Date().toISOString(), image_id: targetImage,
      package_sha256: preflight.plan.package_checksum_sha256, ...Object.fromEntries(evidence.map(key => [key, true])), restored_dashboard_count: dashboards.length };
    console.log('S3 backup and clean-target restore drill passed.');
  }
  cleanup() {
    for (const item of this.projects.toReversed()) this.probe('docker', [...this.compose(item.project, item.envFile), 'down', '-v', '--remove-orphans'],
      { cwd: root, stdio: 'ignore', env: item.env });
    this.probe('docker', this.garageArgs('down', '-v', '--remove-orphans'), { cwd: root, stdio: 'ignore', env: this.garageEnvironment() });
    const path = relative(resolve(tmpdir()), resolve(this.directory));
    if (!path || path.startsWith('..') || !basename(this.directory).startsWith('panda-s3-drill-')) throw new Error('Refusing cleanup outside the disposable S3 drill directory.');
    const result = this.probe('docker', ['run', '--rm', '--user', '0:0', '--mount', `type=bind,source=${this.directory},target=/cleanup`,
      'alpine:3.22.1', 'sh', '-c', 'rm -rf /cleanup/* /cleanup/.[!.]* /cleanup/..?*'], { cwd: root, stdio: 'ignore' });
    if (result.status === 0) rmSync(this.directory, { recursive: true, force: true });
  }
  async main() {
    try { await this.exercise(); }
    catch (error) {
      for (const item of this.projects) this.probe('docker', [...this.compose(item.project, item.envFile), 'logs', '--no-color', '--tail', '200'],
        { cwd: root, stdio: 'inherit', env: item.env });
      throw error;
    } finally {
      mkdirSync(dirname(this.reportPath), { recursive: true });
      writeFileSync(this.reportPath, JSON.stringify(this.report, null, 2) + '\n');
      console.log(`S3 restore drill report: ${this.reportPath}`);
      this.cleanup();
    }
  }
}
if (direct(import.meta.url)) await new S3RestoreDrill(process.argv.slice(2)).main();

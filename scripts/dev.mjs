import { randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolveBuildVersionFromGit } from './resolve-build-version.mjs';
import { childCompletion, delay, direct, pollUntil } from './operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const services = [
  ['api', 'DEV_API_PORT', 3001, 'API'], ['web', 'DEV_WEB_PORT', 5173, 'Web'],
  ['postgres', 'DEV_POSTGRES_PORT', 5432, 'PostgreSQL', 'timescaledb:5432'],
  ['redis', 'DEV_REDIS_PORT', 6379, 'Redis', 'redis:6379'],
  ['garageApi', 'DEV_GARAGE_PORT', 3900, 'Garage API', 'garage:3900'],
  ['garageAdmin', 'DEV_GARAGE_ADMIN_PORT', 3903, 'Garage admin', 'garage:3903'],
  ['restoreAgent', 'DEV_RESTORE_AGENT_PORT', 3002, 'Restore agent'],
];
const stripAnsi = text => text.replace(/\x1B\[[0-?]*[ -/]*[@-~]/g, '');
const normalizedPath = value => String(value).trim().split('\\').join('/').replace(/\/+/g, '/').toLowerCase();
function integer(value, fallback, lower, upper = Infinity) {
  const parsed = Number.parseInt(value, 10);
  return Number.isInteger(parsed) && parsed >= lower && parsed <= upper ? parsed : fallback;
}
function records(output) {
  const result = [];
  for (const line of output.split(/\r?\n/)) if (line.trim()) result.push(JSON.parse(line.trim()));
  return result;
}
export function deriveDevComposeProjectName(_checkoutRoot, env = process.env) {
  return [env.DEV_COMPOSE_PROJECT_NAME, env.COMPOSE_PROJECT_NAME, 'panda'].find(Boolean);
}
export async function allocateDistinctRuntimePorts(definitions, findPort, reusedPorts = new Map()) {
  const reserved = new Set(reusedPorts.values());
  const values = [];
  const probe = findPort ?? ((...args) => new DevelopmentStack().availablePort(...args));
  for (const definition of definitions) {
    let port = reusedPorts.get(definition.key);
    if (!port) {
      port = await probe(definition.start, definition.label, 50, reserved);
      reserved.add(port);
    }
    values.push([definition.key, port]);
  }
  return Object.fromEntries(values);
}
export function parseComposePortMetadata(output) {
  const result = new Map();
  for (const entry of records(output)) {
    const name = entry.Service ?? entry.service;
    const publishers = entry.Publishers ?? entry.publishers;
    if (!name || !Array.isArray(publishers)) continue;
    const running = String(entry.State ?? entry.state ?? '').toLowerCase() === 'running';
    for (const publisher of publishers) {
      const target = Number.parseInt(publisher.TargetPort ?? publisher.targetPort, 10);
      const port = Number.parseInt(publisher.PublishedPort ?? publisher.publishedPort, 10);
      if ([target, port].every(Number.isInteger)) result.set(name + ':' + target, { port, running });
    }
  }
  return result;
}
function composeFiles(record) {
  let value = record.ConfigFiles ?? record.configFiles ?? record.config_files;
  const labels = record.Labels ?? record.labels;
  if (typeof labels === 'string') {
    const captured = /(?:^|,)com\.docker\.compose\.project\.config_files=([\s\S]*?)(?=,[^=,]+=|$)/i.exec(labels);
    value ??= captured?.[1]?.split(',');
  } else if (value === undefined && labels && typeof labels === 'object') {
    value = labels['com.docker.compose.project.config_files'] ?? labels['com.docker.compose.project.config-files'];
  }
  if (typeof value === 'string') {
    try { value = JSON.parse(value); } catch { /* Labels may contain comma-separated paths. */ }
  }
  if (typeof value === 'string') value = value.split(',');
  const unique = new Set();
  if (Array.isArray(value)) for (const path of value) {
    const normalized = normalizedPath(path);
    if (normalized) unique.add(normalized);
  }
  return Array.from(unique).sort();
}
export function parseComposeIdentityMetadata(output, expectedComposeFile) {
  const expected = normalizedPath(expectedComposeFile);
  const production = expected.replace(/\.dev(?=\.[^/]+$)/, '');
  const identities = records(output).map(record => ({ service: record.Service ?? record.service, composeFiles: composeFiles(record) }));
  const signatures = new Set();
  let missing = false, hasExpected = identities.length > 0, hasProduction = false;
  for (const identity of identities) {
    const files = identity.composeFiles;
    signatures.add(files.join('\n'));
    missing ||= files.length === 0;
    hasExpected &&= files.includes(expected);
    hasProduction ||= files.includes(production);
  }
  const reasons = [[missing, 'missing identity metadata'], [signatures.size > 1, 'mixed Compose config files'],
    [hasExpected && hasProduction, 'production Compose config file'], [!hasExpected, 'unexpected Compose config file']];
  return { records: identities, composeFiles: identities[0]?.composeFiles ?? [], hasExistingRecords: identities.length > 0,
    valid: identities.length === 0 || (!missing && signatures.size <= 1 && hasExpected && !hasProduction),
    reason: reasons.find(([active]) => active)?.[1] ?? null };
}
export function extractViteLocalUrl(output) {
  const clean = stripAnsi(output);
  for (const pattern of [/Local:\s+https?:\/\/localhost:(\d+)(?:\/|$)/i, /http:\/\/localhost:(\d+)(?:\/|$)/i]) {
    const result = pattern.exec(clean);
    if (result) return 'http://localhost:' + result[1];
  }
  return null;
}
export function parsePids(output, ownPid = process.pid) {
  const unique = new Set();
  for (const text of output.split(/\s+/)) {
    const pid = Number.parseInt(text, 10);
    if (Number.isInteger(pid) && pid > 0 && pid !== ownPid) unique.add(pid);
  }
  return Array.from(unique);
}

export class DevelopmentStack {
  constructor({ env = process.env, platform = process.platform, launch = spawn, log = console.log } = {}) {
    Object.assign(this, { env, launch, log });
    this.windows = platform === 'win32';
    this.apiDir = resolve(root, 'apps/api');
    this.webDir = resolve(root, 'apps/web');
    this.composeFile = resolve(root, 'compose/docker-compose.dev.yml');
    this.keyFile = resolve(root, 'data/dev-restore-agent-key');
    this.markerFile = resolve(root, 'data/dev-restore-in-progress');
    this.pidFile = resolve(root, 'data/dev-api.pid');
    this.project = deriveDevComposeProjectName(root, env);
    this.ports = Object.fromEntries(services.map(([key, variable, fallback]) => [key, integer(env[variable], fallback, 1, 65535)]));
    this.dbTimeout = integer(env.DEV_DATABASE_READY_TIMEOUT_SECONDS, 600, 60) * 1000;
    this.jobs = this.windows ? integer(env.DEV_CARGO_BUILD_JOBS, 4, 1) : null;
    this.children = new Set();
    this.stopping = false;
  }
  binary(name) { return resolve(this.apiDir, 'target/debug', name + (this.windows ? '.exe' : '')); }
  origins() { return Array.from({ length: 11 }, (_, offset) => `http://localhost:${this.ports.web + offset}`); }
  health() { return `http://localhost:${this.ports.api}/health`; }
  start(command, args, options = {}) {
    const configuration = { cwd: options.cwd ?? root, env: { ...this.env, ...options.env },
      stdio: options.stdio ?? 'inherit', shell: options.shell ?? false, windowsHide: true };
    const child = this.launch(command, args, configuration);
    if (options.track !== false) {
      this.children.add(child);
      child.once('exit', () => this.children.delete(child));
    }
    return child;
  }
  async run(command, args, options = {}) {
    const result = await childCompletion(this.start(command, args, { ...options, track: false }));
    if (result.code !== 0) throw new Error(`${command} ${args.join(' ')} failed with ${result.signal ? `signal ${result.signal}` : `exit code ${result.code}`}`);
  }
  async capture(command, args, options = {}) {
    const child = this.start(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'], track: false });
    const output = { stdout: '', stderr: '' };
    for (const stream of ['stdout', 'stderr']) child[stream]?.on('data', data => { output[stream] += data.toString(); });
    const { code } = await childCompletion(child);
    return { code, ...output };
  }
  async tools() {
    for (const tool of ['pnpm', 'docker', 'curl']) {
      const result = await this.capture(this.windows ? 'where' : 'command', this.windows ? [tool] : ['-v', tool], { shell: !this.windows });
      if (result.code !== 0) throw new Error(`Missing required tool: ${tool}`);
    }
  }
  async dependencies() {
    this.log('Installing workspace dependencies...');
    try { await this.run('pnpm', ['install', '--frozen-lockfile'], { shell: this.windows }); }
    catch {
      this.log('Lockfile needed an update; retrying install without frozen lockfile.');
      await this.run('pnpm', ['install', '--no-frozen-lockfile'], { shell: this.windows });
    }
  }
  async ownedProcesses() {
    const api = this.binary('panda-api'), agent = this.binary('panda-restore-agent');
    if (!this.windows) {
      const response = await this.capture('ps', ['-axo', 'pid=,command=']);
      if (response.code !== 0) {
        this.log(`[dev] Could not inspect existing processes; leaving occupied ports alone. ${stripAnsi(response.stderr).trim()}`);
        return [];
      }
      const found = [];
      for (const line of response.stdout.split(/\r?\n/)) {
        const match = /^(\d+)\s+(.+)$/.exec(line.trim());
        if (!match) continue;
        const command = match[2];
        let kind, name;
        if (command.includes(api)) [kind, name] = ['api', 'panda-api'];
        else if (command.includes(agent)) [kind, name] = ['restore-agent', 'panda-restore-agent'];
        else if (command.includes(this.webDir) && /\bvite(?:\.js)?\b/i.test(command)) [kind, name] = ['vite', 'vite'];
        if (kind) found.push({ pid: Number.parseInt(match[1], 10), name, kind });
      }
      return found;
    }
    const script = 'Get-CimInstance Win32_Process -ErrorAction Stop | Select-Object ProcessId, Name, ExecutablePath, CommandLine | ConvertTo-Json -Compress';
    const response = await this.capture('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script]);
    if (response.code !== 0) {
      this.log(`[dev] Could not inspect existing Windows processes; leaving occupied ports alone. ${stripAnsi(response.stderr).trim()}`);
      return [];
    }
    try {
      if (!response.stdout.trim()) return [];
      const parsed = JSON.parse(response.stdout.trim());
      const rows = Array.isArray(parsed) ? parsed : [parsed];
      const kinds = new Map([[api.toLowerCase(), 'api'], [agent.toLowerCase(), 'restore-agent']]);
      const found = [];
      for (const row of rows) {
        if (!Number.isInteger(row.ProcessId) || row.ProcessId <= 0) continue;
        let kind = kinds.get(String(row.ExecutablePath ?? '').toLowerCase());
        const command = String(row.CommandLine ?? '');
        if (!kind && command.toLowerCase().includes(this.webDir.toLowerCase()) && /\bvite(?:\.cmd|\.js)?\b/i.test(command))
          kind = 'vite';
        if (kind) found.push({ pid: row.ProcessId, name: row.Name, kind });
      }
      return found;
    } catch (error) {
      this.log(`[dev] Could not parse existing Windows process details; leaving occupied ports alone. ${error.message}`);
      return [];
    }
  }
  async clearOwnedProcesses() {
    const found = await this.ownedProcesses();
    if (!found.length) return;
    const ordered = found.filter(row => row.kind === 'vite').concat(found.filter(row => row.kind !== 'vite'));
    this.log(`[dev] Clearing ${ordered.length} stale Panda dev process(es): ${ordered.map(row => `${row.name} (${row.pid})`).join(', ')}`);
    for (const row of ordered) {
      const command = this.windows ? ['taskkill', ['/PID', String(row.pid), '/T', '/F']] : ['kill', ['-TERM', String(row.pid)]];
      const result = await this.capture(...command);
      if (result.code !== 0) this.log(`[dev] Could not stop stale ${row.name} (${row.pid}): ${stripAnsi(result.stderr).trim()}`);
    }
    await delay(500);
  }
  async listeners(port) {
    if (this.windows) {
      const script = `Get-NetTCPConnection -LocalPort ${port} -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess -Unique`;
      return parsePids((await this.capture('powershell.exe', ['-NoProfile', '-Command', script], { shell: false })).stdout);
    }
    const first = await this.capture('lsof', ['-ti', `tcp:${port}`], { shell: false });
    if (first.code === 0) return parsePids(first.stdout);
    const script = `ss -ltnp 'sport = :${port}' 2>/dev/null | sed -n 's/.*pid=\\([0-9][0-9]*\\).*/\\1/p' | sort -u`;
    return parsePids((await this.capture('sh', ['-c', script], { shell: false })).stdout);
  }
  async availablePort(start, label, tries = 50, reserved = new Set()) {
    let port = start;
    while (port < start + tries) {
      const reservedPort = reserved.has(port);
      if (!reservedPort && (await this.listeners(port)).length === 0) {
        if (port > start) this.log(`[dev] ${label} port ${start} was busy, using ${port}`);
        return port;
      }
      this.log(`[dev] ${label} port ${port} is ${reservedPort ? 'already reserved' : 'in use'}; trying ${port + 1}`);
      port += 1;
    }
    throw new Error(`[dev] Could not find an available ${label} port after ${tries} attempts from ${start}.`);
  }
  async allocatePorts() {
    const result = await this.capture('docker', ['compose', '-p', this.project, '-f', this.composeFile, 'ps', '--all', '--format', 'json']);
    if (result.code !== 0) throw new Error('[dev] Could not verify the selected project before startup; set DEV_COMPOSE_PROJECT_NAME to a unique isolated name and retry.');
    const reused = new Map();
    if (result.stdout.trim()) {
      const identity = parseComposeIdentityMetadata(result.stdout, this.composeFile);
      if (!identity.valid) throw new Error(`[dev] The selected project is not the expected development project (${identity.reason}); set DEV_COMPOSE_PROJECT_NAME to a unique isolated name before starting.`);
      const metadata = parseComposePortMetadata(result.stdout);
      for (const [key, , , , address] of services) {
        const existing = metadata.get(address);
        if (existing?.running) reused.set(key, existing.port);
      }
    }
    const definitions = services.map(([key, , , label]) => ({ key, start: this.ports[key], label }));
    this.ports = await allocateDistinctRuntimePorts(definitions, this.availablePort.bind(this), reused);
    for (const [key, variable] of services) this.env[variable] = String(this.ports[key]);
    this.env.DEV_WEB_ORIGINS = this.origins().join(',');
    this.env.COMPOSE_PROJECT_NAME = this.project;
  }
  async assertPort(port, label) {
    const pids = await this.listeners(port);
    if (!pids.length) return;
    throw new Error(`${label} port ${port} became occupied before startup by PID(s): ${pids.join(', ')}`);
  }
  async waitHttp(url, child, label, timeout = 120000) {
    await pollUntil(async () => {
      if (child && child.exitCode !== null) throw new Error(`${label} exited before it became ready.`);
      const response = await this.capture('curl', ['-fsS', '--max-time', '2', url]);
      if (response.code !== 0) return false;
      if (child && child.exitCode !== null) throw new Error(`${label} health check was answered by another process; the managed process exited.`);
      return true;
    }, { timeout, interval: 500, message: `Timed out waiting for ${label} at ${url}` });
  }
  async garage() {
    const prefix = ['compose', '-f', this.composeFile, 'exec', '-T', 'garage', '/garage', '-c', '/config/garage.toml'];
    let response;
    for (let remaining = 60; remaining > 0; remaining -= 1) {
      response = await this.capture('docker', [...prefix, 'status']);
      if (response.code === 0) break;
      if (remaining === 1) throw new Error(`Garage did not become ready.\n${stripAnsi(response.stderr).trim()}`);
      await delay(500);
    }
    if ((await this.capture('docker', [...prefix, 'key', 'list'])).code === 0) return;
    const node = await this.capture('docker', [...prefix, 'node', 'id']);
    if (node.code !== 0 || !node.stdout.trim()) throw new Error(`Could not read the Garage node id.\n${stripAnsi(node.stderr).trim()}`);
    const steps = [
      ['layout', 'assign', node.stdout.trim().split(/\s+/)[0], '-z', 'dc1', '-c', '1G'], ['layout', 'apply', '--version', '1'],
      ['key', 'import', '--yes', '-n', 'dev-key', 'GKdeadbeef0000000000000000000000', 'deadbeef0000000000000000000000000000000000000000000000000000cafe'],
      ['bucket', 'create', 'panda'], ['bucket', 'allow', '--read', '--write', '--owner', 'panda', '--key', 'GKdeadbeef0000000000000000000000'],
    ];
    for (const args of steps) await this.run('docker', prefix.concat(args));
  }
  async infrastructure() {
    this.log(`Starting infrastructure (TimescaleDB, Redis, Garage) for ${this.project}...`);
    const compose = ['compose', '-f', this.composeFile];
    await this.run('docker', [...compose, 'up', '-d', 'timescaledb', 'redis', 'garage']);
    await this.garage();
    const deadline = Date.now() + this.dbTimeout;
    let nextLog = Date.now() + 15000;
    await pollUntil(async () => {
      const id = await this.capture('docker', [...compose, 'ps', '-q', 'timescaledb']);
      if (id.code !== 0 || !id.stdout.trim()) throw new Error('TimescaleDB container was not created. Inspect docker compose logs timescaledb.');
      const status = await this.capture('docker', ['inspect', '--format', '{{.State.Status}}', id.stdout.trim()]);
      if (status.code === 0 && ['exited', 'dead'].includes(status.stdout.trim())) {
        const logs = await this.capture('docker', [...compose, 'logs', '--tail', '40', 'timescaledb']);
        throw new Error(`TimescaleDB container stopped during startup (${status.stdout.trim()}).\n${stripAnsi(logs.stdout).trim()}`);
      }
      if ((await this.capture('docker', [...compose, 'exec', '-T', 'timescaledb', 'pg_isready', '-U', 'panda'])).code === 0) return true;
      if (Date.now() >= nextLog) {
        this.log(`TimescaleDB is still recovering or starting; waiting up to ${Math.ceil((deadline - Date.now()) / 1000)}s more...`);
        nextLog += 15000;
      }
      return false;
    }, { timeout: this.dbTimeout, message: `Timed out waiting for TimescaleDB to accept connections after ${this.dbTimeout / 1000}s. Inspect docker compose logs timescaledb for recovery errors.` });
    for (const line of ['', 'Infrastructure is running', `   TimescaleDB: postgresql://localhost:${this.ports.postgres}`,
      `   Redis: redis://localhost:${this.ports.redis}`, `   S3 (Garage): http://localhost:${this.ports.garageApi}`, '']) this.log(line);
  }
  async ensureKey() {
    await mkdir(dirname(this.keyFile), { recursive: true });
    await rm(this.markerFile, { force: true });
    let key;
    try { key = (await readFile(this.keyFile, 'utf8')).trim(); } catch { /* First startup has no key. */ }
    if (!key) await writeFile(this.keyFile, randomBytes(32).toString('base64') + '\n', { encoding: 'utf8', mode: 0o600 });
  }
  apiEnvironment() {
    const local = { postgres: `postgresql://panda:devpassword@localhost:${this.ports.postgres}/panda?options=-c%20search_path%3Dpanda,timeseries,public`,
      redis: `redis://localhost:${this.ports.redis}`, garage: `http://localhost:${this.ports.garageApi}` };
    return { SQLX_OFFLINE: 'true', DATABASE_URL: local.postgres, REDIS_URL: local.redis, S3_ENDPOINT: local.garage,
      S3_ALLOW_DEVELOPMENT_GARAGE: 'true', S3_ACCESS_KEY: 'GKdeadbeef0000000000000000000000',
      S3_SECRET_KEY: 'deadbeef0000000000000000000000000000000000000000000000000000cafe',
      PORT: String(this.ports.api), ALLOWED_ORIGINS: this.origins().join(','), COOKIE_INSECURE: 'true', PANDA_ENV: 'development',
      PANDA_BUILD_VERSION: resolveBuildVersionFromGit(), RESTORE_AGENT_URL: `http://127.0.0.1:${this.ports.restoreAgent}`,
      RESTORE_AGENT_KEY_FILE: this.keyFile, RESTORE_AGENT_PORT: String(this.ports.restoreAgent),
      PANDA_API_PID_FILE: this.pidFile, PANDA_RESTORE_MARKER_FILE: this.markerFile };
  }
  async launchApi() {
    await this.assertPort(this.ports.api, 'API');
    this.log('Starting API...');
    const child = this.start(this.binary('panda-api'), [], { cwd: this.apiDir, env: this.apiEnvironment() });
    await writeFile(this.pidFile, child.pid + '\n', 'utf8');
    await this.waitHttp(this.health(), child, 'API');
    this.log(`API is responding at ${this.health()}`);
    return child;
  }
  async api() {
    await this.assertPort(this.ports.api, 'API');
    const args = ['build', '--bin', 'panda-api', '--bin', 'panda-restore-agent'];
    if (this.jobs !== null) args.push('--jobs', String(this.jobs));
    this.log(`Building API targets${this.jobs !== null ? ` (maximum ${this.jobs} concurrent Cargo jobs)` : ''}...`);
    await this.run('cargo', args, { cwd: this.apiDir, env: this.apiEnvironment() });
    await this.ensureKey();
    await this.assertPort(this.ports.restoreAgent, 'Restore agent');
    this.log('Starting restore supervisor...');
    const supervisor = this.start(this.binary('panda-restore-agent'), [], { cwd: this.apiDir, env: this.apiEnvironment() });
    const address = `http://127.0.0.1:${this.ports.restoreAgent}`;
    await this.waitHttp(address + '/health', supervisor, 'restore supervisor');
    this.log(`Restore supervisor is responding at ${address}`);
    return this.launchApi();
  }
  unexpectedExit(child, label) {
    return new Promise((_, reject) => child.once('exit', (code, signal) => {
      reject(new Error(`${label} exited unexpectedly (${signal ?? `exit code ${code}`}).`));
    }));
  }
  async supervise(child) {
    let current = child;
    while (!this.stopping) {
      try { await this.unexpectedExit(current, 'API'); return; }
      catch (error) { if (this.stopping) return; this.log(error.message); }
      for (;;) {
        if (this.stopping) return;
        try { await readFile(this.markerFile, 'utf8'); } catch { break; }
        this.log('API stopped for an in-app restore; waiting for the restore supervisor.');
        await delay(500);
      }
      if (this.stopping) return;
      this.log('Restarting API...');
      try { current = await this.launchApi(); }
      catch (error) { throw new Error(`API restart failed: ${error.message}`); }
    }
  }
  viteUrl(child, timeout = 30000) {
    return new Promise((accept, reject) => {
      let output = '';
      const finish = (error, url) => {
        clearTimeout(timer);
        for (const stream of ['stdout', 'stderr']) child[stream]?.off('data', data);
        child.off('exit', exit);
        if (error) reject(error); else accept(url);
      };
      const data = chunk => { output += chunk.toString(); const url = extractViteLocalUrl(output); if (url) finish(null, url); };
      const exit = (code, signal) => finish(new Error(`web dev server exited before announcing a URL (${signal ? `signal ${signal}` : `exit code ${code}`}).`));
      const timer = setTimeout(() => {
        child.kill(this.windows ? undefined : 'SIGTERM');
        finish(new Error('Timed out waiting for Vite to announce a local URL.'));
      }, timeout);
      for (const stream of ['stdout', 'stderr']) child[stream]?.on('data', data);
      child.once('exit', exit);
    });
  }
  async web() {
    this.log('Starting web dev server...');
    const env = { VITE_RESTORE_AGENT_URL: `http://127.0.0.1:${this.ports.restoreAgent}` };
    for (const key of ['VITE_WS_URL', 'VITE_API_URL', 'VITE_PANDA_API_BASE_URL']) env[key] = this.env[key] ?? `http://localhost:${this.ports.api}`;
    const child = this.start(resolve(this.webDir, 'node_modules/.bin', this.windows ? 'vite.cmd' : 'vite'),
      ['--port', String(this.ports.web)], { cwd: this.webDir, env, stdio: ['ignore', 'pipe', 'pipe'], shell: this.windows });
    for (const stream of ['stdout', 'stderr']) child[stream]?.on('data', data => process[stream].write(data));
    const url = await this.viteUrl(child);
    await this.waitHttp(url, child, 'web dev server');
    this.log(`Web dev server is responding at ${url}`);
    return { child, url };
  }
  shutdown(code = 0) {
    if (this.stopping) return;
    this.stopping = true;
    for (const child of Array.from(this.children))
      if (child.exitCode === null && !child.killed) child.kill(this.windows ? undefined : 'SIGTERM');
    process.exit(code);
  }
  async main(once) {
    this.log('Starting Panda development stack...'); this.log('');
    for (const step of [this.tools, this.clearOwnedProcesses, this.dependencies, this.allocatePorts, this.infrastructure]) await step.call(this);
    for (const line of ['Starting local dev servers...', `   API:  ${this.health().replace('/health', '')}`,
      `   Web:  http://localhost:${this.ports.web} (will advance if blocked)`, '']) this.log(line);
    const supervisor = this.supervise(await this.api());
    this.log('To view infra logs in another terminal, run:');
    this.log(`   docker compose -p "${this.project}" -f "${this.composeFile}" logs -f`); this.log('');
    const web = await this.web();
    if (once) {
      this.log('Startup smoke test passed; shutting down managed dev servers.');
      this.shutdown(0);
      return;
    }
    await Promise.race([supervisor, this.unexpectedExit(web.child, 'Web dev server')]);
  }
}
if (direct(import.meta.url)) {
  const stack = new DevelopmentStack();
  for (const [signal, code] of [['SIGINT', 130], ['SIGTERM', 143]]) process.on(signal, () => stack.shutdown(code));
  stack.main(process.argv.includes('--once')).catch(error => { console.error(error.message); stack.shutdown(1); });
}

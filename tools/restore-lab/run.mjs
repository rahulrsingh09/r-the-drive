#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Commands, direct, option, pollUntil } from '../../scripts/operational-runtime.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const checksum = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const now = () => new Date().toISOString();
export function assertManifestExpectations(manifest, checkpoint) {
  for (const [path, expected] of Object.entries(checkpoint?.manifest_expectations ?? {})) {
    let actual = manifest;
    for (const component of path.split('.')) actual = actual?.[component];
    if (JSON.stringify(actual) === JSON.stringify(expected)) continue;
    throw new Error(`Release checkpoint ${checkpoint.id} expected manifest ${path}=${JSON.stringify(expected)}, received ${JSON.stringify(actual)}.`);
  }
}
export function manifestLedgerVersions(manifest) {
  const ledger = manifest.source?.migration_ledger;
  if (!Array.isArray(ledger) || ledger.length === 0) return [];
  const versions = [];
  for (const entry of ledger) {
    if (!Number.isInteger(entry.version) || entry.version <= 0) throw new Error('Recovery manifest migration ledger contains an invalid version.');
    versions.push(entry.version);
  }
  for (const [index, version] of versions.entries())
    if (index > 0 && version - versions[index - 1] !== 1) throw new Error('Recovery manifest migration ledger is not contiguous.');
  const declared = manifest.source?.migration_version;
  if (declared !== undefined && declared !== versions.at(-1)) throw new Error('Recovery manifest migration head does not match its ledger.');
  return versions;
}
export function targetLedgerVersions(report, manifest) {
  const candidates = [report.plan?.target?.migration_ledger, report.validation_report?.target_profile?.migration_ledger,
    report.validation_report?.target_migration_ledger];
  const selected = candidates.find(value => value !== null && value !== undefined);
  if (!Array.isArray(selected) || selected.length === 0) return manifestLedgerVersions(manifest);
  return Array.from(selected, row => row.version ?? row);
}
export function verifyDatabase(manifest, restoreReport, sql) {
  const tables = [
    ['users', 'panda.users'], ['vehicles', 'panda.vehicles'], ['dashboards', 'panda.dashboards'], ['trips', 'panda.trips'],
    ['charge_sessions', 'panda.charge_sessions'], ['backup_runs', 'panda.backup_runs'], ['backup_artifacts', 'panda.backup_artifacts'],
    ['restore_requests', 'panda.backup_restore_requests'], ['telemetry', 'timeseries.telemetry'],
  ];
  const relations = {};
  for (const [, table] of tables) relations[table] = sql(`SELECT to_regclass('${table}') IS NOT NULL`) === 't';
  const missing = Object.keys(relations).filter(name => !relations[name]);
  if (missing.length) throw new Error(`Restored database is missing required relations: ${missing.join(', ')}`);
  const counts = {};
  const ordered = tables.slice(0, 5).concat(tables.slice(8), tables.slice(5, 8));
  for (const [name, table] of ordered) counts[name] = Number.parseInt(sql(`SELECT count(*) FROM ${table}`), 10);
  const actual = sql('SELECT version FROM public._sqlx_migrations WHERE success = TRUE ORDER BY version').split(/\r?\n/).filter(Boolean).map(Number);
  const expected = targetLedgerVersions(restoreReport, manifest);
  if (actual.join(',') !== expected.join(',')) throw new Error(`Expected migration ledger ${expected.join(',') || '(none)'}, received ${actual.join(',') || '(none)'}.`);
  for (const name of ['users', 'vehicles', 'dashboards', 'telemetry', 'trips', 'charge_sessions'])
    if (counts[name] < 1) throw new Error(`Restored ${name} data is empty.`);
  for (const name of ['backup_runs', 'backup_artifacts', 'restore_requests'])
    if (counts[name] < 1) throw new Error(`Restored operational history ${name} is empty.`);
  const hypertable = sql("SELECT EXISTS (SELECT 1 FROM timescaledb_information.hypertables WHERE hypertable_schema = 'timeseries' AND hypertable_name = 'telemetry')") === 't';
  if (!hypertable) throw new Error('timeseries.telemetry is not a TimescaleDB hypertable.');
  return { relations, counts, migration_rows: actual, telemetry_hypertable: hypertable };
}

export class RestoreLab {
  constructor(args, { commands, probe = spawnSync, fetchImpl = fetch, poll = pollUntil } = {}) {
    const supplied = option(args, '--package');
    if (!supplied) throw new Error('--package is required.');
    this.packagePath = resolve(supplied);
    if (!existsSync(this.packagePath)) throw new Error(`Recovery package does not exist: ${this.packagePath}`);
    this.flags = new Set(args);
    Object.assign(this, { probe, fetchImpl, poll });
    this.nonce = `${Date.now().toString(36)}-${process.pid}`;
    this.project = 'panda-restore-lab-' + this.nonce;
    this.port = String(19080 + Math.floor(Math.random() * 800));
    this.directory = mkdtempSync(join(tmpdir(), 'panda-restore-lab-'));
    this.dataRoot = join(this.directory, 'data');
    this.copiedPackage = join(this.directory, basename(this.packagePath));
    this.envFile = join(this.directory, 'restore.env');
    this.reportPath = join(root, 'tools/restore-lab/local/reports', this.nonce + '.json');
    this.environment = { ...process.env, PANDA_DATA_DIR: this.dataRoot.split('\\').join('/'), PANDA_ENV_FILE: this.envFile, PANDA_ORIGIN_PORT: this.port };
    this.commands = commands ?? new Commands(root, this.environment);
    this.compose = ['compose', '-p', this.project, '--env-file', this.envFile, '-f', 'compose/docker-compose.yml'];
    if (this.flags.has('--source-build')) this.compose.push('-f', 'compose/docker-compose.build.yml');
  }
  docker(args, options = {}) { return this.commands.invoke('docker', this.compose.concat(args), options); }
  sql(query) {
    return this.docker(['exec', '-T', 'timescaledb', 'psql', '-U', 'panda', '-d', 'panda', '-At', '-v', 'ON_ERROR_STOP=1', '-c', query], { capture: true });
  }
  async fetchJson(path) {
    const response = await this.fetchImpl(`http://localhost:${this.port}${path}`);
    const text = await response.text();
    if (!response.ok) throw new Error(`GET ${path} failed (${response.status}): ${text}`);
    return text.length ? JSON.parse(text) : null;
  }
  initialize() {
    const registry = JSON.parse(readFileSync(join(root, 'tools/restore-lab/fixtures.json'), 'utf8'));
    copyFileSync(this.packagePath, this.copiedPackage);
    this.originalChecksum = checksum(this.packagePath);
    this.checkpoint = registry.release_checkpoints?.find(row => row.sha256 === this.originalChecksum) ?? null;
    if (this.checkpoint && this.checkpoint.file_name !== basename(this.packagePath))
      throw new Error(`Release checkpoint ${this.checkpoint.id} does not match the supplied package filename.`);
    if (checksum(this.copiedPackage) !== this.originalChecksum) throw new Error('Local package copy checksum mismatch.');
    const postgres = 'restore-db-' + this.nonce, redis = 'restore-redis-' + this.nonce;
    const env = { DATABASE_URL: `postgresql://panda:${postgres}@timescaledb:5432/panda`, POSTGRES_PASSWORD: postgres, REDIS_PASSWORD: redis,
      REDIS_URL: `redis://default:${redis}@redis:6379`, ALLOWED_ORIGINS: `http://localhost:${this.port}`, PANDA_ORIGIN_PORT: this.port,
      COOKIE_INSECURE: 'true', PANDA_ENV: 'development', BACKUP_ARTIFACT_DIR: '/backups',
      VEHICLE_IMAGE_CACHE_DIR: '/data/cache/panda/vehicle-images', RUST_LOG: 'panda_api=info,tower_http=info' };
    writeFileSync(this.envFile, Object.entries(env).map(([key, value]) => `${key}=${value}\n`).join(''));
    this.sentinel = join(this.dataRoot, 'backups/restore-lab-host-catalog-sentinel');
    mkdirSync(dirname(this.sentinel), { recursive: true });
    writeFileSync(this.sentinel, 'host backup directory must survive restore\n');
  }
  reportPackage(manifest) {
    return { file_name: basename(this.packagePath), sha256: this.originalChecksum, format: manifest.format,
      format_version: manifest.format_version, source: manifest.source, release_checkpoint_id: this.checkpoint?.id ?? null };
  }
  async rollback() {
    const email = `restore-rollback-${this.nonce}@example.test`;
    const artwork = join(this.dataRoot, 'cache/panda/vehicle-images/rollback-probe.txt');
    const text = 'previous artwork must survive rollback\n';
    this.sql(`INSERT INTO panda.users(email,is_disabled) VALUES('${email}',TRUE)`);
    writeFileSync(artwork, text);
    const engine = (action, extra = []) => {
      const args = ['run', '--rm', '--no-deps', '--entrypoint', '/app/panda-restore-agent'];
      if (action === 'host-restore') args.push('-v', `${this.copiedPackage}:/restore-package:ro`);
      this.docker(args.concat('panda', action, extra));
    };
    engine('host-restore', ['/restore-package', '--force']);
    this.docker(['stop', 'panda']); engine('host-activate');
    const count = () => this.sql(`SELECT count(*) FROM panda.users WHERE email='${email}'`);
    if (count() !== '0') throw new Error('Rollback probe remained in the activated candidate database.');
    if (existsSync(artwork)) throw new Error('Rollback probe remained in the activated candidate artwork.');
    engine('host-rollback');
    if (count() !== '1') throw new Error('Rollback did not restore the previous database.');
    if (!existsSync(artwork) || readFileSync(artwork, 'utf8') !== text) throw new Error('Rollback did not restore the previous artwork.');
    if (existsSync(join(this.dataRoot, 'backups/.host-restore-state.json'))) throw new Error('Rollback left an unfinished host restore state.');
    const reports = join(this.dataRoot, 'backups/.restore-reports');
    const rolledBack = readdirSync(reports).filter(file => file.endsWith('.json')).map(file => JSON.parse(readFileSync(join(reports, file), 'utf8')))
      .some(report => report.status === 'rolled_back');
    if (!rolledBack) throw new Error('Rollback did not write its durable report.');
    this.sql(`DELETE FROM panda.users WHERE email='${email}'`); rmSync(artwork);
    this.docker(['up', '-d', 'panda']);
    await this.poll(async () => {
      try {
        const response = await this.fetchImpl(`http://localhost:${this.port}/health`, { signal: AbortSignal.timeout(5000) });
        const body = await response.text(); return response.ok && body.trim() === 'ok';
      } catch { return false; }
    }, { timeout: 180000, message: 'Application did not become healthy after rollback.' });
    console.log('Rollback verified: previous database and artwork restored; application healthy.');
    return { database_restored: true, artwork_restored: true, durable_report: true, application_healthy: true };
  }
  async exercise() {
    const manifest = JSON.parse(this.commands.invoke('tar', ['-xOzf', this.copiedPackage, 'manifest.json'], { capture: true }));
    assertManifestExpectations(manifest, this.checkpoint);
    const versions = manifestLedgerVersions(manifest);
    const args = ['scripts/restore-backup.mjs', '--package', this.copiedPackage, '--env-file', this.envFile, '--project', this.project, '--force'];
    if (this.flags.has('--source-build')) args.push('--source-build');
    if (this.flags.has('--reuse-image')) args.push('--skip-build');
    const common = { status: 'passed', created_at: now(), package: this.reportPackage(manifest), source_migration_versions: versions };
    if (this.checkpoint?.expected_outcome === 'unsupported_migration_chain') {
      const response = this.probe(process.execPath, args, { cwd: root, encoding: 'utf8', env: this.environment });
      const output = `${response.stdout ?? ''}\n${response.stderr ?? ''}`;
      if (response.status === 0) throw new Error(`Release checkpoint ${this.checkpoint.id} was restored, but it must be unsupported.`);
      if (!/unsupported[_ ]migration[_ ]chain/i.test(output))
        throw new Error(`Release checkpoint ${this.checkpoint.id} failed without the expected unsupported_migration_chain reason: ${output.trim()}`);
      return { ...common, expected_outcome: this.checkpoint.expected_outcome, rejection_output: output.trim() };
    }
    this.commands.invoke(process.execPath, args);
    const setup = await this.fetchJson('/v1/auth/setup');
    if (setup.setup_required) throw new Error('Restored application unexpectedly requires owner setup.');
    this.docker(['exec', '-T', 'panda', 'curl', '-fsS', 'http://127.0.0.1:3002/health']);
    const directory = join(this.dataRoot, 'backups/.restore-reports');
    const files = existsSync(directory) ? readdirSync(directory).filter(file => file.endsWith('.json')) : [];
    if (files.length !== 1) throw new Error(`Expected one durable host restore report, found ${files.length}.`);
    const report = JSON.parse(readFileSync(join(directory, files[0]), 'utf8'));
    if (report.status !== 'completed') throw new Error(`Host restore report has unexpected status ${report.status}.`);
    const database = verifyDatabase(manifest, report, this.sql.bind(this));
    if (!existsSync(this.sentinel)) throw new Error('Restore replaced or removed the host backup directory.');
    const artworkRoot = join(this.dataRoot, 'cache/panda/vehicle-images');
    const artwork = existsSync(artworkRoot) ? readdirSync(artworkRoot, { recursive: true, withFileTypes: true }).filter(entry => entry.isFile()).length : 0;
    if (artwork < 1) throw new Error('Restored vehicle artwork is missing.');
    if (checksum(this.packagePath) !== this.originalChecksum) throw new Error('Source recovery package changed during the lab run.');
    const rollback = this.flags.has('--verify-rollback') ? await this.rollback() : null;
    const container = this.docker(['ps', '-q', 'panda'], { capture: true });
    const image = this.commands.invoke('docker', ['inspect', '--format', '{{.Image}}', container], { capture: true });
    return { ...common, target: { source_build: this.flags.has('--source-build'), image_id: image, origin: `http://localhost:${this.port}` },
      setup_required: setup.setup_required, restore_supervisor_healthy: true, host_backup_directory_preserved: true,
      artwork_files: artwork, database, rollback, restore_plan: report.plan, validation_report: report.validation_report };
  }
  cleanup() {
    if (this.flags.has('--keep')) { console.log(`Restore lab retained: project=${this.project} data=${this.dataRoot}`); return; }
    this.probe('docker', [...this.compose, 'down', '-v', '--remove-orphans'], { cwd: root, stdio: 'ignore', env: this.environment });
    const path = relative(resolve(tmpdir()), resolve(this.directory));
    if (!path || path.startsWith('..') || !basename(this.directory).startsWith('panda-restore-lab-'))
      throw new Error('Refusing cleanup outside the disposable restore-lab directory.');
    rmSync(this.directory, { recursive: true, force: true });
  }
  async main() {
    this.initialize();
    let report;
    try { report = await this.exercise(); }
    catch (error) {
      report = { status: 'failed', created_at: now(), package: { file_name: basename(this.packagePath), sha256: this.originalChecksum },
        error: error instanceof Error ? error.message : String(error) };
      this.probe('docker', [...this.compose, 'logs', '--no-color', '--tail', '300'], { cwd: root, stdio: 'inherit', env: this.environment });
      throw error;
    } finally {
      mkdirSync(dirname(this.reportPath), { recursive: true });
      writeFileSync(this.reportPath, JSON.stringify(report, null, 2) + '\n');
      console.log(`Restore lab report: ${this.reportPath}`);
      this.cleanup();
    }
  }
}
if (direct(import.meta.url)) await new RestoreLab(process.argv.slice(2)).main();

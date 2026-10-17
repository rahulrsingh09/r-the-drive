#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { gunzipSync } from 'node:zlib';
import { chmodSync, copyFileSync, existsSync, lstatSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Commands, direct, option, pollUntil } from './operational-runtime.mjs';

const SHA256 = /^[a-f0-9]{64}$/i;
const IMAGE_DIGEST = /@sha256:([a-f0-9]{64})$/i;
const PACKAGE_NAME = /\.rma\.tar\.gz$/i;
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export class HarnessError extends Error {}
const insist = (condition, message) => { if (!condition) throw new HarnessError(message); };
const sha256 = data => createHash('sha256').update(data).digest('hex');
const validPort = port => Number.isInteger(port) && port >= 1 && port <= 65535;
const timestamp = () => new Date().toISOString();

export function parseArgs(args) {
  if (args.some(arg => arg === '--help' || arg === '-h')) return { help: true };
  const fields = {
    package: 'packagePath', 'latest-package-dir': 'latestDir', 'baseline-image': 'baselineImage', 'dev-image': 'devImage',
    'production-root': 'productionRoot', 'production-port': 'productionPort', 'test-root': 'testRoot',
    'env-file': 'envFile', 'compose-file': 'composeFile', project: 'project', port: 'port', 'health-host': 'healthHost',
    'komodo-revision': 'komodoRevision', 'source-sha': 'sourceSha', 'prerelease-tag': 'prereleaseTag', 'compose-source-sha': 'composeSourceSha',
  };
  const flags = new Set(['--dry-run', '--plan', '--execute', '--reset-test-storage', '--sha256']);
  const raw = {};
  for (const [flag, key] of Object.entries(fields)) raw[key] = option(args, '--' + flag);
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    const valued = Object.hasOwn(fields, arg.slice(2)) && arg.startsWith('--');
    if (arg.startsWith('-') && !valued && !flags.has(arg)) throw new HarnessError(`Unknown argument: ${arg}`);
    if (valued) insist(args[index + 1] && !args[index + 1].startsWith('-'), `${arg} requires a value.`);
  }
  raw.suppliedSha = option(args, '--sha256');
  if (args.includes('--sha256')) insist(raw.suppliedSha && SHA256.test(raw.suppliedSha), '--sha256 must be 64 hexadecimal characters.');
  insist(!(raw.packagePath && raw.latestDir), 'Use exactly one of --package or --latest-package-dir.');
  insist(raw.packagePath || raw.latestDir, '--package or --latest-package-dir is required.');
  insist(raw.baselineImage && raw.devImage, '--baseline-image and --dev-image are required.');
  insist(raw.productionRoot && isAbsolute(raw.productionRoot), '--production-root must be an absolute path.');
  raw.productionPort = Number(raw.productionPort);
  insist(validPort(raw.productionPort), '--production-port must be an integer from 1 to 65535.');
  insist(raw.testRoot && isAbsolute(raw.testRoot), '--test-root must be an absolute path.');
  insist(raw.envFile, '--env-file is required.');
  insist(raw.composeFile, '--compose-file is required; do not use the production Compose file implicitly.');
  insist(raw.project, '--project is required.');
  raw.port = Number(raw.port);
  insist(validPort(raw.port), '--port must be an integer from 1 to 65535.');
  insist(raw.port !== raw.productionPort, 'The test port must differ from --production-port.');
  return { ...raw, healthHost: raw.healthHost ?? '127.0.0.1', komodoRevision: raw.komodoRevision ?? process.env.KOMODO_STACK_REVISION,
    reset: args.includes('--reset-test-storage'), execute: args.includes('--execute'),
    dryRun: !args.includes('--execute') || args.includes('--dry-run') || args.includes('--plan') };
}
function under(child, parent) {
  const path = relative(parent, child);
  if (path === '') return true;
  return !isAbsolute(path) && path !== '..' && !path.startsWith('..' + sep);
}
function imageInfo(ref, label) {
  insist(!/:latest(?:$|@)/i.test(ref) && !/(^|[/:])latest$/i.test(ref), `${label} must not use tag latest.`);
  const match = IMAGE_DIGEST.exec(ref);
  insist(match, `${label} must be pinned with @sha256:<64 hex>.`);
  return { ref, digest: match[1].toLowerCase() };
}
function targetRoot(root, production) {
  insist(root.toLowerCase() !== production.toLowerCase(), 'The production root is not an allowed test target.');
  const labels = /^(test|tests|testing|tmp|temp|dev|harness)$|(?:^|[-_])(test|testing|dev|harness)(?:[-_]|$)/i;
  const candidates = under(root, production) ? [relative(production, root).split(/[\\/]/)[0]] : root.split(/[\\/]/);
  insist(candidates.some(part => labels.test(part)), 'Target root must be explicitly test-labelled.');
  return root;
}
function safeProject(project) {
  const lower = project.toLowerCase();
  const prod = /(?:^|[-_.])prod(?:uction)?(?:$|[-_.])/.test(lower);
  const exception = /(?:^|[-_.])prod-test(?:$|[-_.])/.test(lower);
  const labelled = /(?:^|[-_.])(test|dev|harness|clone)(?:$|[-_.])/i.test(project);
  insist(/^[a-z0-9][a-z0-9_.-]*$/i.test(project) && (!prod || exception) && labelled, 'Project must be an explicitly test-labelled identifier.');
}
function packagePath(raw, cwd) {
  if (raw.packagePath) return resolve(cwd, raw.packagePath);
  const directory = resolve(cwd, raw.latestDir);
  insist(existsSync(directory) && statSync(directory).isDirectory(), 'Latest-package directory does not exist or is not a directory.');
  let chosen;
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (!entry.isFile() || !PACKAGE_NAME.test(entry.name)) continue;
    const path = join(directory, entry.name), modified = statSync(path).mtimeMs;
    if (!chosen || modified > chosen.modified || (modified === chosen.modified && path.localeCompare(chosen.path) < 0)) chosen = { path, modified };
  }
  insist(chosen, 'No .rma.tar.gz package found in the bounded package directory.');
  return chosen.path;
}
function liveData(path, productionRoot) {
  if (under(path, resolve(productionRoot, 'backups'))) return false;
  const normalized = path.split('\\').join('/').toLowerCase();
  return under(path, resolve(productionRoot)) || /(?:^|\/)(?:postgres(?:ql)?|pgdata|timescaledb|database|live-data)(?:\/|$)/.test(normalized);
}
export function archiveEntries(buffer) {
  const entries = new Map();
  let cursor = 0;
  const text = (start, end) => buffer.subarray(cursor + start, cursor + end).toString('utf8').split('\0', 1)[0];
  while (cursor + 512 <= buffer.length) {
    if (buffer.subarray(cursor, cursor + 512).every(byte => byte === 0)) break;
    const prefix = text(345, 500), name = text(0, 100);
    const path = (prefix ? prefix + '/' + name : name).split('\\').join('/');
    const sizeText = text(124, 136).trim();
    const size = sizeText ? Number.parseInt(sizeText, 8) : 0;
    const type = buffer[cursor + 156];
    insist(Number.isSafeInteger(size) && size >= 0, 'Recovery archive has an invalid member size.');
    insist(!path.startsWith('/') && !isAbsolute(path) && !path.split('/').includes('..'), `Unsafe recovery archive member: ${path}`);
    insist([0, 48, 53].includes(type), `Unsupported recovery archive member type: ${path}`);
    insist(!entries.has(path), `Recovery archive contains a duplicate member: ${path}`);
    const data = type === 53 ? undefined : buffer.subarray(cursor + 512, cursor + 512 + size);
    entries.set(path, { path, size, type, data, hash: data && sha256(data) });
    cursor += 512 * (1 + Math.ceil(size / 512));
  }
  return entries;
}
function componentContract(component, name) {
  insist(component.version === 1 && typeof component.restore_policy === 'string' && component.restore_policy
    && Array.isArray(component.redactions), `Recovery component ${name} has an invalid contract.`);
}
function manifestContracts(manifest, entries) {
  const components = [['database', 'database.dump'], ['backup_settings', 'backup-settings.json']];
  function check(name, path, contract = false) {
    const component = manifest.components?.[name];
    insist(component && typeof component === 'object', `Recovery manifest is missing component ${name}.`);
    if (contract) componentContract(component, name);
    const actual = component.path ?? path;
    insist(actual === path && typeof component.sha256 === 'string' && entries.get(actual)?.hash === component.sha256
      && entries.get(actual)?.size === component.size_bytes, `Recovery package checksum or size mismatch for ${path}.`);
  }
  components.forEach(pair => check(...pair));
  if (manifest.format_version >= 2) {
    const source = manifest.source;
    for (const key of ['postgres_major', 'migration_version', 'migration_ledger', 'schema_fingerprint'])
      insist(source?.[key] !== undefined, `Recovery manifest is missing source.${key}.`);
    const engine = manifest.restore?.engine_version;
    insist(Number.isInteger(engine) && engine >= 1 && engine <= 3, `Recovery package requires unsupported restore engine version ${engine ?? 'unknown'}.`);
    const ledger = source.migration_ledger;
    insist(Array.isArray(ledger) && ledger.length > 0 && Number.isInteger(source.migration_version) && source.migration_version >= 1, 'Recovery migration ledger is incomplete.');
    let version = 0;
    for (const row of ledger) {
      insist(Number.isInteger(row?.version) && row.version >= 1 && row.version > version
        && typeof row.checksum_sha384 === 'string' && row.checksum_sha384, 'Recovery migration ledger is invalid.');
      version = row.version;
    }
    insist(version === source.migration_version, 'Recovery migration ledger does not end at the declared migration version.');
    if (manifest.format_version === 3) for (const key of ['migration_chain_id', 'migration_catalog_digest', 'schema_contract_version', 'app_version'])
      insist(typeof source[key] === 'string' && source[key], `Recovery v3 manifest is missing source.${key}.`);
    check('operational_history', 'operational-history.json', true);
    for (const [name] of components) componentContract(manifest.components[name], name);
  }
  const files = manifest.components?.vehicle_image_cache?.files;
  if (files) for (const file of files) insist(typeof file?.path === 'string' && entries.get(file.path)?.hash === file.sha256
    && entries.get(file.path)?.size === file.size_bytes, `Recovery artwork checksum or size mismatch for ${file?.path ?? 'unknown file'}.`);
}
export function inspectPackage(path, suppliedSha, { requireChecksum = false, productionRoot } = {}) {
  insist(existsSync(path) && lstatSync(path).isFile(), `Recovery package does not exist: ${path}`);
  path = realpathSync(path);
  if (productionRoot) insist(!liveData(path, productionRoot), 'Recovery package may not be inside PostgreSQL live data.');
  const bytes = readFileSync(path), hash = sha256(bytes);
  let expected = suppliedSha;
  if (!expected && existsSync(path + '.sha256')) expected = /[a-f0-9]{64}/i.exec(readFileSync(path + '.sha256', 'utf8'))?.[0];
  insist(!requireChecksum || expected, 'Recovery package SHA-256 requires --sha256 or a matching .sha256 sidecar.');
  if (expected) insist(expected.toLowerCase() === hash, 'Recovery package SHA-256 does not match the supplied verification value.');
  let entries;
  try { entries = archiveEntries(gunzipSync(bytes)); }
  catch (error) { throw new HarnessError(`Invalid recovery archive: ${error.message}`); }
  for (const name of ['manifest.json', 'database.dump', 'backup-settings.json']) insist(entries.has(name), `Recovery package is missing ${name}.`);
  insist(entries.get('database.dump').data.subarray(0, 5).toString() === 'PGDMP', 'Recovery package database.dump is not a PostgreSQL custom-format dump.');
  let manifest;
  try { manifest = JSON.parse(entries.get('manifest.json').data.toString('utf8')); }
  catch { throw new HarnessError('Recovery manifest is invalid JSON.'); }
  insist(['panda-recovery-v1', 'panda-recovery-v2', 'panda-recovery-v3'].includes(manifest.format)
    && [1, 2, 3].includes(manifest.format_version), 'Unsupported recovery package format.');
  manifestContracts(manifest, entries);
  return { path, sha256: hash, bytes: bytes.length, members: entries.size, manifest: {
    format: manifest.format, format_version: manifest.format_version, migrationVersion: manifest.source?.migration_version ?? null } };
}
function fileAt(path, label) {
  insist(existsSync(path) && lstatSync(path).isFile(), `${label} file does not exist: ${path}`);
  return path;
}
export function buildPlan(raw, cwd = process.cwd()) {
  const pkgPath = packagePath(raw, cwd);
  const productionRoot = resolve(cwd, raw.productionRoot);
  const testRoot = targetRoot(resolve(cwd, raw.testRoot), productionRoot);
  safeProject(raw.project);
  insist(validPort(raw.productionPort), 'Production port is invalid.');
  insist(validPort(raw.port) && raw.port !== raw.productionPort, 'Test port is invalid or matches the production port.');
  const images = { baseline: imageInfo(raw.baselineImage, 'Baseline image'), dev: imageInfo(raw.devImage, 'Dev image') };
  const envFile = fileAt(resolve(cwd, raw.envFile), 'Environment');
  insist(!under(envFile, productionRoot) || under(envFile, resolve(testRoot, 'secrets')), 'Production env files may not be copied into the test harness.');
  const composeFile = fileAt(resolve(cwd, raw.composeFile), 'Compose');
  const text = readFileSync(composeFile, 'utf8');
  const normalize = value => value.split('\\').join('/').toLowerCase();
  const production = normalize(productionRoot), test = normalize(testRoot);
  const illegalPath = normalize(text).split(production).slice(1).some(suffix => !suffix.startsWith(test.slice(production.length)));
  const prohibited = [/\blatest\b/i, new RegExp(`(?:^|\\D)${raw.productionPort}(?:\\D|$)`), /t3_proxy|traefik|panda-prod-internal/i];
  insist(!illegalPath && !prohibited.some(pattern => pattern.test(text)), 'Compose file contains production or mutable-image configuration.');
  const pkg = inspectPackage(pkgPath, raw.suppliedSha, { requireChecksum: true, productionRoot });
  const data = Object.fromEntries(['db', 'redis', 'backups', 'cache'].map(name => [name, join(testRoot, name)]));
  const populated = Object.values(data).some(path => existsSync(path) && readdirSync(path).length > 0);
  insist(!populated || raw.reset, 'Target storage is populated; pass --reset-test-storage to replace it.');
  if (raw.reset) insist(Object.values(data).every(path => under(path, testRoot)), 'Refusing to reset storage outside the exact test root.');
  const provenance = Object.fromEntries(['sourceSha', 'prereleaseTag', 'composeSourceSha'].map(name => [name, raw[name] ?? 'not-provided']));
  return { version: 1, kind: 'panda-dev-harness-plan', createdAt: timestamp(), package: pkg, images, provenance,
    target: { root: testRoot, data, state: join(testRoot, 'harness-state.json'), lock: join(testRoot, 'harness.lock.json'), envFile, composeFile,
      port: raw.port, productionRoot, productionPort: raw.productionPort, healthHost: raw.healthHost, project: raw.project,
      komodoRevision: raw.komodoRevision ?? 'not-provided' },
    phases: ['preflight', ...(raw.reset ? ['reset-test-storage'] : []), 'stage-package-read-only', 'restore-with-baseline-image',
      'verify-baseline-data', 'upgrade-to-dev-image', 'verify-dev-health', 'record-komodo-readback'], destructive: Boolean(raw.reset),
    execution: raw.execute ? 'execute' : 'dry-run' };
}

export function help() {
  return `Usage: node scripts/dev-harness.mjs [options]\n\nRequired: --package PATH or --latest-package-dir DIR, --baseline-image REF, --dev-image REF, --production-root PATH, --production-port PORT, --test-root PATH, --env-file PATH, --compose-file PATH, --port PORT, --project NAME\nOptional: --sha256 HASH, --source-sha SHA, --prerelease-tag TAG, --compose-source-sha SHA, --health-host HOST, --komodo-revision REV, --reset-test-storage, --dry-run/--plan (default), --execute\nImages require immutable @sha256 digests. Execution invokes the existing isolated restore script, then upgrades only the test Compose project; Komodo is not called.`;
}

export function writeHarnessEnv(source, destination, image, plan) {
  const lines = readFileSync(source, 'utf8').split(/\r?\n/);
  const development = lines.some(line => /^\s*PANDA_ENV\s*=\s*development\s*$/i.test(line));
  const overrides = new Map();
  if (development) overrides.set('COOKIE_INSECURE', 'true');
  overrides.set('PANDA_IMAGE', image.ref); overrides.set('PANDA_IMAGE_REF', image.ref);
  const match = /^(.*)\/([^/:]+):([^@]+)@sha256:([a-f0-9]{64})$/i.exec(image.ref);
  if (match) { overrides.set('PANDA_IMAGE_REGISTRY', match[1]); overrides.set('IMAGE_TAG', `${match[3]}@sha256:${match[4]}`); }
  const target = plan.target;
  for (const pair of [['PANDA_DATA_DIR', target.root], ['PANDA_ORIGIN_PORT', target.port], ['PANDA_HOST_BIND_ADDRESS', target.healthHost],
    ['PANDA_HEALTH_HOST', target.healthHost], ['COMPOSE_PROJECT_NAME', target.project], ['PANDA_ENV_FILE', destination]]) overrides.set(...pair);
  const removed = new Set([...overrides.keys(), 'PANDA_IMAGE_REGISTRY', 'IMAGE_TAG']);
  const retained = [];
  for (const line of lines) {
    const name = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=/.exec(line)?.[1];
    if (!removed.has(name)) retained.push(line);
  }
  for (const [name, value] of overrides) retained.push(`${name}=${value}`);
  mkdirSync(dirname(destination), { recursive: true, mode: 0o700 });
  writeFileSync(destination, retained.filter((line, index) => line || index < retained.length - 1).join('\n') + '\n', { mode: 0o600 });
}
function jsonFile(path, data) {
  const text = JSON.stringify(data, null, 2) + '\n';
  writeFileSync(path, text, { mode: 0o600 });
}
export async function executePlan(raw, plan, { execute = execFileSync, fetchImpl = fetch, poll = pollUntil } = {}) {
  const target = plan.target;
  mkdirSync(target.root, { recursive: true });
  const lock = (phase, evidence) => jsonFile(target.lock, { kind: 'panda-dev-harness-lock', version: 1, phase, updatedAt: timestamp(),
    package: plan.package, images: plan.images, provenance: plan.provenance, target, ...evidence });
  lock('preflight', { acquiredAt: timestamp() });
  for (const path of Object.values(target.data)) {
    insist(under(path, target.root) && path !== target.root, 'Refusing to reset a path outside the exact test root.');
    insist(!existsSync(path) || !lstatSync(path).isSymbolicLink(), `Refusing to reset symlinked test storage: ${path}`);
    rmSync(path, { recursive: true, force: true });
    mkdirSync(path, { recursive: true, mode: 0o700 });
  }
  const staging = join(target.data.backups, '.dev-harness');
  mkdirSync(staging, { recursive: true, mode: 0o700 });
  const stagedPackage = join(staging, basename(plan.package.path));
  copyFileSync(plan.package.path, stagedPackage);
  try { chmodSync(stagedPackage, 0o444); } catch { /* Windows may not expose modes. */ }
  const baselineEnv = join(target.root, '.dev-harness', 'baseline.env');
  const devEnv = join(target.root, '.dev-harness', 'dev.env');
  for (const [file, image] of [[baselineEnv, plan.images.baseline], [devEnv, plan.images.dev]]) writeHarnessEnv(raw.envFile, file, image, plan);
  const commands = new Commands(root, process.env, execute);
  const compose = (file, args, capture = false) => commands.invoke('docker', ['compose', '-p', target.project, '--env-file', file, '-f', target.composeFile, ...args],
    { capture, env: { ...process.env, PANDA_ENV_FILE: file } });
  const checkData = file => {
    const tables = [['users', 'panda.users'], ['dashboards', 'panda.dashboards'], ['vehicles', 'panda.vehicles'],
      ['telemetry', 'timeseries.telemetry'], ['artwork', 'panda.vehicle_images'], ['providerCredentials', 'panda.vehicle_credentials']];
    const query = 'SELECT ' + tables.map(([, table]) => `(SELECT count(*) FROM ${table})`).join(" || ',' || ");
    const values = compose(file, ['exec', '-T', 'timescaledb', 'psql', '-U', 'panda', '-d', 'panda', '-At', '-c', query], true)
      .split(',').map(value => Number.parseInt(value, 10));
    insist(values.length === 6 && values.every(Number.isSafeInteger), 'Test database data verification returned an invalid result.');
    insist(values[0] >= 1, 'Restored test database contains no users.');
    return Object.fromEntries(tables.map(([name], index) => [name, values[index]]));
  };
  jsonFile(target.state, { ...plan, phase: 'restore-with-baseline-image', stagedPackage, startedAt: timestamp() });
  const restore = join(dirname(fileURLToPath(import.meta.url)), 'restore-backup.mjs');
  commands.invoke(process.execPath, [restore, '--package', stagedPackage, '--env-file', baselineEnv, '--compose-file', target.composeFile,
    '--project', target.project, '--skip-build'], { env: { ...process.env, PANDA_ENV_FILE: baselineEnv } });
  const baselineData = checkData(baselineEnv);
  lock('baseline-verified', { stagedPackage, baselineData });
  jsonFile(target.state, { ...plan, phase: 'upgrade-to-dev-image', stagedPackage, baselineData, baselineVerifiedAt: timestamp() });
  compose(devEnv, ['up', '-d', '--no-build']);
  const origin = `http://${target.healthHost}:${target.port}`;
  await poll(async () => { try { return (await fetchImpl(origin + '/health')).ok; } catch { return false; } },
    { timeout: 180_000, message: `Timed out waiting for ${origin}/health.` });
  const setup = await (await fetchImpl(origin + '/v1/auth/setup')).json();
  insist(!setup.setup_required, 'Restored test application unexpectedly requires setup.');
  const health = { origin, setupRequired: Boolean(setup.setup_required) };
  const devData = checkData(devEnv);
  const evidence = { stagedPackage, baselineData, devData, health,
    migrationTransition: { from: plan.images.baseline, to: plan.images.dev, database: 'test-only', rollback: 'fresh-storage-restore' } };
  jsonFile(target.state, { ...plan, phase: 'complete', ...evidence, completedAt: timestamp() });
  lock('complete', evidence);
  console.log(`Dev harness completed for ${target.project}; production was not targeted.`);
}
export async function main(args = process.argv.slice(2)) {
  if (args.includes('--help') || args.includes('-h')) { console.log(help()); return; }
  try {
    const raw = parseArgs(args), plan = buildPlan(raw);
    if (raw.dryRun) console.log(JSON.stringify(plan, null, 2));
    else await executePlan(raw, plan);
  } catch (error) { process.exitCode = 1; console.error(`dev-harness: ${error.message}`); }
}
if (direct(import.meta.url)) await main();

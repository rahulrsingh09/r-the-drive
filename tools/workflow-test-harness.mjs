import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {execFileSync, spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {parse} from 'yaml';

export const root = fileURLToPath(new URL('../', import.meta.url));
export const sha = 'a'.repeat(40);
export const digest = `sha256:${'b'.repeat(64)}`;

export function workflow(name) {
  const file = `.github/workflows/${name}.yml`;
  const source = process.env.WORKFLOW_ORACLE_REF
    ? execFileSync('git', ['show', `${process.env.WORKFLOW_ORACLE_REF}:${file}`], {cwd: root, encoding: 'utf8'})
    : readFileSync(path.join(root, file), 'utf8');
  return parse(source);
}

export function step(name, job, title) {
  const found = workflow(name).jobs[job].steps.find((entry) => entry.name === title || entry.id === title);
  if (!found?.run) throw new Error(`Missing program: ${name}/${job}/${title}`);
  return found.run;
}

// Every provider/build command is an inert argv recorder. Unknown requests fail.
const fakeCommand = `
const fs = require('node:fs'), path = require('node:path');
const command = path.basename(process.argv[1]), args = process.argv.slice(2);
const plan = JSON.parse(fs.readFileSync(process.env.FAKE_PLAN, 'utf8'));
fs.appendFileSync(process.env.FAKE_LOG, JSON.stringify([command, ...args]) + '\\n');
if (command === 'node' && args[0] === '-e') {
  const m = JSON.parse(fs.readFileSync(args[2], 'utf8'));
  const d = m['containerimage.descriptor']?.digest ?? m['containerimage.digest'];
  if (!d) process.exit(1);
  process.stdout.write(String(d)); process.exit(0);
}
const rule = plan.find(({argv, prefix}) => {
  const values = [command, ...args];
  return argv ? JSON.stringify(values) === JSON.stringify(argv)
    : prefix.every((value, index) => values[index] === value);
});
if (!rule) { console.error('UNPLANNED COMMAND: ' + JSON.stringify([command, ...args])); process.exit(97); }
if (rule.after) {
  const deadline = Date.now() + 2000;
  const wait = new Int32Array(new SharedArrayBuffer(4));
  while (!fs.readFileSync(process.env.FAKE_LOG, 'utf8').split('\\n').filter(Boolean)
    .some((line) => JSON.stringify(JSON.parse(line)) === JSON.stringify(rule.after))) {
    if (Date.now() >= deadline) { console.error('SIMULATED CHILD DID NOT START'); process.exit(98); }
    Atomics.wait(wait, 0, 0, 5);
  }
}
if (rule.metadata !== undefined && args.includes('--metadata-file')) {
  const target = args[args.indexOf('--metadata-file') + 1];
  fs.writeFileSync(target, JSON.stringify(rule.metadata));
}
if (rule.stdout) process.stdout.write(rule.stdout);
if (rule.signal) process.kill(process.pid, rule.signal);
if (rule.hold) setInterval(() => {}, 1000);
else process.exit(rule.status ?? 0);
`;

export function run(program, {env = {}, rules = [], files = {}, substitutions = {}} = {}) {
  const dir = mkdtempSync(path.join(tmpdir(), 'workflow-contract-'));
  const bin = path.join(dir, 'bin');
  mkdirSync(bin);
  for (const command of ['git', 'gh', 'docker', 'cargo', 'pnpm', 'node', 'openssl', 'date', 'curl', 'sleep', 'sudo', 'apt-get', 'npm']) {
    writeFileSync(path.join(bin, command), `#!${process.execPath}\n${fakeCommand}`, {mode: 0o700});
  }
  writeFileSync(path.join(dir, 'plan.json'), JSON.stringify(rules));
  writeFileSync(path.join(dir, 'calls.jsonl'), '');
  for (const [name, value] of Object.entries(files)) {
    mkdirSync(path.dirname(path.join(dir, name)), {recursive: true});
    writeFileSync(path.join(dir, name), value);
  }
  const script = program.replace(/\$\{\{\s*([^}]+?)\s*\}\}/g, (_, expression) => {
    if (!(expression in substitutions)) throw new Error(`Unbound expression: ${expression}`);
    return substitutions[expression];
  });
  const output = path.join(dir, 'output'), summary = path.join(dir, 'summary'), environment = path.join(dir, 'environment');
  try {
    const result = spawnSync('/bin/bash', ['-e', '-o', 'pipefail', '-c', script], {
      cwd: dir, encoding: 'utf8', timeout: 10000,
      env: {
        PATH: `${bin}:/usr/bin:/bin`, LC_ALL: 'C', HOME: dir,
        GITHUB_OUTPUT: output, GITHUB_STEP_SUMMARY: summary, GITHUB_ENV: environment,
        RUNNER_TEMP: dir, FAKE_PLAN: path.join(dir, 'plan.json'), FAKE_LOG: path.join(dir, 'calls.jsonl'),
        GITHUB_REPOSITORY: 'fixture/repository', GITHUB_REPOSITORY_OWNER: 'fixture',
        GITHUB_SHA: sha, SOURCE_SHA: sha, IMAGE: 'registry.invalid/fixture', ...env,
      },
    });
    if (result.error) throw result.error;
    const read = (name) => {
      try { return readFileSync(path.join(dir, name), 'utf8'); }
      catch (error) { if (error.code === 'ENOENT') return ''; throw error; }
    };
    if (result.stderr.includes('UNPLANNED COMMAND:')) throw new Error(result.stderr);
    return {
      status: result.status, stdout: result.stdout, stderr: result.stderr,
      output: read('output'), summary: read('summary'), environment: read('environment'),
      calls: read('calls.jsonl').trim().split('\n').filter(Boolean).map(JSON.parse),
      files: Object.fromEntries(Object.keys(files).map((name) => [name, read(name)])),
      smoke: read('release-smoke.env') || read('prerelease-smoke.env') || read('fresh-install.env'),
      reports: {npm: read('npm-audit.json'), pnpm: read('pnpm-audit.json'), cargo: read('cargo-audit.json')},
      lock: read('images.lock'),
    };
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
}

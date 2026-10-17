import { execFileSync, spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

// Centralize the process boundary so contract tests never launch operational tools.
export class Commands {
  constructor(cwd, env = process.env, execute = execFileSync) {
    this.cwd = cwd;
    this.env = env;
    this.execute = execute;
  }
  invoke(command, args, { capture = false, ...options } = {}) {
    const output = this.execute(command, args, {
      cwd: this.cwd, env: this.env,
      ...(capture ? { encoding: 'utf8' } : { stdio: 'inherit' }), ...options,
    });
    return capture ? output.trim() : output;
  }
}

export function option(args, name, fallback) {
  const position = args.indexOf(name);
  return position < 0 ? fallback : args[position + 1];
}

export function direct(meta, argv = process.argv) {
  return Boolean(argv[1]) && resolve(argv[1]) === fileURLToPath(meta);
}

export const delay = ms => new Promise(done => setTimeout(done, ms));

export async function pollUntil(probe, { timeout, interval = 1000, now = Date.now, sleep = delay, message }) {
  const end = now() + timeout;
  for (;;) {
    if (now() >= end) throw new Error(message);
    const result = await probe();
    if (result) return result;
    await sleep(interval);
  }
}

export function childCompletion(child, event = 'exit') {
  return new Promise((accept, reject) => {
    child.once('error', reject);
    child.once(event, (code, signal) => accept({ code, signal }));
  });
}

export async function runChild(command, args, options = {}, launch = spawn) {
  const { code, signal } = await childCompletion(launch(command, args, options));
  if (code !== 0) throw new Error(`${command} ${args.join(' ')} failed with ${signal ?? `exit code ${code}`}`);
}

export function fieldsMatch(value, expected) {
  return value != null && Object.entries(expected).every(([key, wanted]) => value[key] === wanted);
}

export function demand(condition, message, ErrorType = Error) {
  if (condition) return;
  throw new ErrorType(message);
}

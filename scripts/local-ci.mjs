import { mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { colorTokenViolations } from './check-color-tokens.mjs';
import { selectHookGate } from './local-ci-policy.mjs';
import { isMain } from './lib/validation-files.mjs';

const ciEnv = {
  DEV_POSTGRES_PORT: '55432',
  DEV_REDIS_PORT: '56379',
  DATABASE_URL: 'postgresql://panda:devpassword@127.0.0.1:55432/panda',
  REDIS_URL: 'redis://127.0.0.1:56379',
  PANDA_ENV: 'development',
};
const commonCommands = [
  ['node', ['tools/migration-integrity.mjs'], 'Migration integrity'],
  ['pnpm', ['deps:check'], 'Dependency policy'],
  ['pnpm', ['peers', 'check'], 'Peer dependencies'],
  ['pnpm', ['turbo', 'lint'], 'Lint'],
  [null, [], 'Color tokens'],
  ['pnpm', ['typecheck'], 'Typecheck'],
  ['pnpm', ['docs:check'], 'Documentation check'],
  ['pnpm', ['compose:check'], 'Compose contract'],
  ['pnpm', ['release-workflows:check'], 'Release workflow contract'],
  ['pnpm', ['compose:render-check'], 'Compose render check'],
  ['pnpm', ['architecture:check'], 'Architecture guards'],
  ['pnpm', ['security:routes'], 'Route security inventory'],
  ['pnpm', ['dashboards:sync-defaults', '--check'], 'Dashboard default drift'],
  ['pnpm', ['charts:sync-defaults', '--check'], 'Chart default drift'],
  ['pnpm', ['test'], 'Workspace tests'],
];

export function createLocalCi({
  root,
  environment = process.env,
  execute = spawnSync,
  sleep = delay,
  colors = colorTokenViolations,
} = {}) {
  root ??= resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const env = {
    ...environment,
    CARGO_TARGET_DIR: environment.CARGO_TARGET_DIR ?? join(root, 'apps/api/target-ci-local'),
  };
  const api = join(root, 'apps/api');
  const invoke = (command, args, options = {}) =>
    execute(process.platform === 'win32' && command === 'pnpm' ? 'pnpm.cmd' : command, args, {
      cwd: options.cwd ?? root,
      shell: process.platform === 'win32',
      ...options,
      env: { ...env, ...options.env },
    });
  const run = (command, args = [], options = {}) => {
    const label = options.label ?? `${command} ${args.join(' ')}`;
    console.log(`\n==> ${label}`);
    const result = invoke(command, args, { ...options, stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0)
      throw new Error(`${label} failed with exit code ${result.status ?? 'unknown'}.`);
  };
  const readGit = (args) => {
    const result = invoke('git', args, { encoding: 'utf8', windowsHide: true });
    return result.status === 0 ? result.stdout.trim() : '';
  };
  const hygiene = () => {
    const generated =
      /(^|\/)(\.codex|\.claude|node_modules|target|dist|coverage|\.turbo|\.playwright-cli)(\/|$)|(^|\/)(package-lock\.json|test_output\.txt)$|(^|\/)[^/]+\.tmp$/;
    const forbidden = readGit(['ls-files'])
      .split(/\r?\n/)
      .filter((file) => generated.test(file));
    if (forbidden.length)
      throw new Error(`Generated or local-only files are tracked:\n${forbidden.join('\n')}`);
    run(
      'git',
      readGit(['rev-parse', '--verify', 'HEAD'])
        ? ['diff', '--check', 'HEAD']
        : ['diff', '--cached', '--check'],
      { label: 'Repository whitespace check' }
    );
  };
  const common = ({ build = false, apiTests = 'lib' } = {}) => {
    run('pnpm', ['install', '--frozen-lockfile'], { label: 'Install locked dependencies' });
    hygiene();
    for (const [command, args, label] of commonCommands) {
      if (command) run(command, args, { label });
      else {
        const violations = colors(root);
        if (violations.length) {
          console.error(['Color-token violations found:', ...violations].join('\n'));
          throw new Error('Use design tokens instead of raw colors.');
        }
      }
    }
    if (apiTests === 'all') run('pnpm', ['test:api'], { label: 'API tests' });
    if (apiTests === 'lib')
      run('cargo', ['test', '--manifest-path', 'apps/api/Cargo.toml', '--lib', '--all-features'], {
        label: 'API library tests',
      });
    run('cargo', ['fmt', '--all', '--check'], { cwd: api, label: 'Rust format check' });
    if (build) run('pnpm', ['build'], { label: 'Workspace build' });
  };
  const compose = (args) =>
    run(
      'docker',
      ['compose', '-p', 'panda-ci-local', '-f', 'compose/docker-compose.dev.yml', ...args],
      { env: { ...env, ...ciEnv }, label: `Docker Compose ${args.join(' ')}` }
    );
  const full = async () => {
    Object.assign(env, ciEnv);
    let pendingDisposal = compose;
    const cleanup = () => {
      const dispose = pendingDisposal;
      pendingDisposal = undefined;
      try {
        dispose?.(['down', '--volumes', '--remove-orphans']);
      } catch {
        /* Keep the validation failure. */
      }
    };
    process.once('exit', cleanup);
    try {
      const sqlx = invoke('cargo', ['sqlx', '--version'], {
        cwd: api,
        stdio: 'ignore',
        windowsHide: true,
      });
      if (sqlx.status !== 0)
        run(
          'cargo',
          ['install', 'sqlx-cli', '--no-default-features', '--features', 'postgres', '--locked'],
          { cwd: api, label: 'Install sqlx-cli' }
        );
      compose(['up', '-d', '--wait', 'timescaledb', 'redis']);
      let attempts = 0;
      while (true) {
        try {
          run('cargo', ['run', '--locked', '--bin', 'panda-migrate'], {
            cwd: api,
            label: 'Canonical fork migrations',
          });
          break;
        } catch (error) {
          attempts += 1;
          if (attempts === 5) throw error;
          console.warn(
            `Canonical fork migrations failed during service startup; retrying in 2s (${attempts}/5).`
          );
          await sleep(2000);
        }
      }
      common({ build: true, apiTests: 'none' });
      for (const [args, label] of [
        [
          ['sqlx', 'prepare', '--check', '--workspace', '--', '--all-targets', '--all-features'],
          'SQLx offline metadata',
        ],
        [['clippy', '--all-targets', '--all-features', '--', '-D', 'warnings'], 'Clippy'],
        [['test', '--all', '--all-features'], 'Database-backed API tests'],
        [
          [
            'test',
            '--lib',
            'db::migrations::tests::valid_public_ledger_removes_empty_legacy_ledger',
            '--',
            '--ignored',
            '--exact',
            '--test-threads=1',
          ],
          'Disposable migration ledger repair',
        ],
      ])
        run('cargo', args, { cwd: api, label });
      console.log('\nLocal CI parity gate passed.');
    } finally {
      cleanup();
      process.removeListener('exit', cleanup);
    }
  };
  return async (mode, args = []) => {
    switch (mode) {
      case 'local':
        common();
        break;
      case 'ci':
        await full();
        break;
      case 'hook': {
        const gate = selectHookGate(env);
        if (gate === 'bypass') console.warn('SKIP_LOCAL_CI=1: local verification bypassed.');
        else if (gate === 'full') await full();
        else {
          hygiene();
          run('node', ['tools/migration-integrity.mjs'], { label: 'Migration integrity' });
        }
        break;
      }
      case 'install-hooks':
        mkdirSync(join(root, '.githooks'), { recursive: true });
        run('git', ['config', 'core.hooksPath', '.githooks'], {
          label: 'Install repository Git hooks',
        });
        console.log(
          'Hooks installed. Commits and pushes use the fast local gate by default; set PANDA_FULL_LOCAL_CI=1 for the full CI gate.'
        );
        break;
      case 'uninstall-hooks':
        run('git', ['config', '--unset', 'core.hooksPath']);
        console.log('Repository Git hooks disabled.');
        break;
      case 'create-pr':
        await full();
        run('gh', ['pr', 'create', ...args], { label: 'Create GitHub pull request' });
        break;
      default:
        throw new Error(
          'Usage: local | ci | install-hooks | uninstall-hooks | create-pr | hook pre-commit|pre-push'
        );
    }
  };
}

if (isMain(import.meta.url)) {
  const [mode, ...args] = process.argv.slice(2);
  try {
    await createLocalCi()(mode, args);
  } catch (error) {
    console.error(`\n${error instanceof Error ? error.message : error}`);
    process.exitCode = 1;
  }
}

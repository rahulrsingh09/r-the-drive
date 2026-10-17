import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertLocalDatabaseResetAllowed } from './lib/database-reset-guard.mjs';
import { direct, runChild } from './operational-runtime.mjs';

const apiDir = resolve(dirname(fileURLToPath(import.meta.url)), '../apps/api');
export async function database(args, env = process.env, execute = runChild) {
  const [mode, ...flags] = args;
  const operations = {
    migrate: [['cargo', ['run', '--bin', 'panda-migrate']]],
    reset: [['sqlx', ['database', 'drop', '-y']], ['sqlx', ['database', 'create']],
      ['cargo', ['run', '--bin', 'panda-migrate']]],
  };
  if (!Object.hasOwn(operations, mode)) throw new Error('Usage: node scripts/database.mjs <migrate|reset> [--yes]');
  if (mode === 'reset') {
    const { database, host, port } = assertLocalDatabaseResetAllowed({ args: flags, env });
    console.warn(`Resetting explicitly acknowledged local database ${database} at ${host}:${port}.`);
  }
  for (const [command, arguments_] of operations[mode]) {
    const environment = { ...env, PGOPTIONS: '-c search_path=public' };
    if (command === 'cargo') environment.SQLX_OFFLINE = 'true';
    await execute(command, arguments_, {
      cwd: apiDir, env: environment, stdio: 'inherit',
      shell: process.platform === 'win32', windowsHide: true,
    });
  }
}
if (direct(import.meta.url)) await database(process.argv.slice(2));

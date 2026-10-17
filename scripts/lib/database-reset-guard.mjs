const LOOPBACK_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]']);

function localTarget(value, label, purpose) {
  let target;
  try {
    target = new URL(value);
  } catch {
    throw new Error(`${label} must be a valid PostgreSQL URL.`);
  }
  if (!LOOPBACK_HOSTS.has(target.hostname))
    throw new Error(`${purpose} is limited to loopback hosts; received ${target.hostname}.`);
  return {
    target,
    database: target.pathname.slice(1),
    host: target.hostname,
    port: target.port || '5432',
  };
}

export function assertLocalDatabaseResetAllowed({ args, env }) {
  const refusals = [
    [
      env.PANDA_ENV === 'production' || env.NODE_ENV === 'production',
      'Database reset is disabled in production environments.',
    ],
    [
      env.PANDA_ALLOW_DATABASE_RESET !== 'local-development',
      'Database reset requires PANDA_ALLOW_DATABASE_RESET=local-development.',
    ],
    [!args.includes('--yes'), 'Database reset requires the explicit --yes argument.'],
  ];
  const denial = refusals.find(([refused]) => refused);
  if (denial) throw new Error(denial[1]);
  const { database, host, port } = localTarget(env.DATABASE_URL, 'DATABASE_URL', 'Database reset');
  return { database, host, port };
}

export function assertDisposableUpgradeDatabase(env) {
  const url = env.UPGRADE_DATABASE_URL;
  if (!url)
    throw new Error('The populated upgrade harness requires an explicit UPGRADE_DATABASE_URL.');
  const { target, database } = localTarget(
    url,
    'UPGRADE_DATABASE_URL',
    'The populated upgrade harness'
  );
  const prefix = 'panda_upgrade';
  const suffix = database.slice(prefix.length);
  if (!database.startsWith(prefix) || (suffix !== '' && !/^_[a-z0-9_]+$/.test(suffix)))
    throw new Error('UPGRADE_DATABASE_URL must name a disposable panda_upgrade database.');
  return target.href;
}

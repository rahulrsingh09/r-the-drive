import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { isMain } from './lib/validation-files.mjs';

const authorizationMatrix = new Map([
  ['app_info', 'session'],
  ['api_keys', 'session_admin_or_vehicle_manager'],
  ['auth', 'public_metadata_and_session'],
  ['backfill', 'session_admin'],
  ['backups', 'session_admin'],
  ['battery', 'vehicle_read'],
  ['charging', 'vehicle_read_and_session_manager_mutation'],
  ['charts', 'session_vehicle_member'],
  ['cost_profiles', 'session_vehicle_manager'],
  ['dashboards', 'session_vehicle_member'],
  ['efficiency', 'vehicle_read'],
  ['external_connections', 'session_admin'],
  ['grafana', 'vehicle_read'],
  ['health', 'vehicle_read'],
  ['idle_drain', 'vehicle_read'],
  ['live', 'vehicle_read'],
  ['locations', 'vehicle_read'],
  ['metrics', 'vehicle_read'],
  ['overview', 'vehicle_read'],
  ['parked_energy', 'vehicle_read'],
  ['places', 'session_vehicle_member'],
  ['rivian_stewardship', 'session_vehicle_manager'],
  ['schedules', 'session_vehicle_manager'],
  ['settings', 'session_admin'],
  ['state_timeline', 'vehicle_read'],
  ['themes', 'session_account_owner'],
  ['trips', 'vehicle_read'],
  ['trip_tags', 'vehicle_read_and_manager_mutation'],
  ['themes', 'session_authenticated_owner_scoped'],
  ['users', 'session_admin'],
  ['vehicles', 'vehicle_read_and_session_mutation'],
]);

export function routeSecurityInventory(root) {
  const directory = join(root, 'apps/api/src/routes');
  const assembly = readFileSync(join(directory, 'mod.rs'), 'utf8');
  const modules = readdirSync(directory)
    .filter((file) => file.endsWith('.rs') && file !== 'mod.rs')
    .map((file) => ({
      name: file.slice(0, -3),
      source: readFileSync(join(directory, file), 'utf8'),
    }));
  const tests = new Set();
  const mounted = new Map();
  for (const match of assembly.matchAll(
    /\.merge\((\w+)::(router|protected_router|metadata_router)\(\)\)/g
  )) {
    if (!mounted.has(match[1])) mounted.set(match[1], new Set());
    mounted.get(match[1]).add(match[2]);
  }
  for (const module of modules) {
    for (const match of module.source.matchAll(
      /#\[cfg\(test\)\]\s*#\[path = "([^"/]+)\.rs"\]\s*mod \w+;/g
    ))
      tests.add(match[1]);
  }
  const active = modules.filter(
    (module) => !tests.has(module.name) && module.source.includes('.route(')
  );
  const rules = [
    [
      'API route modules are not mounted behind the protected router',
      (module) => {
        const entries = mounted.get(module.name);
        return module.name === 'auth'
          ? !(entries?.has('protected_router') && entries.has('metadata_router'))
          : !entries?.has('router');
      },
    ],
    [
      'Mounted API route modules missing authorization-matrix entries',
      (module) => !authorizationMatrix.has(module.name),
    ],
    [
      'Deprecated require_vehicle_owned helper is forbidden in route modules',
      (module) => module.source.includes('require_vehicle_owned'),
    ],
  ];
  const failures = rules.flatMap(([message, violates]) => {
    const offenders = active.filter(violates).map((module) => module.name);
    return offenders.length ? [`${message}: ${offenders.join(', ')}`] : [];
  });
  return { failures, count: modules.length };
}

if (isMain(import.meta.url)) {
  const { failures, count } = routeSecurityInventory(process.cwd());
  if (failures.length) {
    console.error(failures[0]);
    process.exitCode = 1;
  } else
    console.log(
      `API route security inventory and authorization matrix passed for ${count} route modules.`
    );
}

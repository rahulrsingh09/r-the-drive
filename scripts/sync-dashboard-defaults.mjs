import { syncDefaults } from './lib/default-sync.mjs';

const dashboards = ['dashboard', 'battery', 'efficiency', 'charging', 'trips'];
syncDefaults(
  process.cwd(),
  dashboards.map((name) => ({
    source: `packages/dashboards/src/defaults/${name}.json`,
    target: `apps/api/dashboards/${name}.json`,
    label: 'dashboard defaults',
  })),
  { check: process.argv.slice(2).includes('--check'), success: 'dashboard defaults are in sync' }
);

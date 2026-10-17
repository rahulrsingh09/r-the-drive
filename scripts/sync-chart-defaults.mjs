import { syncDefaults } from './lib/default-sync.mjs';

const charts = 'packages/dashboards/src/charts';
const contracts = [
  ['defaults/defaults.json', 'defaults.json', 'chart defaults'],
  ['bundled-renderers.json', 'bundled-renderers.json', 'bundled renderer capabilities'],
  ['sources/sources.json', 'sources.json', 'chart source manifest'],
];
syncDefaults(
  process.cwd(),
  contracts.map(([source, target, label]) => ({
    source: `${charts}/${source}`,
    target: `apps/api/charts/${target}`,
    label,
  })),
  {
    check: process.argv.slice(2).includes('--check'),
    createDirectories: true,
    perFileSuccess: true,
  }
);

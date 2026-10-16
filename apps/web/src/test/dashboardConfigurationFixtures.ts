import type { ChartRecord } from '@panda/types';
import type { DashboardConfig, WidgetInstance } from '../../../../packages/dashboards/src/schema';

export function configurationWidget(): WidgetInstance {
  return {
    id: '11111111-1111-4111-8111-111111111111',
    componentType: 'custom',
    definitionId: 'configuration-test.unknown',
    layout: { x: 9, y: 7, w: 3, h: 2 },
    options: { nested: { retained: ['# text', null, false, 0] } },
    visibility: [{ type: 'vehicle-connection', value: 'plugged' }],
  };
}

export function configurationDashboard(): DashboardConfig {
  return {
    schemaVersion: 2,
    id: '22222222-2222-4222-8222-222222222222',
    ownerId: '33333333-3333-4333-8333-333333333333',
    slug: 'configuration-test',
    name: 'Configuration test',
    description: 'Synthetic configuration',
    isDefault: true,
    isLocked: true,
    controls: { dateRange: false },
    widgets: [configurationWidget()],
  };
}

export function configurationChart(): ChartRecord {
  return {
    id: '44444444-4444-4444-8444-444444444444',
    ownerId: '33333333-3333-4333-8333-333333333333',
    baselineRevision: 5,
    slug: 'configuration-test',
    name: 'Configuration test',
    description: 'Synthetic configuration',
    isDefault: true,
    isLocked: true,
    isEnabled: false,
    config: {
      schemaVersion: 1,
      placements: [{ dashboardSlug: 'overview' }],
      timeframe: { mode: 'dashboard' },
      sources: [{
        id: 'main',
        sourceId: 'synthetic.series',
        params: { nested: { values: [null, false, 0, 'text'] } },
        filters: [],
        inherit: { vehicle: true, timeframe: true, tripTags: false },
      }],
      x: { field: { sourceBindingId: 'main', field: 'timestamp' }, kind: 'time' },
      series: [{
        id: 'value',
        label: 'Synthetic value',
        y: { sourceBindingId: 'main', field: 'value' },
        mark: 'line',
        fill: false,
        yAxis: 'y',
        color: { mode: 'token', token: 'accent' },
        transforms: [{ kind: 'expression', formula: '[value] / 2' }],
      }],
      axes: {
        x: { scale: 'linear', domain: { mode: 'auto' } },
        y: { scale: 'linear', domain: { mode: 'fixed', min: -10, max: 100 } },
      },
      display: { legend: 'auto', grid: true, tooltip: true, timeFilter: 'raw', curveSmoothness: 'straight' },
      interaction: { panZoom: true, touchExplore: true, connectGaps: false },
      rendererId: 'synthetic-renderer',
    },
  };
}

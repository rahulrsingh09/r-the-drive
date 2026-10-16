import { describe, expect, it } from 'vitest';
import {
  DEFAULT_DASHBOARDS,
  collectDashboardDataRequirements,
  registerWidget,
  type DashboardDataRequirements,
  type WidgetInstance,
} from '@panda/dashboards';

function declaringWidget(id: string, requirements?: DashboardDataRequirements): WidgetInstance {
  const definitionId = `data-contract-${id}`;
  registerWidget({
    componentType: 'custom',
    definitionId,
    title: id,
    defaultSize: { w: 3, h: 2 },
    minSize: { w: 1, h: 1 },
    component: () => null,
    dataRequirements: () => requirements,
  });
  return {
    id,
    componentType: 'custom',
    definitionId,
    layout: { x: 0, y: 0, w: 3, h: 2 },
  };
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object') {
    Object.values(value).forEach(deepFreeze);
    Object.freeze(value);
  }
  return value;
}

describe('dashboard data requirements', () => {
  it('does not request data for an empty dashboard or a widget without requirements', () => {
    expect(collectDashboardDataRequirements([])).toEqual({});
    expect(collectDashboardDataRequirements([declaringWidget('no-data')])).toEqual({});
  });

  it('deduplicates default-dashboard metric chip requests into one manifest', () => {
    const overview = DEFAULT_DASHBOARDS.find((dashboard) => dashboard.slug === 'dashboard');
    expect(overview).toBeTruthy();

    const requirements = collectDashboardDataRequirements(overview?.widgets ?? []);
    const metricIds = requirements.metrics?.map((request) => request.metric) ?? [];

    expect(metricIds).toEqual([...metricIds].sort());
    expect(new Set(metricIds).size).toBe(metricIds.length);
    expect(requirements.status).toBe(true);
    expect(metricIds.length).toBeGreaterThan(0);
  });

  it('merges duplicate metrics without losing a sparkline requirement', () => {
    const duplicate: WidgetInstance[] = [
      {
        id: 'one',
        componentType: 'sensor',
        definitionId: 'total_miles',
        title: 'Miles',
        layout: { x: 0, y: 0, w: 3, h: 2 },
        options: { metric: 'total_miles', chartType: 'none' },
      },
      {
        id: 'two',
        componentType: 'sensor',
        definitionId: 'total_miles',
        title: 'Miles',
        layout: { x: 3, y: 0, w: 3, h: 2 },
        options: { metric: 'total_miles', chartType: 'line' },
      },
    ];

    expect(collectDashboardDataRequirements(duplicate).metrics).toEqual([
      { metric: 'total_miles', include_latest: true, include_series: true },
    ]);
  });

  it('safely ignores an unknown custom widget', () => {
    const unknown = {
      id: 'unknown',
      componentType: 'custom',
      definitionId: 'future-widget',
      title: 'Future',
      layout: { x: 0, y: 0, w: 3, h: 2 },
      options: {},
    } satisfies WidgetInstance;

    expect(collectDashboardDataRequirements([unknown])).toEqual({});
  });

  it.each([
    [undefined, undefined, true, false],
    [undefined, false, true, false],
    [undefined, true, true, true],
    [false, undefined, true, false],
    [false, false, false, false],
    [false, true, true, true],
    [true, undefined, true, true],
    [true, false, true, true],
    [true, true, true, true],
  ])('merges metric flags %s and %s as latest=%s, series=%s', (left, right, latest, series) => {
    const widgets = [left, right].map((flag, index) => declaringWidget(`flag-${index}`, {
      metrics: [{
        metric: 'trip_miles',
        ...(flag === undefined ? {} : { include_latest: flag, include_series: flag }),
      }],
    }));
    const expected = [{ metric: 'trip_miles', include_latest: latest, include_series: series }];

    expect(collectDashboardDataRequirements(widgets).metrics).toEqual(expected);
    expect(collectDashboardDataRequirements([...widgets].reverse()).metrics).toEqual(expected);
  });

  it('sorts and deduplicates metrics across and within widgets without coupling latest to series', () => {
    const widgets = [
      declaringWidget('mixed-first', {
        metrics: [
          { metric: 'trip_miles', include_latest: false, include_series: true },
          { metric: 'avg_efficiency', include_latest: false },
          { metric: 'trip_miles', include_latest: true, include_series: false },
        ],
      }),
      declaringWidget('mixed-second', {
        metrics: [
          { metric: 'energy_charged', include_latest: false, include_series: true },
          { metric: 'avg_efficiency', include_series: false },
        ],
      }),
    ];
    const expected = [
      { metric: 'avg_efficiency', include_latest: true, include_series: false },
      { metric: 'energy_charged', include_latest: false, include_series: true },
      { metric: 'trip_miles', include_latest: true, include_series: true },
    ];

    expect(collectDashboardDataRequirements(widgets).metrics).toEqual(expected);
    expect(collectDashboardDataRequirements([...widgets].reverse()).metrics).toEqual(expected);
  });

  it.each(['status', 'batteryHealth', 'chargingSummary', 'efficiencySummary'] as const)(
    'retains %s when another widget omits or disables it',
    (summary) => {
      const widgets = [
        declaringWidget('summary-requested', { [summary]: true }),
        declaringWidget('summary-disabled', { [summary]: false }),
        declaringWidget('summary-omitted', {}),
      ];

      expect(collectDashboardDataRequirements(widgets)[summary]).toBe(true);
      expect(collectDashboardDataRequirements([...widgets].reverse())[summary]).toBe(true);
      expect(collectDashboardDataRequirements(widgets.slice(1))[summary]).toBe(false);
    },
  );

  it('leaves widget and registry declarations unchanged and returns independent metric requests', () => {
    const requirements = deepFreeze<DashboardDataRequirements>({
      status: true,
      metrics: [
        { metric: 'trip_miles', include_latest: false, include_series: true },
        { metric: 'avg_efficiency' },
      ],
    });
    const widgets = deepFreeze([declaringWidget('immutable', requirements)]);
    const originalRequirements = structuredClone(requirements);
    const originalWidgets = structuredClone(widgets);

    const result = collectDashboardDataRequirements(widgets);
    expect(result.metrics?.[0]).toEqual({
      metric: 'avg_efficiency', include_latest: true, include_series: false,
    });
    result.metrics![0]!.metric = 'changed-output';
    result.metrics!.push({ metric: 'extra-output' });

    expect(requirements).toEqual(originalRequirements);
    expect(widgets).toEqual(originalWidgets);
    expect(collectDashboardDataRequirements(widgets).metrics?.map(({ metric }) => metric)).toEqual([
      'avg_efficiency', 'trip_miles',
    ]);
  });
});

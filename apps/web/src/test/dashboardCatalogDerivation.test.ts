import { afterEach, describe, expect, it, vi } from 'vitest';
import { BUNDLED_CHART_DEFINITIONS } from '../../../../packages/dashboards/src/charts/defaults';
import { getChartDefinition, getChartDefinitions } from '../../../../packages/dashboards/src/charts/catalog';

afterEach(() => {
  vi.doUnmock('../../../../packages/dashboards/src/charts/defaults');
  vi.resetModules();
});

describe('bundled catalog derivation', () => {
  it('keeps each canonical definition, renderer mode, primary axis, and index identity', () => {
    const modes = {
      'battery-capacity-mileage': 'area', 'battery-degradation': 'line', 'charge-level': 'area',
      'charging-curve-analysis': 'scatter', 'charging-sessions-energy': 'bar', 'charging-weekly-energy': 'bar',
      'efficiency-mode': 'bar', 'efficiency-tags': 'bar', 'efficiency-temperature': 'scatter',
      'efficiency-trend': 'scatter', 'phantom-drain': 'bar', 'projected-range-mileage': 'area',
      'soc-history': 'area', 'tire-pressure-trips': 'line',
    };
    expect(getChartDefinitions()).toHaveLength(14);
    expect(getChartDefinitions().map(chart => chart.title)).toEqual(
      BUNDLED_CHART_DEFINITIONS.map(chart => chart.title).sort((a, b) => a.localeCompare(b)),
    );
    for (const bundled of BUNDLED_CHART_DEFINITIONS) {
      const definition = getChartDefinition(bundled.slug)!;
      const { slug: _slug, title: _title, description: _description, ...config } = bundled;
      expect(definition.config).toEqual(config);
      expect(definition.config).not.toBe(bundled);
      expect(definition.mode).toBe(modes[bundled.slug as keyof typeof modes]);
      expect(definition.pages).toEqual(bundled.placements.map(placement => placement.dashboardSlug));
      expect(definition.stepInterpolation).toBe(['charge-level', 'soc-history'].includes(bundled.slug));
      expect(getChartDefinitions().find(chart => chart.id === bundled.slug)).toBe(definition);
    }
    expect(getChartDefinition('battery-degradation')).toMatchObject({ yUnit: '%', yRange: [80, 100] });
    expect(getChartDefinition('charge-level')).toMatchObject({ yUnit: '%', yRange: [0, 100] });
    expect(getChartDefinition('soc-history')).toMatchObject({ yUnit: '%', yRange: [0, 100] });
    expect(getChartDefinition('missing')).toBeUndefined();
  });

  it.each([
    ['line', false, 'line'], ['area', false, 'area'], ['step', false, 'line'],
    ['bar', false, 'bar'], ['scatter', false, 'scatter'], ['histogram', false, 'line'],
    ['bar', true, 'area'], ['step', true, 'area'],
  ] as const)('derives %s fill=%s and the first series secondary axis', async (mark, fill, mode) => {
    const chart = structuredClone(BUNDLED_CHART_DEFINITIONS[0]!);
    chart.series[0] = { ...chart.series[0]!, mark, fill, yAxis: 'y2' };
    chart.axes.y2 = { scale: 'linear', unit: 'mi', domain: { mode: 'fixed', min: 12, max: 345 } };
    chart.placements = [{ dashboardSlug: 'trips' }, { dashboardSlug: 'private-dashboard' }, { dashboardSlug: 'trips' }];
    const before = structuredClone(chart);
    vi.resetModules();
    vi.doMock('../../../../packages/dashboards/src/charts/defaults', () => ({ BUNDLED_CHART_DEFINITIONS: [chart] }));
    const catalog = await import('../../../../packages/dashboards/src/charts/catalog');
    expect(catalog.getChartDefinition(chart.slug)).toMatchObject({
      mode, stepInterpolation: mark === 'step', yUnit: 'mi', yRange: [12, 345], pages: ['trips', 'trips'],
      defaultSize: { w: 6, h: 4 }, minSize: { w: 4, h: 3 },
    });
    expect(chart).toEqual(before);
  });

  it('omits empty optional metadata and rejects missing routing before schema validation', async () => {
    const chart = structuredClone(BUNDLED_CHART_DEFINITIONS[0]!);
    delete chart.axes.y.unit;
    delete chart.display.emptyTitle;
    vi.resetModules();
    vi.doMock('../../../../packages/dashboards/src/charts/defaults', () => ({ BUNDLED_CHART_DEFINITIONS: [chart] }));
    const catalog = await import('../../../../packages/dashboards/src/charts/catalog');
    const definition = catalog.getChartDefinition(chart.slug)!;
    expect(Object.hasOwn(definition, 'yUnit')).toBe(false);
    expect(Object.hasOwn(definition, 'yRange')).toBe(false);
    expect(Object.hasOwn(definition, 'emptyTitle')).toBe(false);
    vi.resetModules();
    vi.doMock('../../../../packages/dashboards/src/charts/defaults', () => ({
      BUNDLED_CHART_DEFINITIONS: [{ slug: 'no-route' }],
    }));
    await expect(import('../../../../packages/dashboards/src/charts/catalog')).rejects.toThrow(
      'Missing renderer routing for bundled chart: no-route',
    );
  });
});

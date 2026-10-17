import { describe, expect, it } from 'vitest';
import type { ChartRecord } from '@panda/types';
import { BUNDLED_CHART_DEFINITIONS } from '../../../../packages/dashboards/src/charts/defaults';
import { supportsBundledRendererCurve } from '../../../../packages/dashboards/src/charts/catalog';
import { defaultChartPermissions } from '../../../../packages/dashboards/src/charts/model';

describe('dashboard core policy and curve boundaries', () => {
  it.each([
    ['charge-level', 'metrics.series', 'battery_level', true],
    ['charge-level', 'battery.mileage', 'battery_level', false],
    ['charge-level', 'metrics.series', 'timestamp', false],
    ['charge-level', 'metrics.series', undefined, false],
    ['charge-level', 'metrics.series', '', false],
    ['unknown', 'metrics.series', 'battery_level', undefined],
    ['constructor', 'metrics.series', 'battery_level', undefined],
    ['unknown', 'metrics.series', undefined, undefined],
  ] as const)('resolves %s / %s / %s without conflating missing and unsupported', (slug, source, field, expected) => {
    expect(supportsBundledRendererCurve(slug, source, field)).toBe(expected);
  });

  it('treats a present empty owner as personal while retaining the lock veto', () => {
    const chart: ChartRecord = {
      id: 'synthetic-chart',
      ownerId: '',
      slug: 'synthetic-chart',
      name: 'Synthetic chart',
      isDefault: false,
      isEnabled: true,
      isLocked: true,
      config: BUNDLED_CHART_DEFINITIONS[0]!,
    };
    const before = structuredClone(chart);
    expect(defaultChartPermissions(chart, true)).toEqual({
      read: true,
      edit: false,
      duplicate: true,
      reset: false,
      restore: false,
      delete: true,
      lock: false,
    });
    expect(chart).toEqual(before);
  });
});

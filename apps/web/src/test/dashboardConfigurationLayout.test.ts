import { describe, expect, it } from 'vitest';
import { registerWidget } from '../../../../packages/dashboards/src/registry';
import {
  sanitizeDashboardConfig,
  sanitizeWidgetInstance,
  sanitizeWidgetLayout,
} from '../../../../packages/dashboards/src/layout';
import { configurationDashboard, configurationWidget } from './dashboardConfigurationFixtures';

describe('configuration layout contracts', () => {
  it.each([
    [{ x: 11, y: 0, w: 12, h: 1 }, { x: 0, y: 0, w: 12, h: 1 }],
    [{ x: -1, y: -1, w: 0, h: 0 }, { x: 0, y: 0, w: 1, h: 1 }],
    [{ x: 9.5, y: 2.5, w: 2.5, h: 1.5 }, { x: 9, y: 3, w: 3, h: 2 }],
    [{ x: NaN, y: Infinity, w: -Infinity, h: NaN }, { x: 0, y: 0, w: 1, h: 1 }],
    [{ x: 100, y: 1e20, w: 100, h: 1e20 }, { x: 0, y: 1e20, w: 12, h: Number.MAX_SAFE_INTEGER }],
  ])('projects dimensions before horizontal position: %j', (input, expected) => {
    const original = { ...input };
    expect(sanitizeWidgetLayout(input)).toEqual(expected);
    expect(input).toEqual(original);
  });

  it('uses explicit bounds and the lower bound for nonfinite dimensions', () => {
    expect(sanitizeWidgetLayout(
      { x: Infinity, y: NaN, w: NaN, h: Infinity },
      { minSize: { w: 4, h: 3 }, maxSize: { w: 8, h: 5 } },
    )).toEqual({ x: 0, y: 0, w: 4, h: 3 });
    expect(sanitizeWidgetLayout(
      { x: 11, y: 0, w: 12, h: 30 },
      { minSize: { w: 4, h: 3 }, maxSize: { w: 8, h: 5 } },
    )).toEqual({ x: 4, y: 0, w: 8, h: 5 });
  });

  it('retains upper-bound precedence and fractional bound behavior for direct callers', () => {
    expect(sanitizeWidgetLayout(
      { x: 11, y: 0, w: 2, h: 2 },
      { minSize: { w: 20, h: 6 }, maxSize: { w: 30, h: 4 } },
    )).toEqual({ x: 0, y: 0, w: 12, h: 4 });
    expect(sanitizeWidgetLayout(
      { x: 11, y: 0, w: 1, h: 1 },
      { minSize: { w: 2.5, h: 3.5 } },
    )).toEqual({ x: 9.5, y: 0, w: 2.5, h: 3.5 });
  });

  it('pins fixed dimensions to the supplied layout, still capping width at twelve', () => {
    expect(sanitizeWidgetLayout(
      { x: 11, y: 2, w: 20, h: 2.5 },
      { fixedSize: true, minSize: { w: 1, h: 1 }, maxSize: { w: 1, h: 1 } },
    )).toEqual({ x: 0, y: 2, w: 12, h: 2.5 });
  });

  it('reads fixed defaults and resizable bounds from registry capabilities', () => {
    for (const fixedSize of [true, false]) {
      const definitionId = `configuration-test.${fixedSize ? 'fixed' : 'resizable'}`;
      registerWidget({
        componentType: 'custom', definitionId, title: definitionId,
        defaultSize: { w: 4, h: 3 }, minSize: { w: 2, h: 2 },
        editor: { fixedSize, maxSize: { w: 8, h: 5 } }, component: () => null,
      });
      const widget = { ...configurationWidget(), definitionId, layout: { x: 11, y: 4, w: 12, h: 8 } };
      const original = structuredClone(widget);
      const sanitized = sanitizeWidgetInstance(widget);
      expect(sanitized.layout).toEqual(fixedSize
        ? { x: 8, y: 4, w: 4, h: 3 }
        : { x: 4, y: 4, w: 8, h: 5 });
      expect(sanitized.options).toBe(widget.options);
      expect(widget).toEqual(original);
    }
  });

  it('retains unknown widget definitions and direct widget extensions', () => {
    const widget = { ...configurationWidget(), extra: 'retained' };
    expect(sanitizeWidgetInstance(widget)).toEqual(widget);
    expect(sanitizeWidgetInstance(widget)).not.toBe(widget);
  });

  it('strips schema-level extensions but preserves options and managed metadata', () => {
    const config = {
      ...configurationDashboard(), extra: 'discarded',
      controls: { dateRange: false, extra: true },
      widgets: [{
        ...configurationWidget(), extra: true, managed: true, managedKey: 'synthetic-key',
        layout: { x: 11, y: 2, w: 3, h: 2, extra: true },
        visibility: [{ type: 'vehicle-connection' as const, value: 'plugged' as const, extra: true }],
      }],
    };
    const original = structuredClone(config);
    const output = sanitizeDashboardConfig(config);
    expect(output).not.toHaveProperty('extra');
    expect(output.controls).toEqual({ dateRange: false });
    expect(output.widgets[0]).toEqual({
      ...configurationWidget(), managed: true, managedKey: 'synthetic-key',
      layout: { x: 9, y: 2, w: 3, h: 2 },
    });
    expect(sanitizeDashboardConfig(output)).toEqual(output);
    expect(config).toEqual(original);
  });

  it.each([
    { x: -1 }, { x: 12 }, { y: -1 }, { w: 0 }, { w: 13 }, { h: 0 }, { h: 1.5 },
    { w: NaN }, { h: Infinity },
  ])('rejects invalid saved geometry before it can be clamped: %j', (patch) => {
    const config = configurationDashboard();
    config.widgets[0]!.layout = { ...config.widgets[0]!.layout, ...patch };
    expect(() => sanitizeDashboardConfig(config)).toThrow();
  });

  it('preserves valid layout invariants and idempotence across the grid', () => {
    for (let x = -2; x <= 14; x += 0.5) {
      for (let w = -2; w <= 14; w += 0.5) {
        const output = sanitizeWidgetLayout({ x, y: 0, w, h: 2 });
        expect(output.x).toBeGreaterThanOrEqual(0);
        expect(output.x + output.w).toBeLessThanOrEqual(12);
        expect(output.w).toBeGreaterThanOrEqual(1);
        expect(sanitizeWidgetLayout(output)).toEqual(output);
      }
    }
  });
});

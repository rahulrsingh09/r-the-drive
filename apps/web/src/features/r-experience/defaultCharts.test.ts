import { describe, expect, it } from 'vitest';
import { DEFAULT_DASHBOARDS, DashboardConfigSchema, getChartDefinition, getDefaultBySlug } from '@panda/dashboards';

describe('default graph coverage', () => {
  it('exposes every overview graph type without requiring a catalog selection', () => {
    const pinned = DEFAULT_DASHBOARDS.flatMap(dashboard => dashboard.widgets)
      .filter(widget => widget.componentType === 'chart' && widget.options?.showPicker === false);
    const visibleTypes = new Set(pinned.map(widget => widget.options!.chartId));
    const catalog = getDefaultBySlug('dashboard')!.widgets.find(widget => widget.managedKey === 'overview.chart-catalog')!;
    expect(catalog.options!.chartIds).toEqual(expect.arrayContaining([...visibleTypes].filter(id => id !== 'soc-history')));
    for (const chartId of catalog.options!.chartIds as string[]) expect(visibleTypes.has(chartId), chartId).toBe(true);
    for (const widget of pinned) {
      expect(widget.managed).not.toBe(true);
      expect(widget.options!.chartIds).toEqual([widget.options!.chartId]);
      expect(getChartDefinition(widget.options!.chartId as string)).toBeDefined();
    }
  });

  it('keeps defaults valid, with distinct IDs and nonoverlapping default graph positions', () => {
    for (const dashboard of DEFAULT_DASHBOARDS) {
      expect(DashboardConfigSchema.safeParse(dashboard).success, dashboard.slug).toBe(true);
      expect(new Set(dashboard.widgets.map(widget => widget.id)).size).toBe(dashboard.widgets.length);
      const graphs = dashboard.widgets.filter(widget => widget.componentType === 'chart');
      for (const graph of graphs) {
        for (const other of dashboard.widgets.filter(widget => widget.id !== graph.id)) {
          const a = graph.layout; const b = other.layout;
          expect(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y,
            `${dashboard.slug}: ${graph.id} overlaps ${other.id}`).toBe(true);
        }
      }
    }
    expect(getDefaultBySlug('dashboard')!.widgets.find(widget => widget.id.endsWith('000000000002'))).toMatchObject({
      definitionId: 'trip_miles', title: 'Total trip distance', options: { valueMode: 'sum' },
    });
  });
});

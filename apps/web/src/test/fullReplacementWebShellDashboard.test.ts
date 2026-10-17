import { describe, expect, it } from 'vitest';
import type { DashboardConfig } from '@panda/dashboards';
import { dashboardHeightEdit, resolveShellDashboard } from '../components/dashboard/dashboardShellModel';

const dashboard: DashboardConfig = {
  schemaVersion: 2, id: 'synthetic-dashboard', slug: 'battery', name: 'Battery', isDefault: true, isLocked: false, ownerId: null,
  controls: { dateRange: true }, widgets: [{ id: 'chart', componentType: 'chart', definitionId: 'catalog', layout: { x: 0, y: 0, w: 12, h: 5 } }],
};

describe('web shell dashboard resolution', () => {
  it('does not show a bundled fallback for an unavailable exact dashboard or unresolved editing session', () => {
    const input = { exact: true, editing: false, authReady: true, slug: 'battery', response: undefined, fallback: dashboard, loading: false, failed: false };
    expect(resolveShellDashboard(input)).toEqual({ saved: undefined, unavailable: true });
    expect(resolveShellDashboard({ ...input, exact: false })).toEqual({ saved: dashboard, unavailable: false });
    expect(resolveShellDashboard({ ...input, exact: false, editing: true, authReady: false })).toEqual({ saved: undefined, unavailable: false });
    expect(resolveShellDashboard({ ...input, response: { ...dashboard, slug: 'other' } })).toEqual({ saved: undefined, unavailable: true });
  });

  it('only changes the first matching widget, clamps finite heights, and retains references for no-ops', () => {
    expect(dashboardHeightEdit('chart', NaN, dashboard)(null)).toBeNull();
    expect(dashboardHeightEdit('missing', 12, dashboard)(null)).toBeNull();
    expect(dashboardHeightEdit('chart', 5, dashboard)(null)).toBeNull();
    expect(dashboardHeightEdit('chart', 5, dashboard)(dashboard)).toBe(dashboard);
    const source = { ...dashboard, widgets: [...dashboard.widgets, ...dashboard.widgets] };
    const resized = dashboardHeightEdit('chart', 100, source)(null)!;
    expect(resized.widgets.map(widget => widget.layout.h)).toEqual([20, 5]);
    expect(source.widgets.map(widget => widget.layout.h)).toEqual([5, 5]);
    expect(dashboardHeightEdit('chart', -10, dashboard)(null)!.widgets[0]!.layout.h).toBe(1);
  });
});

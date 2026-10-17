import { describe, expect, it } from 'vitest';
import type { DashboardConfig } from '@panda/dashboards';
import { resolveShellDashboard } from '../components/dashboard/dashboardShellModel';

const dashboard: DashboardConfig = {
  schemaVersion: 2, id: 'synthetic-dashboard', slug: 'battery', name: 'Battery', isDefault: true, isLocked: false, ownerId: null,
  controls: { dateRange: true }, widgets: [{ id: 'chart', componentType: 'chart', definitionId: 'catalog', layout: { x: 0, y: 0, w: 12, h: 5 } }],
};

describe('web shell dashboard resolution', () => {
  it('does not show a bundled fallback for an unavailable exact dashboard', () => {
    const input = { exact: true, slug: 'battery', response: undefined, fallback: dashboard, loading: false, failed: false };
    expect(resolveShellDashboard(input)).toEqual({ saved: undefined, unavailable: true });
    expect(resolveShellDashboard({ ...input, exact: false })).toEqual({ saved: dashboard, unavailable: false });
    expect(resolveShellDashboard({ ...input, response: { ...dashboard, slug: 'other' } })).toEqual({ saved: undefined, unavailable: true });
  });

  it('returns the saved custom object without merging bundled widget layout or controls', () => {
    const saved: DashboardConfig = {
      ...dashboard, isDefault: false, ownerId: 'saved-owner', id: 'saved-id',
      controls: { dateRange: false },
      widgets: [{ ...dashboard.widgets[0]!, layout: { x: 5, y: 7, w: 4, h: 3 } }],
    };
    const before = JSON.stringify(saved);
    const result = resolveShellDashboard({
      exact: true, slug: saved.slug, response: saved, fallback: dashboard, loading: false, failed: false,
    });
    expect(result.saved).toBe(saved);
    expect(result.unavailable).toBe(false);
    expect(JSON.stringify(saved)).toBe(before);
    expect(result.saved?.widgets[0]?.layout).toEqual({ x: 5, y: 7, w: 4, h: 3 });
  });
});

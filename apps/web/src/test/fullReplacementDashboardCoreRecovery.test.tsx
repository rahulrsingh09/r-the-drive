import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DashboardRenderer } from '../../../../packages/dashboards/src/DashboardRenderer';
import { registerWidget, type WidgetCtx } from '../../../../packages/dashboards/src/registry';
import { normalizeLegacyBundledChartRecord } from '../../../../packages/dashboards/src/charts/legacyBaseline';
import { resolveIconId } from '../../../../packages/dashboards/src/editor/iconMigration';
import { BUNDLED_CHART_DEFINITIONS } from '../../../../packages/dashboards/src/charts/defaults';
import { normalizeDashboardConfig } from '../../../../packages/dashboards/src/api';
import type { ChartRecord } from '@panda/types';

vi.mock('@panda/hooks', () => ({
  useAuth: () => ({ defaultVehicleId: null, isAdmin: true }),
  useCurrentVehicleStatus: () => ({ data: undefined }),
}));
vi.mock('../../../../packages/dashboards/src/dashboardData', () => ({
  DashboardDataProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  collectDashboardDataRequirements: () => ({}),
}));

describe('dashboard core recovery boundaries', () => {
  it('drops legacy scenario input while retaining live widget preference and measurement callbacks', () => {
    registerWidget({
      componentType: 'custom', definitionId: 'recovery-context', title: 'Context',
      defaultSize: { w: 3, h: 2 }, minSize: { w: 1, h: 1 },
      component: ({ ctx, instance }) => <div>
        <span>{ctx.visibilityState ? 'Scenario override' : 'Live status'}</span>
        <button onClick={() => ctx.updateWidgetOptions?.(instance.id, { unit: 'mi' })}>Preference</button>
        <button onClick={() => ctx.updateWidgetLayout?.(instance.id, 4)}>Measure</button>
      </div>,
    });
    const ctx: WidgetCtx = Object.freeze({
      vehicleId: null, from: null, to: null,
      visibilityState: { 'vehicle-connection': 'plugged' as const },
      updateWidgetOptions: vi.fn(), updateWidgetLayout: vi.fn(),
    });
    const onConfigChange = vi.fn();
    render(<DashboardRenderer mode="edit" ctx={ctx} onConfigChange={onConfigChange} config={{
      schemaVersion: 2, id: 'recovery-saved', slug: 'recovery-saved', name: 'Saved',
      ownerId: 'synthetic-owner', isDefault: false, isLocked: false, controls: { dateRange: true },
      widgets: [{ id: 'recovery-widget', componentType: 'custom', definitionId: 'recovery-context', layout: { x: 7, y: 9, w: 3, h: 2 } }],
    }} />);
    expect(screen.queryByText('Scenario override')).toBeNull();
    expect(screen.getByText('Live status')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Preference' }));
    fireEvent.click(screen.getByRole('button', { name: 'Measure' }));
    expect(ctx.updateWidgetOptions).toHaveBeenCalledWith('recovery-widget', { unit: 'mi' });
    expect(ctx.updateWidgetLayout).toHaveBeenCalledWith('recovery-widget', 4);
    expect(ctx.visibilityState).toEqual({ 'vehicle-connection': 'plugged' });
    expect(onConfigChange).not.toHaveBeenCalled();
  });

  it.each(['constructor', 'toString', '__proto__'])('leaves unrelated saved chart %s unchanged', (slug) => {
    const record: ChartRecord = {
      id: 'synthetic-chart', ownerId: null, slug, name: 'Saved',
      isDefault: false, isLocked: false, isEnabled: true,
      config: structuredClone(BUNDLED_CHART_DEFINITIONS[0]!),
    };
    expect(normalizeLegacyBundledChartRecord(record)).toBe(record);
  });

  it.each(['constructor', 'toString', '__proto__'])('treats inherited icon key %s as unknown', (name) => {
    expect(resolveIconId(name, 'lucide:car')).toBe('lucide:car');
  });

  it.each(['constructor', 'toString', '__proto__'])('preserves a saved widget with unknown legacy stat %s', (stat) => {
    const widget = {
      id: '11111111-1111-4111-8111-111111111111', componentType: 'custom', definitionId: 'trips.stat',
      layout: { x: 2, y: 3, w: 4, h: 5 }, options: { stat, other: 'preserved' },
    };
    const config = {
      schemaVersion: 2, id: '22222222-2222-4222-8222-222222222222', slug: 'personal', name: 'Saved',
      ownerId: null, isDefault: false, isLocked: false, controls: { dateRange: true }, widgets: [widget],
    };
    expect(normalizeDashboardConfig(config).widgets).toEqual([widget]);
  });
});

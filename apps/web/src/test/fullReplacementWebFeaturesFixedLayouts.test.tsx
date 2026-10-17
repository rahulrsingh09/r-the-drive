import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ChartDefinitionV1Schema, getBundledChartDefinition } from '@panda/dashboards';
import type { ChartRecord } from '@panda/types';
import { chartDraftInitial, chartDraftTransition } from '../features/charts/editor/chartDraft';
import { DashboardLibrarySection } from '../features/settings/DashboardLibrarySection';
import { unassignedChartCopy } from '../features/settings/charts/chartUpdateCommand';
import { vehicleSettingsCapabilities } from '../features/settings/settingsVehicleModel';

function savedChart(): ChartRecord {
  return {
    id: 'saved-chart', slug: 'soc-history', name: 'Saved chart', ownerId: 'synthetic',
    isDefault: false, isLocked: false, isEnabled: true,
    config: ChartDefinitionV1Schema.parse(getBundledChartDefinition('soc-history')),
  } as ChartRecord;
}

describe('fixed dashboard capability boundary', () => {
  it('preserves capture permissions and explicit demo metadata precedence', () => {
    const vehicle = { membership_role: 'owner' as const, rivian_vehicle_id: 'synthetic', is_demo: false };
    expect(vehicleSettingsCapabilities(vehicle)).toMatchObject({ canCaptureIngestion: true, canManageMembers: true });
    expect(vehicleSettingsCapabilities({ ...vehicle, membership_role: 'manager' })).toMatchObject({ canCaptureIngestion: true, canManageMembers: false });
    expect(vehicleSettingsCapabilities({ ...vehicle, membership_role: 'viewer' }).canCaptureIngestion).toBe(false);
    expect(vehicleSettingsCapabilities({ rivian_vehicle_id: 'synthetic', is_demo: false }).canCaptureIngestion).toBe(false);
    expect(vehicleSettingsCapabilities({ ...vehicle, is_demo: true }).canCaptureIngestion).toBe(false);
    const missingDemoFlag = { membership_role: 'owner', rivian_vehicle_id: 'demo-synthetic' } as Parameters<typeof vehicleSettingsCapabilities>[0];
    expect(vehicleSettingsCapabilities(missingDemoFlag).canCaptureIngestion).toBe(false);
    expect(vehicleSettingsCapabilities({ ...vehicle, rivian_vehicle_id: 'demo-synthetic' }).canCaptureIngestion).toBe(true);
  });

  it('keeps saved dashboard geometry and exposes only open/export actions', () => {
    const personal = {
      id: 'saved-layout', slug: 'dashboard', name: 'Personal layout', ownerId: 'synthetic',
      isDefault: false, isLocked: false, widgets: [{ id: 'saved-widget', type: 'chart',
        layout: { x: 4, y: 8, w: 6, h: 5 }, config: { chartId: 'saved-chart' } }],
    };
    const defaults = { ...personal, id: 'default-layout', name: 'Default layout', ownerId: null, isDefault: true };
    const dashboards = [defaults, personal];
    const snapshot = structuredClone(dashboards);
    const open = vi.fn(), download = vi.fn();
    render(<DashboardLibrarySection dashboards={dashboards as never} isLoading={false} onOpen={open} onExport={download} />);
    expect(screen.getAllByRole('button').map((button) => button.textContent)).toEqual(['Open default', 'Export', 'Open', 'Export']);
    expect(screen.queryByRole('switch')).not.toBeInTheDocument();
    expect(screen.getAllByText('Active for you')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: /^Open$/ }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Export' })[1]!);
    expect(open).toHaveBeenCalledWith(personal);
    expect(download).toHaveBeenCalledWith(personal);
    expect(dashboards).toEqual(snapshot);
  });

  it('rejects advanced JSON placement changes while retaining chart definition edits', () => {
    const initial = chartDraftInitial(savedChart());
    const requested = { ...initial.draft!.config, placements: [{ dashboardSlug: 'another-layout' }] };
    const rejected = chartDraftTransition(initial, { type: 'json', text: JSON.stringify(requested) });
    expect(rejected.draft).toBe(initial.draft);
    expect(rejected.advancedError?.message).toContain('Dashboard assignments are fixed');
    const accepted = chartDraftTransition(rejected, {
      type: 'json', text: JSON.stringify({ ...initial.draft!.config, display: { ...initial.draft!.config.display, grid: false } }),
    });
    expect(accepted.advancedError).toBeNull();
    expect(accepted.draft?.config.display.grid).toBe(false);
    expect(accepted.draft?.config.placements).toEqual(initial.draft?.config.placements);
  });

  it('removes assignments from imported or copied definitions without mutating their source', () => {
    const source = savedChart();
    const snapshot = structuredClone(source);
    const copy = unassignedChartCopy(source);
    expect(copy.config.placements).toEqual([]);
    expect(copy.config.series).toEqual(source.config.series);
    expect(source).toEqual(snapshot);
    expect(source.config.placements.length).toBeGreaterThan(0);
  });
});

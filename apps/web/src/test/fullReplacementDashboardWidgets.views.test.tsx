import React from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
const fixture = vi.hoisted(() => ({
  demo: false,
  healthLoading: false,
  statusLoading: false,
  health: {
    battery_health_pct: 90,
    estimated_degradation_pct: 10,
    usable_now_kwh: 120,
    usable_new_kwh: 133.3,
    charge_count: 6,
    charging_cycles: 0,
    total_energy_added_kwh: 240.25,
    charging_efficiency_pct: 92.5,
  },
  summary: {
    session_count: 6,
    total_energy_kwh: 240.25,
    total_cost_usd: 40,
    home_kwh: 120,
    away_kwh: 80,
    unknown_location_kwh: 40,
    ac_kwh: 160,
    dc_kwh: 80,
    charging_cycles: 0,
    charging_efficiency_pct: 92.5,
    max_charge_rate_kw: 150.2,
    max_charge_limit_pct: 85,
  },
  schedule: {
    enabled: true,
    start_time_minutes: 1320,
    duration_minutes: 240,
    amperage: 32,
    week_days: ['Monday'],
  },
  writes: vi.fn(),
}));
vi.mock('@panda/hooks', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@panda/hooks')>()),
  VEHICLE_CONTROLS_ENABLED: true,
  useVehicles: () => ({ data: [{ id: 'synthetic', is_demo: fixture.demo }] }),
  useCurrentVehicleStatus: () => ({
    data: { range_miles: 300, battery_level: 50 },
    isLoading: fixture.statusLoading,
  }),
  useBatteryHealth: () => ({ data: fixture.health, isLoading: fixture.healthLoading }),
  useChargingSummary: () => ({ data: fixture.summary, isLoading: false }),
  useChargingSchedule: () => ({ data: fixture.schedule, isLoading: false }),
  useDepartureSchedules: () => ({
    data: [
      {
        id: 'departure',
        rivian_schedule_id: 'remote',
        name: 'Morning',
        enabled: true,
        occurrence: { days: ['Monday'], time_minutes: 450 },
        comfort_settings: { cabin_temp_c: 21 },
      },
    ],
    isLoading: false,
  }),
  useUpdateChargingSchedule: () => ({ mutateAsync: fixture.writes, isPending: false }),
  useCreateDepartureSchedule: () => ({ mutateAsync: fixture.writes, isPending: false }),
  useUpdateDepartureSchedule: () => ({ mutateAsync: fixture.writes, isPending: false }),
  useDeleteDepartureSchedule: () => ({ mutateAsync: fixture.writes, isPending: false }),
}));
import { getWidget } from '../../../../packages/dashboards/src/registry';
import '../../../../packages/dashboards/src/widgets/battery/BatteryStatWidget';
import '../../../../packages/dashboards/src/widgets/charging/ChargingStatWidget';
import '../../../../packages/dashboards/src/widgets/charging/ChargingScheduleEditorWidget';
import '../../../../packages/dashboards/src/widgets/charging/DepartureSchedulesWidget';
function view(type: 'custom' | 'battery' | 'charging', id: string) {
  const Component = getWidget(type, id)!.component;
  return render(
    <Component
      instance={{
        id,
        componentType: type,
        definitionId: id,
        options: {},
        layout: { x: 0, y: 0, w: 4, h: 4 },
      }}
      ctx={{ vehicleId: 'synthetic', from: null, to: null }}
    />
  );
}
beforeEach(() => {
  fixture.demo = false;
  fixture.healthLoading = false;
  fixture.statusLoading = false;
  fixture.writes.mockReset().mockResolvedValue({});
  localStorage.clear();
});
afterEach(cleanup);
describe('widget view contracts', () => {
  it.each([
    ['health_pct', '90.0%'],
    ['usable_capacity', '120.0 kWh/133.3 kWh'],
    ['max_range', '600 mi/667 mi'],
    ['charge_count', '6'],
    ['charging_cycles', '6'],
    ['energy_added', '240.3 kWh'],
    ['charge_efficiency', '92.5%'],
  ])('keeps battery %s readout', (id, text) => {
    view('battery', id);
    expect(screen.getByTestId('battery-stat-chip')).toHaveTextContent(text);
  });
  it.each([
    ['sessions', '6'],
    ['total_energy', '240.3 kWh'],
    ['total_cost', '$40.00'],
    ['avg_session', '40.0 kWh'],
    ['charging_cycles', '0'],
    ['charge_efficiency', '92.5%'],
    ['max_charge_rate', '150.2 kW'],
    ['max_charge_limit', '85%'],
    ['home_share', '50%'],
    ['dc_share', '33%'],
  ])('keeps charging %s readout', (id, text) => {
    view('charging', id);
    expect(screen.getByTestId('charging-stat-chip')).toHaveTextContent(text);
  });
  it('gates only the max-range stat on status loading', () => {
    fixture.statusLoading = true;
    view('battery', 'max_range');
    expect(screen.getByText('…')).toBeVisible();
    cleanup();
    view('battery', 'health_pct');
    expect(screen.getByText('90.0%')).toBeVisible();
  });
  it('edits, saves and cancels charging schedule drafts', async () => {
    view('custom', 'charging.schedule.editor');
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    fireEvent.change(screen.getByDisplayValue('22:00'), { target: { value: '23:30' } });
    fireEvent.click(screen.getByRole('button', { name: 'Tu' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(fixture.writes).toHaveBeenCalledWith({
        enabled: true,
        start_time_minutes: 1410,
        duration_minutes: 240,
        amperage: 32,
        week_days: ['Monday', 'Tuesday'],
      })
    );
    await waitFor(() => expect(screen.getByRole('button', { name: 'Edit' })).toBeVisible());
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }));
    fireEvent.change(screen.getByDisplayValue('23:30'), { target: { value: '20:00' } });
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByText('22:00')).toBeVisible();
    expect(fixture.writes).toHaveBeenCalledTimes(1);
  });
  it('uses provider schedule identity for toggles and deletes', async () => {
    view('custom', 'charging.departure.schedules');
    fireEvent.click(screen.getByRole('button', { name: 'Disable schedule' }));
    await waitFor(() =>
      expect(fixture.writes).toHaveBeenCalledWith({
        scheduleId: 'remote',
        body: {
          name: 'Morning',
          enabled: false,
          occurrence: { days: ['Monday'], time_minutes: 450 },
          comfort_settings: { cabin_temp_c: 21 },
        },
      })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete schedule' }));
    await waitFor(() => expect(fixture.writes).toHaveBeenCalledWith('remote'));
  });
  it.each(['charging.schedule.editor', 'charging.departure.schedules'])(
    'denies demo writes for %s even when controls are enabled',
    (id) => {
      fixture.demo = true;
      view('custom', id);
      expect(screen.getByText('Schedules are read-only for demos')).toBeVisible();
      expect(screen.queryByRole('button')).toBeNull();
      expect(fixture.writes).not.toHaveBeenCalled();
    }
  );
});

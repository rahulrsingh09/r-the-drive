import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const writes = vi.hoisted(() => vi.fn());
const schedule = vi.hoisted(() => ({ enabled: true, start_time_minutes: 1320, duration_minutes: 240, amperage: 32, week_days: ['Monday'] }));
vi.mock('@panda/hooks', async (importOriginal) => ({
  ...await importOriginal<typeof import('@panda/hooks')>(),
  useVehicles: () => ({ data: [{ id: 'vehicle-1', is_demo: false, membership_role: 'owner' }] }),
  useChargingSchedule: () => ({
    data: schedule,
    isLoading: false,
  }),
  useDepartureSchedules: () => ({
    data: [{
      id: 'departure-1', rivian_schedule_id: 'remote-1', name: 'Morning commute', enabled: true,
      occurrence: { days: ['Monday'], time_minutes: 450 }, comfort_settings: { cabin_temp_c: 21 },
    }],
    isLoading: false,
  }),
  useUpdateChargingSchedule: () => ({ mutateAsync: writes }),
  useCreateDepartureSchedule: () => ({ mutateAsync: writes }),
  useUpdateDepartureSchedule: () => ({ mutateAsync: writes }),
  useDeleteDepartureSchedule: () => ({ mutateAsync: writes }),
}));

import { getWidget } from '../../../../packages/dashboards/src/registry';
import '../../../../packages/dashboards/src/widgets/charging/ChargingScheduleEditorWidget';
import '../../../../packages/dashboards/src/widgets/charging/DepartureSchedulesWidget';

afterEach(() => { cleanup(); writes.mockClear(); });

describe('telemetry-only schedule views', () => {
  it.each([375, 1280])('retains schedule telemetry without vehicle controls at width %i', (width) => {
    Object.defineProperty(window, 'innerWidth', { value: width, configurable: true });
    for (const definitionId of ['charging.schedule.editor', 'charging.departure.schedules']) {
      const Component = getWidget('custom', definitionId)!.component;
      render(<Component
        instance={{ id: definitionId, componentType: 'custom', definitionId, options: {}, layout: { x: 0, y: 0, w: 5, h: 8 } }}
        ctx={{ vehicleId: 'vehicle-1', from: null, to: null }}
      />);
    }
    expect(screen.getByText('32A')).toBeInTheDocument();
    expect(screen.getByText('22:00')).toBeInTheDocument();
    expect(screen.getByText('Morning commute')).toBeInTheDocument();
    expect(screen.getAllByText(/Read-only telemetry/)).toHaveLength(2);
    expect(screen.queryByRole('button', { name: /^(Edit|New|Save|Set Up Schedule|Delete schedule)$/ })).not.toBeInTheDocument();
    const toggle = screen.getByRole('button', { name: 'Schedule enabled (read-only)' });
    expect(toggle).toBeDisabled();
    fireEvent.click(toggle);
    fireEvent.click(screen.getByRole('button', { name: 'Show comfort settings' }));
    expect(screen.getByText('Cabin: 21°C')).toBeInTheDocument();
    expect(writes).not.toHaveBeenCalled();
  });
});

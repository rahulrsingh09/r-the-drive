import React from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  role: 'owner' as 'owner' | 'manager' | 'viewer',
  gaps: [] as unknown[],
  parallaxQuiet: null as null | { last_frame_at: string; moved_at: string },
  recover: vi.fn(),
  recoverCharges: vi.fn(),
  backfill: { status: null as string | null, history_backfilled_at: null as string | null },
  chargeGaps: { data: undefined as unknown, isError: false },
  status: {
    doors_locked: true,
    door_front_left_closed: true,
    door_front_right_closed: true,
    door_rear_left_closed: true,
    door_rear_right_closed: true,
    closure_frunk_closed: true,
    closure_liftgate_closed: true,
    odometer_miles: 1611,
    battery_limit: 70,
    ota_current_version: '2026.36.42',
    tire_fl_psi: 40.6, tire_fr_psi: 41, tire_rl_psi: 40.6, tire_rr_psi: 40.6,
    service_mode: false,
    charger_derate_active: true,
    last_updated: new Date().toISOString(),
    location_ts: new Date().toISOString(),
    battery_level_ts: new Date().toISOString(),
    field_availability: {},
  } as Record<string, unknown>,
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, children, ...props }: { to: string; children: React.ReactNode }) => <a href={to} {...props}>{children}</a>,
}));
vi.mock('../features/r-experience/RAppLayout', () => ({
  RAppLayout: ({ children }: { children: React.ReactNode }) => <main>{children}</main>,
  RPageActions: () => null,
}));
vi.mock('../components/layout/NoVehicleState', () => ({
  NoVehicleState: ({ title }: { title: string }) => <p>{title}</p>,
}));
vi.mock('../components/feedback/toast', () => ({ emitToast: vi.fn() }));
vi.mock('../features/r-experience/RVehicleOrbit', () => ({
  RVehicleTurn: ({ label }: { label: string }) => <div role="slider" aria-label={label} aria-valuenow={260} />,
}));
vi.mock('@panda/hooks', () => ({
  useResolvedVehicleSelection: () => ({
    authReady: true,
    vehicleSelectionReady: true,
    effectiveVehicleId: 'veh-1',
    vehicles: [{
      id: 'veh-1', model: 'R2', trim: 'Performance', color: 'Catalina Cove', wheel_option: '21" Liquid Tungsten All-Season',
      display_name: 'R2', membership_role: mocks.role, target_tire_pressure_psi: 40, created_at: '2026-10-06T12:00:00Z',
    }],
  }),
  useDegradation: () => ({ data: [
    { ts: '2026-10-06T12:00:00Z', usable_kwh: 88 },
    { ts: '2026-11-07T12:00:00Z', usable_kwh: 87.8 },
  ] }),
  useLifetimeTrips: () => ({ isPending: false, data: [
    { id: 't2', started_at: '2026-10-08T00:00:00Z', distance_mi: 9, start_odometer_mi: 1600.5, end_odometer_mi: 1609.5 },
    { id: 't1', started_at: '2026-10-07T00:00:00Z', distance_mi: 10, start_odometer_mi: 1590, end_odometer_mi: 1600 },
  ] }),
  useChargeGaps: () => mocks.chargeGaps,
  useChargeHistoryBackfillStatus: () => ({ data: { vehicle_id: 'veh-1', local_session_count: 3, rivian_session_count: 3, ...mocks.backfill } }),
  useRecoverMissedCharges: () => ({ mutate: mocks.recoverCharges, isPending: false }),
  useVehicleHealth: () => ({
    isLoading: false,
    data: {
      vehicle: { model: 'R2', name: 'R2' },
      runtime: { worker_health: 'connected', last_event_at: new Date().toISOString() },
      latest: { ts: new Date().toISOString(), twelve_volt_health: 'good', hv_thermal_event: 'none' },
      tires: null,
      closures: null,
      software_history: [{ version: '2026.35', installed_at: '2026-10-20T00:00:00Z', observed_until: null }],
      current_software_version: '2026.35',
      thermal_events_30d: 2,
      extended_telemetry: { parallax: { status: 'connected', last_frame_at: new Date().toISOString() } },
    },
  }),
  useCurrentVehicleStatus: () => ({ data: mocks.status }),
  useTripGaps: () => ({ data: { gaps: mocks.gaps, parallax_quiet: mocks.parallaxQuiet, lookback_days: 7 } }),
  useRecoverTripGap: () => ({ mutate: mocks.recover, isPending: false }),
}));

import { RVehicleHealth } from '../features/r-experience/RVehicleHealth';
import { emitToast } from '../components/feedback/toast';
import { RMissedDriveBanner } from '../features/r-experience/RMissedDrives';

const gap = {
  id: '0123456789abcdef0123456789abcdef',
  started_at: '2026-10-26T04:13:21Z',
  ended_at: '2026-10-26T05:22:25Z',
  distance_miles: 42.87,
  duration_seconds: 4144,
  soc_start: 70.2,
  soc_end: 51.9,
};

describe('R vehicle health', () => {
  beforeEach(() => {
    localStorage.clear();
    mocks.role = 'owner';
    mocks.gaps = [];
    mocks.parallaxQuiet = null;
    mocks.recover.mockReset();
    mocks.recoverCharges.mockReset();
    mocks.backfill = { status: null, history_backfilled_at: null };
    mocks.chargeGaps = { data: { vehicle_id: 'veh-1', lookback_days: 30, gaps: [], parked_periods_checked: 69,
      daily_samples: [{ day: '2026-11-06', samples: 1200 }, { day: '2026-11-07', samples: 90 }, { day: '2026-11-08', samples: 1300 }] }, isError: false };
  });

  const row = (label: string) => screen.getByText(label, { selector: '.r-car-hrow span' }).closest('li')!;

  it('shows the car turn, spec grid, tires and closures', () => {
    render(<RVehicleHealth />);
    expect(screen.getByRole('heading', { level: 1, name: 'Car' })).toBeInTheDocument();
    expect(screen.getByText('R2 Performance · Catalina Cove · 21" Tungsten wheels')).toBeInTheDocument();
    expect(screen.getByRole('slider', { name: /Turn the car/ })).toBeInTheDocument();
    expect(screen.getByText('Odometer').nextSibling).toHaveTextContent('1,611 mi');
    expect(screen.getByText('Charge limit').nextSibling).toHaveTextContent('70%');
    expect(screen.getByText('Battery health').nextSibling).toHaveTextContent('99.8%');
    const tires = screen.getByRole('region', { name: 'Tires' });
    expect(within(tires).getByText('Front right').parentElement).toHaveTextContent('41.0 psi');
    expect(within(tires).getAllByText('OK')).toHaveLength(4);
    const closures = screen.getByRole('region', { name: 'Closures' });
    expect(within(closures).getByText('Doors').parentElement).toHaveTextContent('Locked');
    expect(within(closures).getByText('Windows').parentElement).toHaveTextContent('Not reported');
    expect(screen.queryByText('Possible missed drive')).not.toBeInTheDocument();
  });

  it('lists vehicle health with Not reported for missing fields', () => {
    render(<RVehicleHealth />);
    const list = screen.getByRole('region', { name: 'Vehicle health' });
    expect(within(list).getAllByRole('listitem').map(item => item.textContent)).toEqual([
      'Battery thermal2 events in 30 daysNo event',
      '12 V batteryGood',
      'TiresAll OK',
      'Brake fluidNot reported',
      'Washer fluidNot reported',
      'AlarmNot reported',
      'Service modeOff',
      'Charging power limitLast report. It only matters while charging.Reported on',
      'Software2026.36.42Up to date',
    ]);
  });

  it('shows data health from the connection, trips, charge gaps and samples', () => {
    render(<RVehicleHealth />);
    expect(row('Rivian connection')).toHaveTextContent(/Connected/);
    expect(row('Rivian connection')).toHaveTextContent(/Since Oct 6 · last refresh/);
    expect(row('Odometer vs trips')).toHaveTextContent('90.5% covered');
    expect(row('Odometer vs trips')).toHaveTextContent('2.0 mi in 1 short move not in a trip');
    expect(row('Missed charges')).toHaveTextContent('(69 parked periods checked)None found');
    expect(row('Missed drives')).toHaveTextContent('None found');
    expect(screen.getByRole('img', { name: /Data samples per day for the last 3 days\. 1 days had few samples/ })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Reconnect Rivian' })).toHaveAttribute('href', '/settings');
  });

  it('does not say None found when the missed-charge check fails', () => {
    mocks.chargeGaps = { data: undefined, isError: true };
    render(<RVehicleHealth />);
    expect(row('Missed charges')).toHaveTextContent('Not checked');
    expect(screen.queryByRole('img', { name: /samples per day/ })).not.toBeInTheDocument();
  });

  it('counts possible missed charges', () => {
    mocks.chargeGaps = { data: { vehicle_id: 'veh-1', lookback_days: 30, gaps: [
      { id: 'g1', started_at: '2026-11-01T01:00:00Z', ended_at: '2026-11-01T03:00:00Z', soc_start: 40, soc_end: 55 },
    ] }, isError: false };
    render(<RVehicleHealth />);
    expect(row('Missed charges')).toHaveTextContent('1 possible');
  });

  it('recovers missed charges and shows the running state', () => {
    const { unmount } = render(<RVehicleHealth />);
    fireEvent.click(screen.getByRole('button', { name: 'Recover missed charges' }));
    expect(mocks.recoverCharges).toHaveBeenCalled();
    unmount();
    mocks.backfill = { status: 'running', history_backfilled_at: null };
    render(<RVehicleHealth />);
    expect(screen.getByRole('button', { name: 'Recovering…' })).toBeDisabled();
    expect(screen.getByText('Recovering missed charges from Rivian…')).toBeInTheDocument();
  });

  it('shows the software history timeline', () => {
    render(<RVehicleHealth />);
    const history = screen.getByRole('region', { name: 'Software history' });
    expect(within(history).getByRole('listitem')).toHaveTextContent(/2026\.35Current · Oct 20/);
  });

  it('recovers a missed drive only after confirmation', () => {
    mocks.gaps = [gap];
    render(<RVehicleHealth />);
    const card = screen.getByRole('region', { name: 'Possible missed drive' });
    expect(within(card).getByText(/42\.9 mi · 1h 9m · 70% → 52%/)).toBeInTheDocument();
    fireEvent.click(within(card).getByRole('button', { name: 'Recover trip' }));
    expect(mocks.recover).not.toHaveBeenCalled();
    fireEvent.click(within(card).getByRole('button', { name: 'Confirm recovery' }));
    expect(mocks.recover).toHaveBeenCalledWith(gap.id, expect.any(Object));
  });

  it('cancels confirmation without recovering', () => {
    mocks.gaps = [gap];
    render(<RVehicleHealth />);
    fireEvent.click(screen.getByRole('button', { name: 'Recover trip' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByRole('button', { name: 'Recover trip' })).toBeInTheDocument();
    expect(mocks.recover).not.toHaveBeenCalled();
  });

  it('moves keyboard focus between recover, confirm and cancel', () => {
    mocks.gaps = [gap];
    render(<RVehicleHealth />);
    fireEvent.click(screen.getByRole('button', { name: 'Recover trip' }));
    expect(screen.getByRole('button', { name: 'Confirm recovery' })).toHaveFocus();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByRole('button', { name: 'Recover trip' })).toHaveFocus();
  });

  it('explains when the drive is no longer listed', () => {
    mocks.gaps = [gap];
    mocks.recover.mockImplementation((_id: string, options: { onError: (error: unknown) => void }) =>
      options.onError(Object.assign(new Error('404'), { status: 404 })));
    render(<RVehicleHealth />);
    fireEvent.click(screen.getByRole('button', { name: 'Recover trip' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm recovery' }));
    expect(emitToast).toHaveBeenCalledWith(expect.objectContaining({ title: 'Drive no longer listed', variant: 'info' }));
  });

  it('does not offer recovery to viewers', () => {
    mocks.role = 'viewer';
    mocks.gaps = [gap];
    render(<RVehicleHealth />);
    expect(screen.queryByRole('button', { name: 'Recover trip' })).not.toBeInTheDocument();
    expect(screen.getByText('An owner or manager can recover this drive.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Recover missed charges' })).not.toBeInTheDocument();
    expect(screen.getByText('An owner or manager can recover missed charges.')).toBeInTheDocument();
  });

  it('remembers a dismissed gap on this device', () => {
    mocks.gaps = [gap];
    const { unmount } = render(<RVehicleHealth />);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByText('Possible missed drive')).not.toBeInTheDocument();
    unmount();
    render(<RVehicleHealth />);
    expect(screen.queryByText('Possible missed drive')).not.toBeInTheDocument();
  });

  it('warns when the live stream went quiet while the car moved', () => {
    mocks.parallaxQuiet = { last_frame_at: '2026-10-26T00:13:03Z', moved_at: '2026-10-26T04:13:21Z' };
    render(<RVehicleHealth />);
    const notice = screen.getAllByRole('status').find(item => item.classList.contains('r-health-notice'));
    expect(notice).toHaveTextContent(/Rivian’s live stream sent no data after/);
    expect(notice).not.toHaveTextContent(/Parallax/);
  });

  it('links the Trips banner to the recovery card', () => {
    mocks.gaps = [gap];
    render(<RMissedDriveBanner vehicleId="veh-1" />);
    expect(screen.getByRole('link', { name: /1 possible missed drive/ })).toHaveAttribute('href', '/vehicle-health');
  });
});

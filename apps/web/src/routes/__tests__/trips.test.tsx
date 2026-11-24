import React from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@panda/ui/primitives', async () => import('../../test/mockPrimitives'));

const navigate = vi.fn();
vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    useNavigate: () => navigate,
    Link: ({ children, to, params, search: _search, ...rest }: { children: React.ReactNode; to: string; params?: Record<string, string>; search?: unknown }) => (
      <a href={Object.entries(params ?? {}).reduce((path, [key, value]) => path.replace(`$${key}`, value), to)} {...rest}>{children}</a>
    ),
  };
});

vi.mock('../../features/r-experience/RMissedDrives', () => ({ RMissedDriveBanner: () => <div data-testid="missed-drive-banner" /> }));
vi.mock('@panda/ui/charts', () => ({
  RouteReplayMap: ({ progress }: { progress: number | null }) => <div data-testid="route-replay-map" data-progress={progress ?? ''} />,
}));
vi.mock('@panda/ui/hooks', () => ({ useDocumentTheme: () => true }));

const HOME = { lat: 47.6769, lng: -122.2060 };
const now = Date.now();
const iso = (hoursAgo: number) => new Date(now - hoursAgo * 3_600_000).toISOString();

const trips = [
  {
    id: 't1', vehicle_id: 'v1', started_at: iso(2), ended_at: iso(1.5), distance_mi: 12, duration_min: 30, energy_used_kwh: 3.84,
    efficiency_wh_mi: 320, max_speed_mph: 10, drive_mode: 'all_purpose', soc_start: 78, soc_end: 74,
    start_odometer_mi: 1000, end_odometer_mi: 1012, start_lat: HOME.lat, start_lng: HOME.lng, end_lat: 47.61, end_lng: -122.20,
    start_place: null, end_place: null, start_address: null, end_address: null, outside_temp_c: 12,
  },
  {
    id: 't2', vehicle_id: 'v1', started_at: iso(5), ended_at: iso(4.8), distance_mi: 1.2, duration_min: 10, energy_used_kwh: null,
    efficiency_wh_mi: null, max_speed_mph: null, drive_mode: null, soc_start: 78, soc_end: 78,
    start_odometer_mi: 997, end_odometer_mi: 998.2, start_lat: 47.61, start_lng: -122.20, end_lat: HOME.lat, end_lng: HOME.lng,
    start_place: null, end_place: null, start_address: null, end_address: null,
  },
];
const charges: Array<{
  id: string; vehicle_id: string; started_at: string; ended_at: string; location_name: string | null;
  charger_type: string; energy_added_kwh: number; duration_min: number; soc_start: number; soc_end: number;
  peak_power_kw: number; cost_usd: number | null; cost_method: string; location_lat: number; location_lng: number;
  cost_override_mode?: 'automatic' | 'free' | 'manual';
}> = [
  {
    id: 'c1', vehicle_id: 'v1', started_at: iso(20), ended_at: iso(18), location_name: 'Home', charger_type: 'ac', energy_added_kwh: 20,
    duration_min: 120, soc_start: 55, soc_end: 80, peak_power_kw: 11, cost_usd: 3.78, cost_method: 'profile', location_lat: HOME.lat, location_lng: HOME.lng,
  },
  {
    id: 'c2', vehicle_id: 'v1', started_at: iso(30), ended_at: iso(29.5), location_name: null, charger_type: 'dc', energy_added_kwh: 40,
    duration_min: 30, soc_start: 28, soc_end: 72, peak_power_kw: 190, cost_usd: null, cost_method: 'unknown', location_lat: 45.8151, location_lng: -122.7426,
  },
];
const places = [{
  id: 'p-home', name: 'Home', latitude: HOME.lat, longitude: HOME.lng, radius_m: 100, is_home: true, is_work: false, address: null,
  charging: { id: 'cp', name: 'Home', billing_type: 'per_kwh', rate: 0.189119, session_fee: 0, currency: 'USD', timezone: 'UTC', tou_periods: [] },
}];

const mocks = vi.hoisted(() => ({
  api: {
    listTrips: vi.fn(), listChargeSessions: vi.fn(), getTripMap: vi.fn(),
    apiFetch: vi.fn(),
  },
  updateCost: vi.fn(),
}));

vi.mock('@panda/hooks', () => ({
  api: mocks.api,
  useAuthReady: () => true,
  useAuth: (select?: (state: { accessToken: string }) => unknown) => (select ? select({ accessToken: 'token' }) : { accessToken: 'token' }),
  useSavedPlaces: () => ({ data: places }),
  useBasemapConfig: () => ({ data: undefined }),
  useResolvedVehicleSelection: () => ({ vehicles: [{ id: 'v1', battery_capacity_kwh: 88 }] }),
  invalidateChargingData: vi.fn(),
  useUpdateChargeSession: () => ({ mutate: mocks.updateCost, isPending: false }),
  useChargeCurve: () => ({
    isLoading: false,
    data: Array.from({ length: 8 }, (_, index) => ({ minutes_elapsed: index * 4, soc_pct: 28 + index * 6, power_kw: 150 - index * 10, power_method: 'soc_delta' })),
  }),
  useChargeSession: (id: string) => ({ data: charges.find((charge) => charge.id === id), isError: false }),
  useTripDetailData: (id: string) => detailFor(id),
}));

const detailCache = new Map<string, unknown>();
function detailFor(id: string) {
  if (!detailCache.has(id)) detailCache.set(id, {
    isError: false,
    data: {
      trip: trips.find((trip) => trip.id === id),
      sample_interval_seconds: 60,
      samples: Object.fromEntries(['elapsed_s', 'lat', 'lng', 'altitude_m', 'speed_mph', 'power_kw', 'regen_power_kw', 'battery_level', 'outside_temp_c',
        'cabin_temp_c', 'driver_temp_c', 'hvac_active', 'tire_fl_psi', 'tire_fr_psi', 'tire_rl_psi', 'tire_rr_psi'].map((name) => [name,
        Array.from({ length: 31 }, (_, index) => (name === 'elapsed_s' ? index * 60 : name === 'lat' ? HOME.lat - index * 0.002 : name === 'lng' ? HOME.lng
          : name === 'battery_level' ? 78 - index * 0.13 : name === 'altitude_m' ? 30 + index : null))])),
      outside_temperature: { source: 'unavailable', attribution: null, samples: [] },
    },
  });
  return detailCache.get(id);
}

import { ActivityPage } from '../../features/trips/activity/ActivityPage';
import { toActivityCharge } from '../../features/trips/activity/activityModel';

function renderPage(props: Partial<React.ComponentProps<typeof ActivityPage>> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ActivityPage vehicleId="v1" selection={null} search={{}} canManage {...props} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  mocks.api.listTrips.mockResolvedValue({ items: trips, total: trips.length, page: 1, per_page: 200 });
  mocks.api.listChargeSessions.mockResolvedValue({ items: charges, total: charges.length, page: 1, per_page: 200 });
  mocks.api.getTripMap.mockResolvedValue({ routes: [{ trip_id: 't1', tags: [], coordinates: [[HOME.lng, HOME.lat], [-122.203, 47.64], [-122.20, 47.61]] }] });
  mocks.api.apiFetch.mockResolvedValue({ tag: 'home', place_id: 'p-home', updated: 1, session_ids: ['c2'] });
});
afterEach(() => vi.clearAllMocks());

describe('Activity list', () => {
  it('shows the summary and a day-grouped timeline of trips, charges, parked time and odometer gaps', async () => {
    renderPage();
    expect(await screen.findByText('Home → Bellevue')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Activity' })).toBeInTheDocument();
    expect(screen.getByTestId('missed-drive-banner')).toBeInTheDocument();
    const summary = document.querySelector('[data-activity-summary="trips"]')!;
    expect(summary).toHaveTextContent('13mi');
    expect(summary).toHaveTextContent('2 trips');
    expect(screen.getByText('Short trip')).toBeInTheDocument();
    expect(screen.getByText('320 Wh/mi')).toBeInTheDocument();
    expect(screen.getByText('Odometer moved 1.8 mi with no trip recorded')).toBeInTheDocument();
    expect(screen.getByText('Parked 13 h')).toBeInTheDocument();
    expect(screen.getByText('−2.0%')).toBeInTheDocument();
    expect(screen.getByText('$3.78')).toBeInTheDocument();
    expect(screen.getByText('DC fast · Ridgefield')).toBeInTheDocument();
    expect(screen.getByText('Add cost')).toBeInTheDocument();
    expect(screen.getByText('Home → Bellevue').closest('a')).toHaveAttribute('href', '/trips/t1');
  });

  it('shows the charge summary with the Charges filter and the place chip from Insights', async () => {
    renderPage({ search: { place: 'Home' } });
    const summary = await waitFor(() => {
      const element = document.querySelector('[data-activity-summary="charges"]');
      if (!element) throw new Error('no summary');
      return element;
    });
    expect(summary).toHaveTextContent('20kWh');
    expect(summary).toHaveTextContent('1 sessions');
    expect(screen.queryByText('Home → Bellevue')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Show all activity' }));
    expect(navigate).toHaveBeenCalledWith({ to: '/trips', search: {} });
  });

  it('changes the period through the URL', async () => {
    renderPage();
    await screen.findByText('Home → Bellevue');
    fireEvent.change(screen.getByLabelText('Period'), { target: { value: '7' } });
    expect(navigate).toHaveBeenCalledWith({ to: '/trips', search: { p: '7' } });
  });
});

describe('Trip detail', () => {
  beforeEach(() => {
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(performance.now()), 16) as unknown as number);
    vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id));
  });
  afterEach(() => vi.unstubAllGlobals());

  it('opens the trip with position, replay, odometer check, battery chart and details', async () => {
    renderPage({ selection: { kind: 'trip', id: 't1' } });
    expect(await screen.findByRole('heading', { name: 'Home → Bellevue' })).toBeInTheDocument();
    await waitFor(() => expect(document.querySelector('[data-detail-position]')).toHaveTextContent('Trip · 1 of 4'));
    expect(screen.getByLabelText('Older')).toHaveAttribute('href', '/trips/t2');
    expect(document.querySelector('[data-odometer-result]')).toHaveTextContent('Matches the trip');
    expect(screen.getByTestId('trip-efficiency')).toHaveTextContent('320 Wh/mi');
    expect(screen.getByTestId('trip-battery-chart')).toBeInTheDocument();
    const details = document.querySelector('[data-trip-details]')!;
    // The top speed reading (10 mph) is below the average speed (24 mph), so it stays hidden.
    expect(within(details as HTMLElement).getByText('Top speed').nextSibling).toHaveTextContent('–');
    expect(within(details as HTMLElement).getByText('Average speed').nextSibling).toHaveTextContent('24 mph');
    expect(screen.getByText('Bellevue is the nearest town, not an address.')).toBeInTheDocument();
  });

  it('plays, pauses, scrubs and changes the replay speed', async () => {
    renderPage({ selection: { kind: 'trip', id: 't1' } });
    const play = await screen.findByRole('button', { name: 'Play replay' });
    const readout = document.querySelector('[data-replay-readout]')!;
    expect(readout.querySelector('[data-readout="distance"]')).toHaveTextContent('12.0 mi');
    fireEvent.change(screen.getByLabelText('Replay position'), { target: { value: '500' } });
    expect(readout.querySelector('[data-readout="distance"]')).not.toHaveTextContent('12.0 mi');
    expect(readout.querySelector('[data-readout="battery"]')).toHaveTextContent('76%');
    expect(document.querySelector('[data-chart-cursor]')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '240×' }));
    expect(screen.getByRole('button', { name: '240×' })).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(play);
    expect(await screen.findByRole('button', { name: 'Pause replay' })).toBeInTheDocument();
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 60)); });
    fireEvent.click(screen.getByRole('button', { name: 'Pause replay' }));
    expect(screen.getByRole('button', { name: 'Play replay' })).toBeInTheDocument();
  });
});

describe('Charge detail', () => {
  it('shows the place tag, the cost entry and the charge charts', async () => {
    renderPage({ selection: { kind: 'charge', id: 'c2' } });
    expect(await screen.findByRole('heading', { name: 'DC fast · Ridgefield' })).toBeInTheDocument();
    await waitFor(() => expect(document.querySelector('[data-detail-position]')).toHaveTextContent('Charge · 4 of 4'));
    expect(document.querySelector('[data-charge-cost]')).toHaveTextContent('Not set');
    expect(screen.getByText('This place has no rate. Enter what you paid.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Other' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('charge-level-chart')).toBeInTheDocument();
    expect(document.querySelector('[data-power-note]')).toHaveTextContent('Estimated from the battery-level change and a 88.0 kWh pack.');

    fireEvent.change(screen.getByLabelText('Total paid'), { target: { value: '18.40' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocks.updateCost).toHaveBeenCalledWith({ sessionId: 'c2', cost_mode: 'manual', cost_usd: 18.4 }, expect.anything());

    fireEvent.click(screen.getByRole('button', { name: 'Price per kWh' }));
    fireEvent.change(screen.getByLabelText('Price per kWh'), { target: { value: '0.48' } });
    expect(screen.getByText('= $19.20')).toBeInTheDocument();
  });

  it('tags only this charge Home immediately through the place-tag API with session scope', async () => {
    renderPage({ selection: { kind: 'charge', id: 'c2' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Home' }));
    await waitFor(() => expect(mocks.api.apiFetch).toHaveBeenCalledWith(
      'POST',
      '/v1/vehicles/v1/charging-sessions/c2/place-tag',
      { tag: 'home', scope: 'session' },
    ));
  });

  it('allows Mark free ($0) on an individual charge', async () => {
    renderPage({ selection: { kind: 'charge', id: 'c2' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Mark free ($0)' }));
    expect(mocks.updateCost).toHaveBeenCalledWith({ sessionId: 'c2', cost_mode: 'free' }, expect.anything());
  });

  it('allows manual cost editing and Use automatic rate on a Home-tagged charge', async () => {
    charges.push({
      ...charges[0]!,
      id: 'c3',
      cost_method: 'user_manual',
      cost_override_mode: 'manual',
      cost_usd: 4.5,
    });
    try {
      renderPage({ selection: { kind: 'charge', id: 'c3' } });
      expect(await screen.findByRole('button', { name: 'Mark free ($0)' })).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: 'Use automatic rate' }));
      expect(mocks.updateCost).toHaveBeenCalledWith({ sessionId: 'c3', cost_mode: 'automatic' }, expect.anything());
    } finally {
      charges.pop();
    }
  });

  it('keeps a charge with location_override_mode=none in Other even at Home coordinates', () => {
    const converted = toActivityCharge({
      ...(charges[0] as unknown as Parameters<typeof toActivityCharge>[0]),
      location_name: null,
      location_override_mode: 'none',
    }, places as unknown as Parameters<typeof toActivityCharge>[1]);
    expect(converted?.category).toBe('public');
    expect(converted?.savedPlace).toBeNull();
  });

  it('asks for an Office place when none is saved and the tag fails', async () => {
    mocks.api.apiFetch.mockRejectedValueOnce(new Error('Save an Office place first'));
    renderPage({ selection: { kind: 'charge', id: 'c2' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Office' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Set your Office rate in Settings first.');
  });
});

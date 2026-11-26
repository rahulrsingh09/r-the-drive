import React from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { setAppTimezone } from '@panda/ui/lib/dateTime';

const mocks = vi.hoisted(() => ({
  status: null as unknown,
  latestTrip: undefined as unknown,
  latestCharge: undefined as unknown,
  latestLoading: false,
  trips: [] as unknown[],
  charges: [] as unknown[],
  parks: [] as unknown[],
  live: null as unknown,
  curve: [] as unknown[],
  images: null as unknown,
  activeTrip: null as unknown,
  backfilled: false,
  map: vi.fn(),
  statusRefetch: vi.fn(),
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, params, search, children, ...props }: { to: string; params?: Record<string, string>; search?: Record<string, string>; children: React.ReactNode }) => {
    const path = Object.entries(params ?? {}).reduce((href, [key, value]) => href.replace(`$${key}`, value), to);
    return <a href={search ? `${path}?${new URLSearchParams(search)}` : path} {...props}>{children}</a>;
  },
}));

vi.mock('@panda/ui/charts', () => ({
  TripMapChart: (props: { track: unknown[]; activePoint: unknown }) => { mocks.map(props); return <div data-testid="live-map" />; },
}));

vi.mock('../r-experience/orbitFrames', async (importOriginal) => ({
  ...await importOriginal<typeof import('../r-experience/orbitFrames')>(),
  createOrbitFrames: () => ({ get: () => new Promise(() => {}), close: () => {}, setPaint: () => {} }),
}));

vi.mock('@panda/hooks', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@panda/hooks')>();
  const query = (data: unknown, extra: object = {}) => ({ data, isLoading: false, isError: false, isPlaceholderData: false, refetch: vi.fn(), ...extra });
  return {
    ...actual,
    api: {
      vehicleImages: async () => mocks.images,
      getBackfillStatus: async () => ({ vehicle_id: 'owner-vehicle', history_backfilled_at: mocks.backfilled ? '2026-11-07T00:00:00Z' : null, status: null, rivian_session_count: 0, local_session_count: 0 }),
      requestResponse: async () => {
        if (!mocks.activeTrip) throw Object.assign(new Error('Not found'), { status: 404 });
        return { status: 200, json: async () => mocks.activeTrip };
      },
    },
    useAuthReady: () => true,
    useAuth: () => ({ accessToken: 'token' }),
    useCurrentVehicleStatus: () => query(mocks.status, { refetch: mocks.statusRefetch }),
    useSavedPlaces: () => query([{ id: 'p', name: 'Home', latitude: 47.6, longitude: -122.3, radius_m: 100, is_home: true, is_work: false }]),
    useTrips: (_id: string, from: string | null) => from ? query({ items: mocks.trips })
      : query({ items: mocks.latestTrip ? [mocks.latestTrip] : [] }, { isLoading: mocks.latestLoading }),
    useChargeSessions: (_id: string, from: string | null) => from ? query({ items: mocks.charges })
      : query({ items: mocks.latestCharge ? [mocks.latestCharge] : [] }),
    usePhantomDrainPeriods: () => query({ periods: mocks.parks }),
    useTripTrack: () => query([{ lat: 47.6, lng: -122.3 }, { lat: 47.65, lng: -122.2 }]),
    useLiveSession: () => query(mocks.live),
    useChargeCurve: () => query(mocks.curve),
    useBasemapConfig: () => query({ enabled: true, revision: '1', styles: [] }),
    useUserPreferences: () => query({ map_style: 'follow-theme' }),
    useVehicleArtwork: (source: string | null) => ({ src: source, restoring: false, isLoading: false, isError: false }),
    AuthenticatedVehicleArtwork: ({ source, fallbackSource, fallbackProps: _fallbackProps, onFallbackChange: _onFallbackChange, ...props }: React.ImgHTMLAttributes<HTMLImageElement> & {
      source?: string | null; fallbackSource?: string | null; fallbackProps?: unknown; onFallbackChange?: unknown;
    }) => <img {...props} src={source ?? fallbackSource ?? undefined} />,
  };
});

import { NowView } from './NowView';
import { ORBIT_LABEL } from '../r-experience/RVehicleOrbit';
import { EFFICIENCY_UNIT_STORAGE_KEY } from '../r-experience/efficiencyUnit';

const vehicle = { id: 'owner-vehicle', model: 'R2', trim: 'Performance', color: 'Launch Green', display_name: 'Bumble Beee', images: null } as unknown as Vehicle;
const art = (placement: string, file: string) => ({ placement, design: 'light', size: 'large', resolution: null, url: `/art/${file}.png`, metadata: { source: 'vehicle:photo' } });
const ownerImages = {
  all: [art('rear', 'rear'), art('side', 'side'), art('front', 'front'), art('front-three-quarter', 'fq')],
  side: { light: '/art/side.png', dark: '/art/side.png' },
};
const parked = {
  vehicle_id: 'owner-vehicle', battery_level: 62, battery_limit: 70, range_miles: 206, odometer_miles: 12486,
  power_state: 'ready', charger_state: 'Disconnected', charger_status: 'chrgr_sts_not_connected', speed_mph: 0,
  latitude: 47.6, longitude: -122.3, doors_locked: true, cabin_temp_c: 17.2,
  tire_fl_psi: 41, tire_fr_psi: 41, tire_rl_psi: 41.2, tire_rr_psi: 40.9, last_updated: '2026-11-07T19:42:00Z',
} as unknown as VehicleStatus;
const lastTrip = {
  id: 'trip-1', started_at: '2026-11-07T16:39:00Z', ended_at: '2026-11-07T16:56:00Z', distance_mi: 1.2, duration_min: 17,
  efficiency_wh_mi: 410, start_place: 'Near Kirkland', end_place: 'Home', end_odometer_mi: 12486, soc_end: 80,
};
const lastCharge = {
  id: 'charge-1', started_at: '2026-11-06T04:00:00Z', ended_at: '2026-11-06T09:00:00Z', location_name: 'Home', charger_type: 'ac',
  energy_added_kwh: 45.8, soc_start: 20, soc_end: 70, cost_usd: 8.66, peak_power_kw: 11,
};

class NoopObserver { observe() {} disconnect() {} unobserve() {} }

function renderNow(subject: Vehicle = vehicle) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><NowView vehicle={subject} /></QueryClientProvider>);
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date('2026-11-07T20:00:00Z'));
  setAppTimezone('UTC');
  vi.stubGlobal('IntersectionObserver', NoopObserver);
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('reduce'), media: query, addEventListener: () => {}, removeEventListener: () => {} }));
  Element.prototype.setPointerCapture = vi.fn();
  Element.prototype.hasPointerCapture = vi.fn(() => false);
  Element.prototype.releasePointerCapture = vi.fn();
  localStorage.clear();
  Object.assign(mocks, {
    status: parked, latestTrip: lastTrip, latestCharge: lastCharge, latestLoading: false, live: null, curve: [], images: null, activeTrip: null, backfilled: false,
    trips: [
      { ...lastTrip },
      { id: 'trip-2', started_at: '2026-11-06T16:00:00Z', distance_mi: 120, duration_min: 110, efficiency_wh_mi: 350, start_place: 'Home', end_place: 'Ridgefield' },
      { id: 'trip-3', started_at: '2026-10-20T16:00:00Z', distance_mi: 40, duration_min: 50, efficiency_wh_mi: 330, start_place: 'Home', end_place: 'Office' },
    ],
    charges: [
      { ...lastCharge },
      { ...lastCharge, id: 'charge-2', started_at: '2026-11-03T04:00:00Z', location_name: 'Ridgefield', charger_type: 'dc', energy_added_kwh: 50, cost_usd: null },
      { ...lastCharge, id: 'charge-3', started_at: '2026-10-21T04:00:00Z', location_name: 'Office', energy_added_kwh: 30, cost_usd: 0 },
    ],
    parks: [{ period_start: '2026-11-05T02:00:00Z', duration_hours: 12, soc_lost_pct: 0.4, validation_status: 'validated' }],
  });
  mocks.map.mockClear();
  mocks.statusRefetch.mockClear();
});
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('Now page, parked', () => {
  it('shows the header, car, battery, tiles, Latest, This week and Last 30 days from real data', () => {
    renderNow();
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Bumble Beee');
    expect(screen.getByText('Parked · Home · Locked')).toBeInTheDocument();
    expect(screen.getByText(/^Updated 7:42\sPM$/)).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Battery 62 percent, charge limit 70 percent' })).toBeInTheDocument();
    expect(screen.getByText('Limit 70%')).toBeInTheDocument();
    expect(screen.getByText('Estimated range')).toBeInTheDocument();
    const doors = screen.getByRole('link', { name: /Doors/ });
    expect(doors).toHaveTextContent('Locked');
    expect(doors).toHaveAttribute('href', '/vehicle-health');
    expect(screen.getByRole('link', { name: /Tires/ })).toHaveTextContent('41 psi');
    expect(screen.getByRole('link', { name: /Cabin/ })).toHaveTextContent('17°C');
    expect(screen.getAllByRole('heading', { level: 2 }).map(heading => heading.textContent)).toEqual(['Latest', 'This week', 'Last 30 days']);

    const trip = screen.getByRole('link', { name: /Kirkland → Home/ });
    expect(trip).toHaveAttribute('href', '/trips/trip-1');
    expect(trip).toHaveTextContent('1.2 mi');
    expect(trip).toHaveTextContent('Short trip');
    const charge = screen.getByRole('link', { name: /\+45\.8 kWh/ });
    expect(charge).toHaveAttribute('href', '/charging/charge-1');
    expect(charge).toHaveTextContent('+45.8 kWh');
    expect(charge).toHaveTextContent('$8.66');
    expect(charge).toHaveTextContent('20% → 70%');

    const week = screen.getByRole('region', { name: 'This week' });
    expect(within(week).getByText('2 trips', { exact: false })).toBeInTheDocument();
    expect(within(week).getByText(/Nov 2 – today/)).toBeInTheDocument();
    expect(within(week).getByRole('img', { name: 'Trip miles per day this week' })).toBeInTheDocument();
    expect(within(week).getByText('350')).toBeInTheDocument();
    expect(within(week).getByText('96')).toBeInTheDocument();
    expect(week).toHaveTextContent('$8.66 +1');
    expect(within(week).getByText('1 charge this week has no cost yet.')).toBeInTheDocument();

    expect(screen.getByRole('link', { name: /Charging cost/ })).toHaveAttribute('href', '/insights?tab=cost');
    expect(screen.getByRole('link', { name: /Charging cost/ })).toHaveTextContent('Other 1 not set');
    expect(screen.getByRole('link', { name: /Charging cost/ })).toHaveTextContent('Office Free');
    expect(screen.getByRole('link', { name: /^Efficiency/ })).toHaveAttribute('href', '/insights?tab=efficiency');
    expect(screen.getByRole('link', { name: /Loss while parked/ })).toHaveAttribute('href', '/insights?tab=battery');
    expect(screen.getByRole('link', { name: /Loss while parked/ })).toHaveTextContent('0.80 % / day');
    expect(document.body.textContent).not.toMatch(/->|SoC|Drag to turn/);
  });

  it('shows efficiency in the owner unit', () => {
    localStorage.setItem(EFFICIENCY_UNIT_STORAGE_KEY, 'mi_kwh');
    renderNow();
    const week = screen.getByRole('region', { name: 'This week' });
    expect(week).toHaveTextContent('2.86 mi/kWh');
    expect(screen.getByRole('link', { name: /^Efficiency/ })).toHaveTextContent('higher is better');
  });

  it('marks a stale reading as not updating', () => {
    mocks.status = { ...parked, telemetry_stale: true };
    renderNow();
    expect(screen.getByText('Not updating · Home · Locked')).toBeInTheDocument();
  });

  it('refreshes live data when tapping the refresh button or returning to the foreground', async () => {
    renderNow();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh vehicle data' }));
    await waitFor(() => expect(mocks.statusRefetch).toHaveBeenCalledTimes(1));

    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    vi.setSystemTime(new Date('2026-11-07T20:01:00Z'));
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await waitFor(() => expect(mocks.statusRefetch).toHaveBeenCalledTimes(2));
  });
});

describe('Now car turn', () => {
  it('never shows the bundled R1S turn for an R2 without owner images: a neutral placeholder instead', async () => {
    const { container } = renderNow();
    expect(await screen.findByRole('img', { name: 'Your car image is not available yet' })).toBeInTheDocument();
    expect(screen.queryByRole('slider')).toBeNull();
    expect(container.querySelector('.r-orbit-poster, canvas')).toBeNull();
    expect(screen.queryByRole('button', { name: /Front|Side|Replay|Reset/ })).toBeNull();
  });

  it('turns through the owner angle images when the server has them', async () => {
    mocks.images = ownerImages;
    const { container } = renderNow();
    await waitFor(() => expect(container.querySelectorAll('img.r-orbit-angle').length).toBe(4));
    const slider = screen.getByRole('slider');
    expect(slider).toHaveAttribute('aria-valuetext', 'Side');
    fireEvent.keyDown(slider, { key: 'ArrowRight' });
    expect(slider).not.toHaveAttribute('aria-valuetext', 'Side');
  });
  it('shows the loading placeholder, not the bundled turn, while the server image mirror is still filling', async () => {
    mocks.images = { ...ownerImages, cache: { status: 'repairing' } };
    const { container } = renderNow();
    await act(async () => {});
    expect(screen.getByRole('img', { name: 'Loading your car' })).toBeInTheDocument();
    expect(screen.queryByRole('slider', { name: ORBIT_LABEL })).toBeNull();
    expect(container.querySelectorAll('img.r-orbit-angle, .r-orbit-poster')).toHaveLength(0);
  });

  it('shows the owner images kept from an earlier visit at once, before the server answers', async () => {
    localStorage.setItem('r-owner-images:v1:owner-vehicle', JSON.stringify(ownerImages));
    mocks.images = new Promise(() => {});
    const { container } = renderNow();
    await waitFor(() => expect(container.querySelectorAll('img.r-orbit-angle, .r-orbit-stage').length).toBeGreaterThan(0));
    expect(screen.queryByRole('img', { name: 'Loading your car' })).toBeNull();
    expect(container.querySelector('.r-orbit-poster, canvas')).toBeNull();
  });
});

describe('Now states', () => {
  it('shows the skeleton while the first reads load', () => {
    mocks.latestLoading = true;
    renderNow();
    expect(screen.getByLabelText('Loading')).toHaveAttribute('aria-busy', 'true');
  });

  it('shows Day one with empty cards and the setup checklist for a car with no trips or charges', async () => {
    mocks.latestTrip = undefined;
    mocks.latestCharge = undefined;
    renderNow();
    expect(screen.getByText('Your first drive shows up here')).toBeInTheDocument();
    const checklist = screen.getByRole('region', { name: 'Getting set up' });
    expect(checklist).toHaveTextContent('Rivian account connected');
    expect(checklist).toHaveTextContent('Car found: Bumble Beee');
    expect(checklist).toHaveTextContent('Loading Rivian history · about a minute');
    expect(within(checklist).getByRole('link', { name: 'Check your Home and Office rates' })).toHaveAttribute('href', '/settings');
    expect(screen.queryByText('Last 30 days')).toBeNull();
  });

  it('shows live charging from the live session: power, battery, added energy, time left, range added and the curve', () => {
    mocks.status = { ...parked, power_state: 'charging', charger_state: 'charging', battery_limit: 80 };
    mocks.latestCharge = { ...lastCharge, id: 'live', ended_at: null, location_name: 'Ridgefield', charger_type: 'dc', started_at: '2026-11-07T19:47:00Z' };
    mocks.live = { soc_pct: 36, power_kw: 187.4, energy_kwh: 7.3, range_added_km: 33.8, time_remaining_min: 13, charger_type: 'DCFC', ts: '2026-11-07T19:50:00Z' };
    mocks.curve = [{ soc_pct: 30, power_kw: 150 }, { soc_pct: 34, power_kw: 190, power_method: 'recorded' }];
    renderNow();
    expect(screen.getByText('Charging · DC fast')).toBeInTheDocument();
    expect(screen.getByText(/Ridgefield · started 7:47\sPM/)).toBeInTheDocument();
    const card = screen.getByRole('region', { name: 'Charging now' });
    expect(card).toHaveTextContent('36%');
    expect(card).toHaveTextContent('187 kW');
    expect(card).toHaveTextContent('+7.3 kWh');
    expect(card).toHaveTextContent('13 min');
    expect(card).toHaveTextContent('+21 mi');
    expect(card.querySelector('.now-bar[data-charging]')).not.toBeNull();
    expect(screen.getByRole('img', { name: 'Power by battery level so far' })).toBeInTheDocument();
    expect(screen.queryByText(/Trial|Preview/)).toBeNull();
  });

  it('shows live driving with the map, trip so far from the odometer, battery and range', async () => {
    mocks.status = { ...parked, power_state: 'drive', gear_status: 'drive', speed_mph: 52, odometer_miles: 12507.5, battery_level: 74, range_miles: 245 };
    renderNow();
    expect(document.querySelector('.now-chip')).toHaveTextContent(/^Driving · 52 mph$/);
    expect(screen.getByText('From Home')).toBeInTheDocument();
    const card = screen.getByRole('region', { name: 'This trip' });
    expect(card).toHaveTextContent('21.5mi');
    expect(card).toHaveTextContent('74%');
    expect(card).toHaveTextContent('245 mi');
    expect(card).toHaveTextContent('6%');
    expect(screen.getByTestId('live-map')).toBeInTheDocument();
    expect(mocks.map.mock.lastCall?.[0]).toMatchObject({ activePoint: { lat: 47.6, lng: -122.3 } });
  });

  // The API has no trip-in-progress endpoint yet (ACTIVE_TRIP_API_AVAILABLE is false), so a server
  // trip is not read. When the endpoint ships, restore the server-trip case: 7.5 mi and 27 min here.
  it('works from the last trip while the API has no trip in progress', async () => {
    mocks.status = { ...parked, power_state: 'drive', speed_mph: 30, odometer_miles: 12507.5 };
    mocks.activeTrip = { started_at: '2026-11-07T19:33:00Z', start_odometer_mi: 12500, soc_start: 70, range_start_mi: 230,
      points: [{ ts: '2026-11-07T19:33:00Z', lat: 47.5, lng: -122.4 }, { ts: '2026-11-07T19:40:00Z', lat: 47.55, lng: -122.35 }] };
    renderNow();
    const card = screen.getByRole('region', { name: 'This trip' });
    await waitFor(() => expect(card).toHaveTextContent('21.5mi'));
    expect(card).not.toHaveTextContent('27 min');
  });
});

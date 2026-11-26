import React from 'react';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { places, sessions, trips } from './__tests__/insightsFixture';

const mocks = vi.hoisted(() => ({
  pathname: '/insights/cost',
  status: null as null | { last_updated: string | null; telemetry_stale?: boolean },
  trips: [] as unknown[],
  charges: [] as unknown[],
  parks: [] as unknown[],
  parkedEnergy: [] as unknown[],
  statePeriods: [] as unknown[],
  applyChargePlaceTag: vi.fn(),
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, params, search, children, ...props }: { to: string; params?: Record<string, string>; search?: Record<string, string>; children: React.ReactNode }) => {
    const path = Object.entries(params ?? {}).reduce((p, [key, value]) => p.replace(`$${key}`, value), to);
    return <a href={search ? `${path}?${new URLSearchParams(search)}` : path} {...props}>{children}</a>;
  },
  useLocation: () => ({ pathname: mocks.pathname }),
}));
vi.mock('../r-experience/useRConnection', () => ({
  useRConnection: () => ({ status: mocks.status, feed: null, renewal: null, onlineState: 'online', banner: null, vehicleId: 'v1' }),
}));
vi.mock('./useInsightsData', () => ({
  useAllTrips: () => ({ data: mocks.trips, isError: false }),
  useAllCharges: () => ({ data: mocks.charges, isError: false }),
  useChargeCurves: () => ({ curves: new Map(), loading: false }),
}));
vi.mock('../../hooks/useThemePreferenceController', () => ({
  useThemePreferenceController: () => ({ mode: 'dark', onModeChange: vi.fn(), isPending: false }),
}));
vi.mock('@panda/hooks', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@panda/hooks')>()),
  api: { applyChargePlaceTag: mocks.applyChargePlaceTag },
  invalidateChargingData: vi.fn(),
  useResolvedVehicleSelection: () => ({ authReady: true, vehicleSelectionReady: true, effectiveVehicleId: 'v1', vehicles: [] }),
  useCurrentVehicleStatus: () => ({ data: { vehicle_id: 'v1', battery_level: 62, range_miles: 206, battery_limit: 70, odometer_miles: 1163 } }),
  useDegradation: () => ({ data: [{ ts: '2026-10-07T18:00:00Z', usable_kwh: 91.4 }, { ts: '2026-10-15T18:00:00Z', usable_kwh: 91.2 }] }),
  useMe: () => ({ data: { user_id: 'u1' } }),
  useSavedPlaces: () => ({ data: places }),
  usePhantomDrainPeriods: () => ({ data: { vehicle_id: 'v1', periods: mocks.parks } }),
  useParkedEnergy: () => ({ data: { vehicle_id: 'v1', generated_at: '2026-10-15T18:00:00Z', source: 'rivian_reported', samples: mocks.parkedEnergy } }),
  useStateTimeline: () => ({ data: { vehicle_id: 'v1', periods: mocks.statePeriods } }),
  useVehicleHealth: () => ({ data: { software_history: [{ version: '2026.30' }, { version: '2026.35' }] } }),
}));

import { setAppTimezone } from '@panda/ui/lib/dateTime';
import { InsightsHub, type InsightTab } from './InsightsHub';
import { RAppLayout, rFreshness } from '../r-experience/RAppLayout';

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

function renderTab(tab: InsightTab) {
  mocks.pathname = `/insights/${tab}`;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><InsightsHub tab={tab} /></QueryClientProvider>);
}

describe('Insights', () => {
  beforeEach(() => {
    mocks.status = null;
    mocks.trips = trips;
    mocks.charges = sessions;
    mocks.parks = [];
    mocks.parkedEnergy = [];
    mocks.statePeriods = [];
    window.localStorage.clear();
    setAppTimezone('America/Los_Angeles');
    vi.spyOn(window, 'matchMedia').mockImplementation(query => ({
      matches: query.includes('reduce'), media: query, onchange: null, addListener: vi.fn(), removeListener: vi.fn(),
      addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
    }) as MediaQueryList);
    mocks.applyChargePlaceTag.mockReset().mockResolvedValue({ tag: 'home', place_id: 'place-home', updated: 1, session_ids: ['c-mill-1'] });
  });

  it('shows five tabs on one line, the current one marked, and no custom dashboards', () => {
    renderTab('charging');
    const tabs = within(screen.getByRole('navigation', { name: 'Insights sections' })).getAllByRole('link');
    expect(tabs.map(tab => [tab.textContent, tab.getAttribute('href')])).toEqual([
      ['Cost', '/insights/cost'], ['Charging', '/insights/charging'], ['Battery', '/insights/battery'],
      ['Efficiency', '/insights/efficiency'], ['Lifetime', '/insights/lifetime'],
    ]);
    expect(tabs.filter(tab => tab.getAttribute('aria-current') === 'page').map(tab => tab.textContent)).toEqual(['Charging']);
    expect(screen.queryByText(/dashboard/i)).toBeNull();
    expect(screen.getByText('Since Oct 6, the first day of data')).toBeInTheDocument();
  });

  it('renders inside the shell with four tabs and Insights active', () => {
    renderTab('cost');
    const nav = screen.getByRole('navigation', { name: 'Primary navigation' });
    expect(within(nav).getAllByRole('link').map(tab => tab.textContent)).toEqual(['Now', 'Activity', 'Insights', 'Car']);
    expect(within(nav).getByRole('link', { name: 'Insights' })).toHaveAttribute('aria-current', 'page');
  });

  it('Cost: total, cents per mile, the unpriced callout, places and rates', () => {
    renderTab('cost');
    expect(screen.getByText('$25.03')).toBeInTheDocument();
    expect(screen.getByText('15.5¢')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /1 charge has no cost yet/ })).toHaveAttribute('href', '/charging/c-mill-1');
    const where = screen.getByRole('region', { name: 'Where you charge' });
    expect(within(where).getByText('1 not set')).toBeInTheDocument();
    expect(within(where).getByText('Free')).toBeInTheDocument();
    const placesCard = screen.getByRole('region', { name: 'Places' });
    expect(within(placesCard).getByRole('group', { name: 'Tag Home' }).querySelector('[aria-pressed="true"]')).toHaveTextContent('Home');
    expect(within(placesCard).getByRole('link', { name: /Mill Creek/ })).toHaveAttribute('href', '/trips?filter=charges&place=Mill+Creek');
    const rates = screen.getByRole('region', { name: 'Your rates' });
    expect(within(rates).getByText('$0.1891')).toBeInTheDocument();
    expect(within(rates).getByText('Free')).toBeInTheDocument();
    expect(within(rates).getByRole('link', { name: /Change rates in Settings/ })).toHaveAttribute('href', '/settings');
  });

  it('Cost: tagging a place applies to every charge there, after the owner confirms the count and the price', async () => {
    renderTab('cost');
    const places = screen.getByRole('region', { name: 'Places' });
    const group = within(places).getByRole('group', { name: 'Tag Mill Creek' });
    fireEvent.click(within(group).getByRole('button', { name: 'Home' }));
    const confirm = within(places).getByRole('group', { name: 'Tag Mill Creek as Home?' });
    expect(confirm).toHaveTextContent('1 charge at Mill Creek moves to Home and is priced at $0.1891 per kWh. A cost you entered on a charge stays.');
    expect(mocks.applyChargePlaceTag).not.toHaveBeenCalled();
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    expect(within(places).queryByRole('group', { name: 'Tag Mill Creek as Home?' })).toBeNull();
    expect(mocks.applyChargePlaceTag).not.toHaveBeenCalled();
    fireEvent.click(within(group).getByRole('button', { name: 'Home' }));
    fireEvent.click(within(places).getByRole('button', { name: 'Tag as Home' }));
    await waitFor(() => expect(mocks.applyChargePlaceTag).toHaveBeenCalledWith('v1', 'c-mill-1', 'home', 'session'));
    expect(await screen.findByText('Mill Creek is now tagged Home. 1 charge was updated.')).toBeInTheDocument();
    fireEvent.click(within(group).getByRole('button', { name: 'Other' }));
    expect(mocks.applyChargePlaceTag).toHaveBeenCalledTimes(1);
  });

  it('Cost: prevents bulk re-tagging when a place has multiple charges', () => {
    mocks.charges = [
      ...sessions,
      { ...sessions[0]!, id: 'c-home-2', started_at: '2026-10-08T04:00:00Z', ended_at: '2026-10-08T06:00:00Z' },
    ];
    renderTab('cost');
    const placesCard = screen.getByRole('region', { name: 'Places' });
    expect(within(placesCard).getByText(/2 charges ·/)).toBeInTheDocument();
    expect(within(placesCard).queryByRole('group', { name: 'Tag Home' })).toBeNull();
    expect(within(placesCard).getByText('Home · per charge')).toBeInTheDocument();
    expect(within(placesCard).getByRole('group', { name: 'Tag Mill Creek' })).toBeInTheDocument();
  });

  it('Cost: the gas comparison is stored per user', () => {
    renderTab('cost');
    fireEvent.change(screen.getByLabelText('Miles per gallon'), { target: { value: '20' } });
    fireEvent.click(screen.getByRole('button', { name: 'Update' }));
    expect(JSON.parse(window.localStorage.getItem('r-insights-gas:u1')!)).toEqual({ mpg: 20, price: 4.5 });
  });

  it('Charging: energy, AC and DC, cycles from usable capacity, and the calendar', () => {
    renderTab('charging');
    expect(screen.getByText('94 kWh')).toBeInTheDocument();
    expect(screen.getByText('3 AC · 1 DC')).toBeInTheDocument();
    expect(screen.getByText('energy added ÷ 91.2 kWh')).toBeInTheDocument();
    expect(screen.getByRole('table', { name: 'Charging calendar' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Start and end of each charge' })).toBeInTheDocument();
  });

  it('Battery: health from usable capacity and the parked-loss card', () => {
    const dayAt = (daysAgo: number, utcHour: number) => {
      const base = new Date();
      base.setUTCDate(base.getUTCDate() - daysAgo);
      base.setUTCHours(utcHour, 0, 0, 0);
      return base.toISOString();
    };
    const normalStart = dayAt(4, 6);
    const normalEnd = dayAt(4, 18);
    const spikeStart = dayAt(2, 8);
    const spikeAwake = dayAt(2, 16);
    const spikeEnd = dayAt(2, 20);

    mocks.parks = [
      {
        period_start: normalStart,
        period_end: normalEnd,
        duration_hours: 12,
        sleep_share_pct: 0.94,
        state_coverage_pct: 1,
        soc_start: 74,
        soc_end: 73.5,
        soc_lost_pct: 0.5,
        drain_pct_per_hour: 0.04,
        range_start_mi: 240,
        range_end_mi: 238,
        range_lost_mi: 2,
        range_lost_per_hour_mi: 0.16,
        energy_drained_kwh: 0.45,
        avg_power_w: 38,
        has_reduced_range: false,
        validation_status: 'validated',
        validation_reason: null,
        sample_count: 10,
        start_sample_at: normalStart,
        end_sample_at: normalEnd,
        movement_detected: false,
        overlaps_trip: false,
        overlaps_charge: false,
      },
      {
        period_start: spikeStart,
        period_end: spikeEnd,
        duration_hours: 12,
        sleep_share_pct: 0.65,
        state_coverage_pct: 1,
        soc_start: 70,
        soc_end: 66.8,
        soc_lost_pct: 3.2,
        drain_pct_per_hour: 0.27,
        range_start_mi: 230,
        range_end_mi: 219,
        range_lost_mi: 11,
        range_lost_per_hour_mi: 0.92,
        energy_drained_kwh: 2.9,
        avg_power_w: 242,
        has_reduced_range: true,
        avg_outside_temp_c: -2,
        avg_cabin_temp_c: 15,
        wake_count: 4,
        max_sleep_streak_minutes: 180,
        gear_guard_active: true,
        climate_active: false,
        ota_active: true,
        open_closure_detected: false,
        doors_unlocked: false,
        twelve_volt_attention: false,
        plugged_in: false,
        validation_status: 'validated',
        validation_reason: null,
        sample_count: 18,
        start_sample_at: spikeStart,
        end_sample_at: spikeEnd,
        movement_detected: false,
        overlaps_trip: false,
        overlaps_charge: false,
      },
    ];
    mocks.statePeriods = [
      { id: 1, state: 'sleep', started_at: spikeStart, ended_at: spikeAwake, duration_seconds: 28800 },
      { id: 2, state: 'idle', started_at: spikeAwake, ended_at: spikeEnd, duration_seconds: 14400 },
    ];
    mocks.parkedEnergy = [
      {
        window: '24h',
        source_at: spikeEnd,
        received_at: spikeEnd,
        parked_started_at: spikeStart,
        duration_minutes: 720,
        total_kwh: 3.0,
        vehicle_systems_kwh: 1.5,
        climate_kwh: 0.6,
        gear_guard_kwh: 0.9,
        outlets_kwh: 0,
        total_range_impact_km: 10,
        vehicle_systems_range_impact_km: 5,
        outlets_range_impact_km: null,
        climate_range_impact_km: 2,
        gear_guard_range_impact_km: 3,
      },
    ];

    renderTab('battery');
    expect(screen.getByText('0.2 kWh below the first day', { exact: false })).toBeInTheDocument();
    expect(screen.getByText('332 mi')).toBeInTheDocument();
    const parkedCard = screen.getByRole('region', { name: 'Parked loss, last 30 days' });
    expect(parkedCard).toBeInTheDocument();
    expect(within(parkedCard).getByText('High drain day')).toBeInTheDocument();
    expect(within(parkedCard).getByText(/Deep sleep · 8h/)).toBeInTheDocument();
    expect(within(parkedCard).getByText(/Kept awake · 4h \(Gear Guard, OTA update\)/)).toBeInTheDocument();
    expect(within(parkedCard).getByText('Below freezing · extra thermal draw')).toBeInTheDocument();

    // Selecting the healthy night switches the card cleanly to the calm 1-line summary.
    const normalBar = screen.getByRole('button', { name: /0\.5 percent lost/ });
    fireEvent.click(normalBar);
    expect(within(parkedCard).getByText('Healthy sleep')).toBeInTheDocument();
    expect(within(parkedCard).getByText(/Normal overnight sleep/)).toBeInTheDocument();

    const breakdownCard = screen.getByRole('region', { name: 'Where parked energy went' });
    expect(within(breakdownCard).getByText(/Latest 24h report · 3\.0 kWh total/)).toBeInTheDocument();
    expect(within(breakdownCard).getByText(/~6\.2 mi range impact/)).toBeInTheDocument();
    expect(within(breakdownCard).getByText('Vehicle systems')).toBeInTheDocument();
    expect(within(breakdownCard).getByText('Gear Guard')).toBeInTheDocument();
    expect(screen.queryByText(/phantom/i)).toBeNull();
  });

  it('Efficiency: the unit switch changes every value and persists', () => {
    renderTab('efficiency');
    const unit = screen.getByRole('group', { name: 'Efficiency unit' });
    expect(within(unit).getByRole('button', { name: 'Wh/mi' })).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(within(unit).getByRole('button', { name: 'mi/kWh' }));
    expect(within(unit).getByRole('button', { name: 'mi/kWh' })).toHaveAttribute('aria-pressed', 'true');
    expect(window.localStorage.getItem('r-efficiency-unit')).toBe('mi_kwh');
    expect(screen.getByText('Trips under 3 miles are left out. Their battery readings are too coarse to measure.')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Average efficiency' })).toHaveTextContent('3 tripsHigher is better.');
  });

  it('Lifetime: miles, trip coverage and the month table', () => {
    renderTab('lifetime');
    expect(screen.getByRole('region', { name: 'Trip coverage' })).toHaveTextContent('The odometer moved 163.0 mi. Recorded trips add up to 161.2 mi.');
    expect(screen.getByText('October 2026')).toBeInTheDocument();
    expect(screen.getByText('1')).toBeInTheDocument();
  });

  it('opens a chart full screen with its table and closes it with Escape', () => {
    renderTab('cost');
    fireEvent.click(screen.getByRole('button', { name: 'Expand Cost per week' }));
    const dialog = screen.getByRole('dialog', { name: 'Cost per week' });
    expect(within(dialog).getByRole('table')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows a first-run state without data', () => {
    mocks.trips = []; mocks.charges = [];
    renderTab('cost');
    expect(screen.getByText('No trips or charges yet')).toBeInTheDocument();
  });
});

describe('rFreshness', () => {
  const now = Date.parse('2026-11-07T12:00:00Z');
  const at = (minutes: number) => new Date(now - minutes * 60_000).toISOString();

  it.each([
    [0, false, 'Updated just now'],
    [2, false, 'Updated 2 min ago'],
    [59, false, 'Updated 59 min ago'],
    [125, false, 'Updated 2 h ago'],
    [72 * 60, false, 'Updated 3 days ago'],
    [0, true, 'No data for 1 min'],
    [6 * 60, true, 'No data for 6 h'],
  ])('reads %i minutes (stale %s) as "%s"', (minutes, stale, label) => {
    expect(rFreshness({ last_updated: at(minutes), telemetry_stale: stale }, now)?.label).toBe(label);
  });

  it('shows nothing without a valid status timestamp', () => {
    expect(rFreshness(null, now)).toBeNull();
    expect(rFreshness({ last_updated: null }, now)).toBeNull();
    expect(rFreshness({ last_updated: 'not a date' }, now)).toBeNull();
  });
});

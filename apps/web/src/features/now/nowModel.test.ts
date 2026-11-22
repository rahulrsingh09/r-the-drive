import { beforeEach, describe, expect, it } from 'vitest';
import type { ChargeSession, PhantomDrainPeriod, Place, Trip, VehicleStatus } from '@panda/types';
import { setAppTimezone } from '@panda/ui/lib/dateTime';
import {
  chargeCategory, chargeTitle, costText, dayKey, durationText, mondayOf, monthSummary, nowActivity, nowPlace, parkedChip,
  placeAt, routePath, spinLabel, tireRange, tireText, tripTitle, updatedLabel, vehicleConfiguration, weekSummary,
} from './nowModel';

const status = (overrides: Partial<VehicleStatus> = {}) => ({
  vehicle_id: 'v', battery_level: 68, range_miles: 218, power_state: 'ready', charger_state: 'disconnected',
  charger_status: 'chrgr_sts_not_connected', speed_mph: 0, latitude: 47.6, longitude: -122.3, last_updated: null, doors_locked: true,
  ...overrides,
}) as VehicleStatus;

const place = (overrides: Partial<Place>): Place => ({
  id: 'p1', name: 'Home', latitude: 47.6, longitude: -122.3, radius_m: 100, is_home: true, is_work: false, address: null, charging: null,
  ...overrides,
});
const home = place({});
const office = place({ id: 'p2', name: 'Campus', latitude: 47.62, is_home: false, is_work: true });
const trip = (overrides: Partial<Trip> = {}) => ({
  id: 't1', vehicle_id: 'v', started_at: '2026-11-07T10:00:00Z', ended_at: '2026-11-07T10:30:00Z', distance_mi: 20,
  duration_min: 30, energy_used_kwh: 6.4, efficiency_wh_mi: 320, max_speed_mph: 50, drive_mode: null, soc_start: 70, soc_end: 64,
  ...overrides,
}) as Trip;
const charge = (overrides: Partial<ChargeSession>) => ({
  id: 'c1', vehicle_id: 'v', started_at: '2026-11-06T04:00:00Z', ended_at: '2026-11-06T06:00:00Z', location_name: 'Home',
  charger_type: 'ac', energy_added_kwh: 20, duration_min: 120, soc_start: 40, soc_end: 65, peak_power_kw: 11, cost_usd: null,
  ...overrides,
}) as ChargeSession;
const park = (overrides: Partial<PhantomDrainPeriod>) => ({
  period_start: '2026-11-05T02:00:00Z', period_end: '2026-11-05T14:00:00Z', duration_hours: 12, soc_lost_pct: 1, validation_status: 'validated',
  ...overrides,
}) as PhantomDrainPeriod;

/** Saturday, 2026-11-07 15:00 UTC. */
const NOW = Date.parse('2026-11-07T15:00:00Z');

beforeEach(() => setAppTimezone('UTC'));

describe('status', () => {
  it('reads the vehicle state, keeping the last known state when data is stale', () => {
    expect(nowActivity(null)).toBe('waiting');
    expect(nowActivity(status({ power_state: 'charging' }))).toBe('charging');
    expect(nowActivity(status({ charger_status: 'chrgr_sts_connected_charging' }))).toBe('charging');
    expect(nowActivity(status({ power_state: 'drive', gear_status: 'drive' }))).toBe('driving');
    expect(nowActivity(status({ power_state: 'ready', gear_status: 'drive', speed_mph: 30 }))).toBe('driving');
    expect(nowActivity(status({ power_state: 'go', gear_status: 'reverse' }))).toBe('driving');
    expect(nowActivity(status({ power_state: 'go', gear_status: 'neutral' }))).toBe('driving');
    expect(nowActivity(status({ power_state: 'go', speed_mph: 12 }))).toBe('driving');
    // The car is on in park: the power state says "go", but the car is parked.
    expect(nowActivity(status({ power_state: 'go', gear_status: 'park' }))).toBe('parked');
    expect(nowActivity(status({ power_state: 'drive', gear_status: 'park', speed_mph: 0 }))).toBe('parked');
    expect(nowActivity(status({ power_state: 'go' }))).toBe('parked');
    expect(nowActivity(status({ power_state: 'sleep', telemetry_stale: true }))).toBe('parked');
  });

  it('names the saved place at the car, or the end place of the latest trip', () => {
    expect(placeAt(47.6003, -122.3, [home])).toBe('Home');
    expect(placeAt(47.61, -122.3, [home])).toBeNull();
    expect(nowPlace(status(), [home], undefined)).toBe('Home');
    expect(nowPlace(status({ latitude: null, longitude: null }), [], trip({ end_place: 'Office' }))).toBe('Office');
    expect(nowPlace(status({ latitude: 47.7 }), [], trip({ end_place: 'Office', end_lat: 47.6, end_lng: -122.3 }))).toBeNull();
  });

  it('writes the parked chip from real readings and leaves out unknown parts', () => {
    expect(parkedChip(status(), 'Home')).toBe('Parked · Home · Locked');
    expect(parkedChip(status({ doors_locked: false }), 'Near Kirkland')).toBe('Parked · Kirkland · Unlocked');
    expect(parkedChip(status({ doors_locked: null }), null)).toBe('Parked');
    expect(parkedChip(null, null)).toBe('Waiting for the car');
  });

  it('states the update time, with the date when it is not today', () => {
    expect(updatedLabel(status({ last_updated: '2026-11-07T14:42:00Z' }), NOW)).toMatch(/^Updated 2:42\s?PM$/);
    expect(updatedLabel(status({ last_updated: '2026-11-06T14:42:00Z' }), NOW)).toMatch(/^Updated Nov 6, 2:42\s?PM$/);
    expect(updatedLabel(status(), NOW)).toBeNull();
  });

  it('shows the tire range in whole psi', () => {
    expect(tireText(tireRange(status({ tire_fl_psi: 41.2, tire_fr_psi: 40.8, tire_rl_psi: 41, tire_rr_psi: 41.4 })))).toBe('41');
    expect(tireText(tireRange(status({ tire_fl_psi: 39, tire_fr_psi: 42 })))).toBe('39–42');
    expect(tireText(tireRange(status()))).toBeNull();
  });
});

describe('vehicle identity', () => {
  it('uses the real model, trim and color, with no fixed paint name', () => {
    expect(vehicleConfiguration({ model: 'R2', trim: 'Performance', color: 'Launch Green' })).toBe('R2 Performance · Launch Green');
    expect(vehicleConfiguration({ model: 'R1S', trim: 'R1S Adventure', color: null })).toBe('R1S Adventure');
    expect(spinLabel({ model: 'R2', color: 'Catalina Cove' })).toBe('Your R2, Catalina Cove. Use arrow keys to turn.');
  });
});

describe('rows', () => {
  it('titles trips and charges with the glossary arrow and the DC fast label', () => {
    expect(tripTitle({ start_place: 'Near Kirkland', end_place: 'Home' })).toBe('Kirkland → Home');
    expect(tripTitle({ start_place: 'Home', end_place: 'Home' })).toBe('Round trip · Home');
    expect(chargeTitle(charge({ charger_type: 'dc', location_name: 'Ridgefield' }))).toBe('DC fast · Ridgefield');
    expect(chargeTitle(charge({}))).toBe('Home');
    expect(costText(charge({ cost_usd: null }))).toBe('Add cost');
    expect(costText(charge({ cost_usd: 0 }))).toBe('Free');
    expect(costText(charge({ cost_usd: 8.664 }))).toBe('$8.66');
    expect(durationText(17)).toBe('17 min');
    expect(durationText(65)).toBe('1 h 5 min');
  });

  it('puts each charge in Home, Office or Other from its saved place', () => {
    expect(chargeCategory({ location_name: 'Home' }, [home])).toBe('home');
    expect(chargeCategory({ location_name: 'Campus' }, [home, office])).toBe('office');
    expect(chargeCategory({ location_name: 'Office' }, [])).toBe('office');
    expect(chargeCategory({ location_name: 'Bellevue SC' }, [home, office])).toBe('other');
    expect(chargeCategory({ location_name: null }, [home])).toBe('other');
  });

  it('draws a route only with two points', () => {
    expect(routePath([{ lat: 47.6, lng: -122.3 }], 64, 52)).toBeNull();
    expect(routePath([{ lat: 47.6, lng: -122.3 }, { lat: 47.7, lng: -122.2 }], 64, 52)).toMatch(/^M[\d.]+,[\d.]+L[\d.]+,[\d.]+$/);
  });
});

describe('this week', () => {
  it('starts on Monday and sums trip miles, efficiency from trips of 3 mi or more, energy and known cost', () => {
    expect(dayKey(mondayOf(NOW))).toBe('2026-11-02');
    const week = weekSummary([
      trip({ started_at: '2026-11-07T10:00:00Z', distance_mi: 20, efficiency_wh_mi: 300 }),
      trip({ id: 't2', started_at: '2026-11-03T10:00:00Z', distance_mi: 10, efficiency_wh_mi: 400 }),
      trip({ id: 't3', started_at: '2026-11-03T12:00:00Z', distance_mi: 1.2, efficiency_wh_mi: 900 }),
      trip({ id: 'old', started_at: '2026-11-01T10:00:00Z', distance_mi: 99 }),
    ], [
      charge({ cost_usd: 3.78 }), charge({ id: 'c2', started_at: '2026-11-04T04:00:00Z', energy_added_kwh: 30 }),
      charge({ id: 'open', started_at: '2026-11-07T14:00:00Z', ended_at: null, energy_added_kwh: 5 }),
    ], NOW);
    expect(week.days.map(day => day.label)).toEqual(['M', 'T', 'W', 'T', 'F', 'S', 'S']);
    expect(week.days.map(day => day.miles)).toEqual([0, 11.2, 0, 0, 0, 20, 0]);
    expect(week.days.find(day => day.today)?.key).toBe('2026-11-07');
    expect(week.trips).toBe(3);
    expect(week.miles).toBeCloseTo(31.2);
    expect(week.efficiencyWhMi).toBeCloseTo((20 * 300 + 10 * 400) / 30);
    expect(week.chargedKwh).toBe(50);
    expect(week.cost).toBeCloseTo(3.78);
    expect(week.unpriced).toBe(1);
  });
});

describe('last 30 days', () => {
  it('splits charging cost by place, takes efficiency per week and loss while parked per day', () => {
    const month = monthSummary([
      trip({ started_at: '2026-11-03T10:00:00Z', efficiency_wh_mi: 300 }),
      trip({ id: 't2', started_at: '2026-10-27T10:00:00Z', efficiency_wh_mi: 360 }),
      trip({ id: 'old', started_at: '2026-10-01T10:00:00Z', efficiency_wh_mi: 999 }),
    ], [
      charge({ cost_usd: 3.78 }),
      charge({ id: 'c2', location_name: 'Campus', cost_usd: 0, energy_added_kwh: 10 }),
      charge({ id: 'c3', location_name: 'Ridgefield', charger_type: 'dc', cost_usd: null, energy_added_kwh: 40 }),
    ], [park({}), park({ period_start: '2026-11-06T02:00:00Z', duration_hours: 12, soc_lost_pct: 2 }), park({ validation_status: 'excluded', soc_lost_pct: 9 })],
    [home, office], NOW);
    expect(month.cost).toBeCloseTo(3.78);
    expect(month.byCategory.map(item => [item.label, item.kwh, item.cost, item.unpriced])).toEqual([
      ['Home', 20, 3.78, 0], ['Office', 10, 0, 0], ['Other', 40, 0, 1],
    ]);
    expect(month.weeks.map(week => [dayKey(week.from), week.whMi])).toEqual([['2026-10-26', 360], ['2026-11-02', 300]]);
    expect(month.drainDays).toHaveLength(30);
    expect(month.drainDays.find(day => day.key === '2026-11-05')?.lossPct).toBe(1);
    expect(month.drainPerDay).toBeCloseTo(3 / (24 / 24));
  });
});

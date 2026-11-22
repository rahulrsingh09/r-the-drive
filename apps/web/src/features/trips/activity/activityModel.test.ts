import { describe, expect, it } from 'vitest';
import type { ChargeSession, Place, Trip } from '@panda/types';
import {
  activityFeed, averageEfficiency, buildActivityData, chargeCost, detailOrder, groupByDay, odometerGaps, parkedPeriods,
  periodStartDay, shiftDay, summarize, toActivityCharge, toActivityTrip, tripPlace, tripTitle,
} from './activityModel';
import { nearestTown } from '../../../lib/placeNames';

const home: Place = {
  id: 'home', name: 'Home', latitude: 47.6769, longitude: -122.206, radius_m: 100, is_home: true, is_work: false, address: null, charging: null,
};
const trip = (id: string, start: string, end: string, extra: Partial<Trip> = {}): Trip => ({
  id, vehicle_id: 'v', started_at: start, ended_at: end, distance_mi: 10, duration_min: 20, energy_used_kwh: 3, efficiency_wh_mi: 300,
  max_speed_mph: null, drive_mode: null, soc_start: 80, soc_end: 76, ...extra,
});
const charge = (id: string, start: string, end: string, extra: Partial<ChargeSession> = {}): ChargeSession => ({
  id, vehicle_id: 'v', started_at: start, ended_at: end, location_name: null, charger_type: 'ac', energy_added_kwh: 12, duration_min: 60,
  soc_start: 50, soc_end: 70, peak_power_kw: 11, cost_usd: null, ...extra,
});

describe('activity places', () => {
  it('names a saved place, an address town, then the nearest town', () => {
    const base = trip('a', '2026-11-07T10:00:00Z', '2026-11-07T10:20:00Z');
    expect(tripPlace({ ...base, start_lat: 47.6769, start_lng: -122.206 }, 'start', [home])).toEqual({ name: 'Home', approximate: false });
    expect(tripPlace({ ...base, start_place: 'Main St, Redmond', start_address: '1 Main St, Redmond, WA' }, 'start', [])).toEqual({ name: 'Redmond', approximate: false });
    expect(tripPlace({ ...base, start_lat: 47.61, start_lng: -122.2 }, 'start', [])).toEqual({ name: 'Near Bellevue', approximate: true });
    expect(nearestTown(40.7, -74)).toBeNull();
  });

  it('titles round trips and one-way trips with the arrow', () => {
    expect(tripTitle({ from: { name: 'Home', approximate: false }, to: { name: 'Near Kirkland', approximate: true } })).toBe('Home → Kirkland');
    expect(tripTitle({ from: { name: 'Home', approximate: false }, to: { name: 'Home', approximate: false } })).toBe('Round trip · Home');
  });

  it('tags a charge Home, Office or Other from the saved place', () => {
    const office: Place = { ...home, id: 'office', name: 'Work', latitude: 47.61, longitude: -122.2015, is_home: false, is_work: true };
    expect(toActivityCharge(charge('c', '2026-11-07T01:00:00Z', '2026-11-07T02:00:00Z', { location_lat: 47.6769, location_lng: -122.206 }), [home, office])!.category).toBe('home');
    expect(toActivityCharge(charge('c', '2026-11-07T01:00:00Z', '2026-11-07T02:00:00Z', { location_lat: 47.61, location_lng: -122.2015 }), [home, office])!.category).toBe('office');
    const other = toActivityCharge(charge('c', '2026-11-07T01:00:00Z', '2026-11-07T02:00:00Z', { location_lat: 45.8151, location_lng: -122.7426, charger_type: 'dc' }), [home]);
    expect(other).toMatchObject({ category: 'public', place: 'Near Ridgefield', placeApproximate: true, dc: true });
  });
});

describe('activity cost', () => {
  it('reads free methods as 0 and a missing cost as not set', () => {
    expect(chargeCost(charge('a', 'x', 'y', { cost_method: 'network_free', cost_usd: null }))).toBe(0);
    expect(chargeCost(charge('a', 'x', 'y', { cost_usd: 4.2 }))).toBe(4.2);
    expect(chargeCost(charge('a', 'x', 'y', { cost_method: 'unknown' }))).toBeNull();
    expect(chargeCost(charge('a', 'x', null as unknown as string, { cost_usd: 4 }))).toBeNull();
  });
});

describe('activity timeline', () => {
  const trips = [
    trip('t1', '2026-11-07T08:00:00Z', '2026-11-07T08:20:00Z', { start_odometer_mi: 100, end_odometer_mi: 110, soc_start: 90, soc_end: 86 }),
    trip('t2', '2026-11-07T22:00:00Z', '2026-11-07T22:20:00Z', { start_odometer_mi: 112.5, end_odometer_mi: 113.2, distance_mi: 0.7, efficiency_wh_mi: 900, soc_start: 80 }),
    trip('t3', '2026-11-08T01:00:00Z', '2026-11-08T01:20:00Z', { start_odometer_mi: 113.4, end_odometer_mi: 120 }),
  ];
  const data = buildActivityData(trips, [], []);

  it('finds parked periods with the battery loss', () => {
    const parked = parkedPeriods(data.trips);
    expect(parked).toHaveLength(1);
    expect(parked[0]).toMatchObject({ loss: 6 });
    expect(parked[0]!.hours).toBeCloseTo(13.67, 1);
  });

  it('finds odometer moves over 0.3 mi between trips', () => {
    const gaps = odometerGaps(data.trips);
    expect(gaps).toHaveLength(1);
    expect(gaps[0]!.miles).toBeCloseTo(2.5);
  });

  it('builds the feed for each filter, newest first, and groups it by day', () => {
    const options = { filter: 'all' as const, period: 'all' as const, place: null, today: '2026-11-08' };
    expect(activityFeed(data, options).map((item) => item.kind)).toEqual(['trip', 'trip', 'parked', 'gap', 'trip']);
    expect(activityFeed(data, { ...options, filter: 'trips' }).every((item) => item.kind === 'trip')).toBe(true);
    expect(detailOrder(activityFeed(data, options)).map((item) => item.id)).toEqual(['t3', 't2', 't1']);
    expect(groupByDay(activityFeed(data, options)).map((group) => group.day)).toEqual(['2026-11-08', '2026-11-07']);
  });

  it('leaves trips under 3 mi out of the efficiency average', () => {
    expect(averageEfficiency(data.trips)).toBe(300);
    const summary = summarize(data, { filter: 'all', period: '7', place: null, today: '2026-11-08' });
    expect(summary).toMatchObject({ kind: 'trips', count: 3, whPerMile: 300 });
    expect(summary.days).toHaveLength(7);
  });

  it('computes period days', () => {
    expect(periodStartDay('7', '2026-11-08')).toBe('2026-11-02');
    expect(periodStartDay('all', '2026-11-08')).toBeNull();
    expect(shiftDay('2026-03-01', -1)).toBe('2026-02-28');
  });

  it('keeps a short trip without efficiency', () => {
    expect(toActivityTrip(trip('s', '2026-11-07T08:00:00Z', '2026-11-07T08:05:00Z', { efficiency_wh_mi: null }), [])!.whPerMile).toBeNull();
  });
});

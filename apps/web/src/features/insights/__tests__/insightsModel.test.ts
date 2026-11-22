import { describe, expect, it } from 'vitest';
import {
  activeMinutes, addDays, averageWhMi, batteryBand, byCategory, byMonth, capacityByDay, chargeRows, dayKeyOf, dayRange, drainByDay,
  gasCost, measured, mondayOf, monthDay, niceTicks, odometerGaps, placeGroups, speedGroups, tripRows, weeksFrom,
} from '../insightsModel';
import { profileInput, profileSource, rateDetail, ratePlace } from '../placeTags';
import { homePlace, officePlace, places, sessions, trips } from './insightsFixture';
import { batteryHealthPct, usableCapacityNow } from '../../../lib/batteryHealth';
import { batteryHealthPct as carBatteryHealthPct } from '../../health/carModel';

const TZ = 'America/Los_Angeles';

describe('calendar days', () => {
  it('reads days in the app timezone and walks weeks from Monday', () => {
    expect(dayKeyOf('2026-10-07T04:00:00Z', TZ)).toBe('2026-10-06');
    expect(mondayOf('2026-10-08')).toBe('2026-10-05');
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01');
    expect(weeksFrom('2026-10-06', '2026-10-20')).toEqual(['2026-10-05', '2026-10-12', '2026-10-19']);
    expect(dayRange('2026-10-30', '2026-11-02')).toHaveLength(4);
    expect(monthDay('2026-10-06')).toBe('Oct 6');
  });

  it('makes round ticks', () => {
    expect(niceTicks(3, 19, 5)).toEqual([0, 5, 10, 15, 20]);
  });
});

describe('charges', () => {
  const rows = chargeRows(sessions, places, TZ);

  it('leaves open charges out and tags each charge by its saved place', () => {
    expect(rows.map(r => [r.id, r.category, r.placeName])).toEqual([
      ['c-home-1', 'home', 'Home'], ['c-office-1', 'office', 'Office'], ['c-dc-1', 'other', 'Ridgefield'], ['c-mill-1', 'other', 'Mill Creek'],
    ]);
  });

  it('sums energy and cost by place; a charge without a price is "not set"', () => {
    const by = byCategory(rows);
    expect(by.find(b => b.category === 'home')).toMatchObject({ kwh: 20, cost: 3.78, unpriced: 0 });
    expect(by.find(b => b.category === 'office')).toMatchObject({ kwh: 30, cost: 0, unpriced: 0 });
    expect(by.find(b => b.category === 'other')).toMatchObject({ kwh: 44, cost: 21.25, unpriced: 1 });
    expect(by.find(b => b.category === 'other')!.perKwh).toBeCloseTo(21.25 / 40);
  });

  it('groups charges by place, largest energy first', () => {
    expect(placeGroups(rows).map(g => [g.name, g.charges.length, g.unpriced])).toEqual([
      ['Ridgefield', 1, 0], ['Office', 1, 0], ['Home', 1, 0], ['Mill Creek', 1, 1],
    ]);
  });

  it('times a charge until the battery stopped rising', () => {
    expect(activeMinutes([
      { minutes_elapsed: 0, soc_pct: 40, power_kw: 11 }, { minutes_elapsed: 60, soc_pct: 52, power_kw: 11 },
      { minutes_elapsed: 120, soc_pct: 62, power_kw: 11 }, { minutes_elapsed: 180, soc_pct: 62, power_kw: 0 },
    ])).toBe(120);
    expect(activeMinutes([{ minutes_elapsed: 0, soc_pct: 40, power_kw: 0 }])).toBeNull();
    const curves = new Map([['c-home-1', [{ minutes_elapsed: 0, soc_pct: 40, power_kw: 11 }, { minutes_elapsed: 120, soc_pct: 62, power_kw: 11 }]]]);
    expect(speedGroups(rows, curves).find(g => g.label === 'Home')?.kw).toBeCloseTo(10);
  });

  it('compares with a gas car', () => {
    expect(gasCost(250, { mpg: 25, price: 4 })).toBe(40);
  });
});

describe('place tags', () => {
  it('reads the Home and Office rates from the flagged places', () => {
    expect(ratePlace(places, 'home')?.id).toBe('place-home');
    expect(ratePlace(places, 'office')?.id).toBe('place-office');
    expect(ratePlace(places, 'other')).toBeNull();
  });

  it('turns a free profile into $0 per kWh', () => {
    expect(profileInput(officePlace.charging)).toMatchObject({ billing_type: 'per_kwh', rate: 0 });
    expect(profileInput(homePlace.charging)).toMatchObject({ billing_type: 'per_kwh', rate: 0.189119 });
  });

  it('names a charge with only coordinates by the nearest town, as Activity does', () => {
    const coordinates = [
      { ...sessions[2]!, id: 'c-ridge', location_name: '45.8178, -122.6827', location_lat: 45.8178, location_lng: -122.6827 },
      { ...sessions[2]!, id: 'c-kelso', location_name: '46.1433, -122.8986', location_lat: 46.1433, location_lng: -122.8986 },
      { ...sessions[3]!, id: 'c-mill', location_name: '47.8284, -122.2115', location_lat: 47.8284, location_lng: -122.2115 },
    ];
    const named = chargeRows(coordinates, places, TZ);
    expect(named.map(r => r.placeName)).toEqual(['Ridgefield', 'Kelso', 'Mill Creek']);
    expect(new Set(named.map(r => r.placeKey)).size).toBe(3);
  });

  it('writes the rate sub-line from the place town and the rate source', () => {
    const rows = chargeRows(sessions, places, TZ);
    expect(profileSource('Home charging (PSE Schedule 7)')).toBe('PSE Schedule 7');
    expect(profileSource('Office charging (free)')).toBeNull();
    expect(profileSource('Home')).toBeNull();
    expect(rateDetail(homePlace, rows)).toBe('Kirkland · PSE Schedule 7');
    expect(rateDetail(officePlace, rows)).toBe('Bellevue');
    const atOffice = rows.map(r => (r.id === 'c-office-1' ? { ...r, session: { ...r.session, network_vendor: 'chargepoint' } } : r));
    expect(rateDetail(officePlace, atOffice)).toBe('Bellevue · ChargePoint');
    expect(rateDetail(null, rows)).toBe('No place tagged yet');
  });

  it('keeps a charge tagged Other out of the saved place that holds its location', () => {
    const tagged = sessions.map(s => (s.id === 'c-home-1' ? { ...s, location_override_mode: 'none' as const } : s));
    expect(chargeRows(tagged, places, TZ)[0]).toMatchObject({ id: 'c-home-1', category: 'other', place: null });
  });
});

describe('trips, battery and lifetime', () => {
  const rows = tripRows(trips, TZ);

  it('titles trips with the shared place names', () => {
    const toRidgefield = { ...trips[2]!, id: 't-ridge', start_lat: 47.7136, start_lng: -122.2037, end_place: null, end_lat: 45.8178, end_lng: -122.6827 };
    expect(tripRows([toRidgefield], TZ, places)[0]!.title).toBe('Home → Ridgefield');
    expect(rows.find(r => r.id === 't3')!.title).toBe('Home → Kelso');
  });

  it('leaves trips under 3 mi out of efficiency and weights by distance', () => {
    expect(measured(rows).map(t => t.id)).toEqual(['t1', 't2', 't3']);
    expect(averageWhMi(rows)).toBeCloseTo((20 * 300 + 20 * 260 + 120 * 333) / 160);
  });

  it('finds odometer miles between trips', () => {
    expect(odometerGaps(rows, TZ).map(g => g.miles)).toEqual([1]);
  });

  it('groups by month and finds the daily battery band', () => {
    const charges = chargeRows(sessions, places, TZ);
    expect(byMonth(rows, charges)).toEqual([{ month: '2026-10', trips: 4, miles: 161.2, kwh: 94 }]);
    const band = batteryBand(['2026-10-06', '2026-10-07'], rows, charges);
    expect(band[0]).toMatchObject({ day: '2026-10-06', lo: 40, hi: 62 });
  });

  it('keeps the median capacity reading of each day', () => {
    expect(capacityByDay([
      { ts: '2026-10-07T18:00:00Z', usable_kwh: 91.4 }, { ts: '2026-10-07T19:00:00Z', usable_kwh: 91.0 },
      { ts: '2026-10-07T20:00:00Z', usable_kwh: 91.2 }, { ts: '2026-10-08T18:00:00Z', usable_kwh: 0 },
    ], TZ)).toEqual([{ day: '2026-10-07', kwh: 91.2 }]);
  });

  it('gives Insights and Car the same battery health', () => {
    const readings = [
      { ts: '2026-10-06T18:00:00Z', usable_kwh: 91.4 }, { ts: '2026-10-06T19:00:00Z', usable_kwh: 91.2 }, { ts: '2026-10-06T20:00:00Z', usable_kwh: 91.4 },
      { ts: '2026-11-07T17:00:00Z', usable_kwh: 91.0 }, { ts: '2026-11-07T18:00:00Z', usable_kwh: 91.3 }, { ts: '2026-11-07T19:00:00Z', usable_kwh: 91.3 },
    ];
    const days = capacityByDay(readings, TZ);
    expect(usableCapacityNow(days)).toBe(91.3);
    expect(batteryHealthPct(days)).toBeCloseTo(91.3 / 91.4 * 100, 6);
    expect(carBatteryHealthPct(readings, TZ)).toBe(batteryHealthPct(days));
  });

  it('counts only validated parked periods', () => {
    const base = { period_end: null, sleep_share_pct: null, state_coverage_pct: null, soc_start: 70, soc_end: 69, drain_pct_per_hour: null,
      range_start_mi: null, range_end_mi: null, range_lost_mi: null, range_lost_per_hour_mi: null, energy_drained_kwh: null, avg_power_w: null,
      has_reduced_range: null, validation_reason: null, sample_count: 10, start_sample_at: null, end_sample_at: null, movement_detected: false };
    const map = drainByDay([
      { ...base, period_start: '2026-10-07T06:00:00Z', duration_hours: 10, soc_lost_pct: 1, validation_status: 'validated' },
      { ...base, period_start: '2026-10-07T20:00:00Z', duration_hours: 5, soc_lost_pct: 3, validation_status: 'excluded' },
    ] as never, TZ);
    expect(map.get('2026-10-06')).toEqual({ hours: 10, loss: 1, count: 1 });
    expect(map.size).toBe(1);
  });
});

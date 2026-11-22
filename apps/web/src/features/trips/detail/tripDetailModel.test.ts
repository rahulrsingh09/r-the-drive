import { describe, expect, it } from 'vitest';
import type { Insight, Place } from '@panda/types';
import type { UnitPreferences } from '@panda/ui/lib/utils';
import {
  buildConditions, buildSpeedBands, buildTimeline, cumulativeDistanceMiles, efficiencyMeasure, findHomePrice,
  findRouteComparison, minuteSplits, odometerMeasure, rankText, readoutPowerKw, regenEnergyKwh,
  routeDifferenceText, routeTitle, speedCoverageText, summarizeElevation, tripCost, type TimelinePoint,
} from './tripDetailModel';

const imperial: UnitPreferences = {
  mode: 'imperial', distance_unit: 'miles', speed_unit: 'mph', temperature_unit: 'fahrenheit',
  pressure_unit: 'psi', altitude_unit: 'feet', place_radius_unit: 'feet', efficiency_display: 'distance_per_energy',
};
const metric: UnitPreferences = {
  ...imperial, mode: 'metric', distance_unit: 'kilometers', speed_unit: 'kmh', temperature_unit: 'celsius',
  pressure_unit: 'kpa', altitude_unit: 'meters',
};

const point = (elapsed_s: number, extra: Partial<TimelinePoint> = {}): TimelinePoint => ({
  elapsed_s, ts: new Date(elapsed_s * 1000).toISOString(), speed_mph: null, power_kw: null, regen_kw: null,
  estimated_net_power_kw: null, battery_level: null, outside_temp_c: null, cabin_temp_c: null, driver_temp_c: null,
  hvac_active: null, tire_fl_psi: null, tire_fr_psi: null, tire_rl_psi: null, tire_rr_psi: null, altitude_m: null,
  lat: null, lng: null, ...extra,
});

const comparison = (values: Partial<Extract<Insight, { kind: 'route_comparison' }>['values']>, tripId = 'trip-1'): Insight => ({
  id: `route_comparison:trip:${tripId}`, kind: 'route_comparison', severity: 'info', sample_size: 12,
  baseline: { method: 'median', window: 'earlier_drives_same_route', count: 12 },
  evidence: [{ type: 'trip', id: tripId }],
  values: { this_mi_per_kwh: 3.4, usual_mi_per_kwh: 3.1, pct_diff: 9.7, rank: 3, count: 12, ...values },
});

describe('trip detail copy', () => {
  it('titles the trip by its route with an arrow, not "->"', () => {
    expect(routeTitle({ label: 'Home', source: 'place' }, { label: 'Office', source: 'place' })).toBe('Home → Office');
    expect(routeTitle({ label: 'x', source: 'unavailable' }, { label: 'y', source: 'unavailable' })).toBeNull();
  });

  it('places minute ticks for a readable minute axis', () => {
    expect(minuteSplits(1800)).toEqual([0, 600, 1200, 1800]);
    expect(minuteSplits(600)).toEqual([0, 120, 240, 360, 480, 600]);
    expect(minuteSplits(0)).toBeUndefined();
  });

  it('formats efficiency and odometer in the owner units', () => {
    expect(efficiencyMeasure(320, imperial)).toEqual({ value: '3.1', unit: 'mi/kWh' });
    expect(efficiencyMeasure(320, { ...imperial, efficiency_display: 'energy_per_distance' })).toEqual({ value: '320', unit: 'Wh/mi' });
    expect(efficiencyMeasure(320, metric)).toEqual({ value: '5.0', unit: 'km/kWh' });
    expect(efficiencyMeasure(null, imperial)).toBeNull();
    expect(odometerMeasure(12467.4, imperial)).toEqual({ value: '12,467', unit: 'mi' });
    expect(odometerMeasure(null, imperial)).toBeNull();
  });

  it('states speed coverage in one line inside the trip duration', () => {
    expect(speedCoverageText({ durationSeconds: 1800, recordedSeconds: 1800, estimatedSeconds: 0 }))
      .toBe('Speed recorded for 30 of 30 min.');
    expect(speedCoverageText({ durationSeconds: 1800, recordedSeconds: 1200, estimatedSeconds: 480 }))
      .toBe('Speed recorded for 20 of 30 min. GPS estimate for 8 min.');
    expect(speedCoverageText({ durationSeconds: 1800, recordedSeconds: 0, estimatedSeconds: 1800 }))
      .toBe('Speed estimated from GPS for 30 of 30 min.');
    // Bad inputs never show more time than the drive.
    expect(speedCoverageText({ durationSeconds: 1800, recordedSeconds: 41_000, estimatedSeconds: 900 }))
      .toBe('Speed recorded for 30 of 30 min.');
    expect(speedCoverageText({ durationSeconds: 0, recordedSeconds: 0, estimatedSeconds: 0 })).toBeNull();
  });
});

describe('trip detail values', () => {
  it('builds the timeline with regen as negative kW', () => {
    const timeline = buildTimeline('2026-10-08T17:00:00Z', {
      elapsed_s: [0, 60], lat: [1, 2], lng: [3, 4], altitude_m: [5, 6], speed_mph: [10, 20], power_kw: [12, null],
      regen_power_kw: [null, 8], battery_level: [70, 69], outside_temp_c: [10, 11], cabin_temp_c: [20, 21],
      driver_temp_c: [21, 21], hvac_active: [true, false], tire_fl_psi: [44, 44], tire_fr_psi: [45, 45],
      tire_rl_psi: [44, 44], tire_rr_psi: [45, 45],
    });
    expect(timeline[1]).toMatchObject({ ts: '2026-10-08T17:01:00.000Z', regen_kw: -8, power_kw: null });
    expect(readoutPowerKw(timeline[0], false)).toBe(12);
    expect(readoutPowerKw(timeline[1], false)).toBe(-8);
  });

  it('integrates regen energy and ignores long gaps', () => {
    const timeline = [point(0, { regen_kw: -12 }), point(60, { regen_kw: -12 }), point(120, { regen_kw: 0 }), point(1200, { regen_kw: -30 })];
    expect(regenEnergyKwh(timeline)).toBeCloseTo(0.2);
    expect(regenEnergyKwh([point(0)])).toBeNull();
  });

  it('scales GPS distance to the trip distance', () => {
    const timeline = [point(0, { lat: 47.6, lng: -122.34 }), point(60), point(120, { lat: 47.61, lng: -122.34 })];
    const distance = cumulativeDistanceMiles(timeline, 2);
    expect(distance?.[0]).toBe(0);
    expect(distance?.[1]).toBe(0);
    expect(distance?.[2]).toBeCloseTo(2);
    expect(cumulativeDistanceMiles([point(0)], 2)).toBeNull();
  });

  it('summarizes elevation with a dead band and decides the chart threshold', () => {
    const summary = summarizeElevation([10, 10.4, 12, 11.8, 40, 20].map((altitude_m, index) => point(index, { altitude_m })));
    expect(summary).toEqual({ gainM: 30, lossM: 20, rangeM: 30 });
    expect(summarizeElevation([point(0, { altitude_m: 1 })])).toBeNull();
  });

  it('lists conditions in the owner units and drops implausible tire readings', () => {
    const timeline = [
      point(0, { outside_temp_c: 8, cabin_temp_c: 21, hvac_active: true, tire_fl_psi: 44, tire_fr_psi: 45, tire_rl_psi: 0, tire_rr_psi: 44 }),
      point(60, { outside_temp_c: 8, cabin_temp_c: 21, tire_fl_psi: 44, tire_fr_psi: 45, tire_rl_psi: 44, tire_rr_psi: 44 }),
    ];
    const elevation = { gainM: 11.6, lossM: 3.7, rangeM: 12 };
    expect(buildConditions({ timeline, elevation, prefs: imperial, driveModeLabel: 'All-Purpose' })).toEqual([
      { key: 'outside', label: 'Outside', value: '46 °F' },
      { key: 'cabin', label: 'Cabin', value: '70 °F · climate on' },
      { key: 'elevation', label: 'Elevation', value: '+38 ft, −12 ft' },
      { key: 'tires', label: 'Tires', value: '44–45 psi' },
      { key: 'drive_mode', label: 'Drive mode', value: 'All-Purpose' },
    ]);
    expect(buildConditions({ timeline, elevation, prefs: metric, outsideSamples: [{ temperature_c: 9 }], outsideEstimated: true })
      .map(row => row.value)).toEqual(['9 °C · estimated', '21 °C · climate on', '+12 m, −4 m', '303–310 kPa']);
    expect(buildConditions({ timeline: [point(0)], elevation: null, prefs: imperial })).toEqual([]);
  });

  it('joins 5 mph bins into 10 mph bands and trims empty ends', () => {
    const bin = (min: number, duration_seconds: number) => ({
      label: `${min}-${min + 5}`, min, max: min + 5, count: 1, duration_seconds, sample_elapsed_s: 0,
    });
    const bins = [bin(0, 0), bin(5, 0), bin(10, 60), bin(15, 120), bin(20, 0), bin(25, 0), bin(30, 300), bin(35, 0)];
    expect(buildSpeedBands(bins, imperial)).toEqual([
      { label: '10–20', min: 10, max: 20, seconds: 180 },
      { label: '20–30', min: 20, max: 30, seconds: 0 },
      { label: '30–40', min: 30, max: 40, seconds: 300 },
    ]);
    expect(buildSpeedBands(bins, metric)[0]!.label).toBe('16–32');
    expect(buildSpeedBands([], imperial)).toEqual([]);
  });
});

describe('route comparison and cost', () => {
  it('uses only the comparison for this trip', () => {
    expect(findRouteComparison([comparison({})], 'trip-1')?.rank).toBe(3);
    expect(findRouteComparison([comparison({}, 'trip-2')], 'trip-1')).toBeNull();
    expect(findRouteComparison(undefined, 'trip-1')).toBeNull();
  });

  it('writes rank and difference copy', () => {
    const values = findRouteComparison([comparison({})], 'trip-1')!;
    expect(rankText(values)).toBe('3rd best of 13 drives');
    expect(rankText({ ...values, rank: 1 })).toBe('Best of 13 drives');
    expect(rankText({ ...values, rank: 12 })).toBe('12th best of 13 drives');
    expect(routeDifferenceText(values)).toBe('10% better than your usual on this route');
    expect(routeDifferenceText({ ...values, pct_diff: -4.2 })).toBe('4% worse than your usual on this route');
    expect(routeDifferenceText({ ...values, pct_diff: 0.2 })).toBe('Same as your usual on this route');
  });

  it('prices the trip only from a per-kWh home price', () => {
    const place = (charging: Place['charging'], is_home = true): Place => ({
      id: 'home', name: 'Home', latitude: 0, longitude: 0, radius_m: 100, is_home, is_work: false, address: null, charging,
    });
    const perKwh = { id: 'c', name: 'Home', billing_type: 'per_kwh' as const, rate: 0.12, session_fee: 0, currency: 'USD', timezone: null, tou_periods: [] };
    expect(findHomePrice([place(perKwh)])).toEqual({ rate: 0.12, currency: 'USD' });
    expect(tripCost({ energy_used_kwh: 4.5 }, findHomePrice([place(perKwh)]))).toBeCloseTo(0.54);
    expect(findHomePrice([place({ ...perKwh, billing_type: 'tou' })])).toBeNull();
    expect(findHomePrice([place(perKwh, false)])).toBeNull();
    expect(findHomePrice([place(null)])).toBeNull();
    expect(tripCost({ energy_used_kwh: null }, { rate: 0.12, currency: 'USD' })).toBeNull();
  });
});

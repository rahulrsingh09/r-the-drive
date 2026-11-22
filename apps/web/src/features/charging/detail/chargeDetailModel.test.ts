import { describe, expect, it } from 'vitest';
import type { ChargeCurvePoint, ChargeSession, Place } from '@panda/types';
import {
  buildChargeCurve, chargeKind, chargeTitle, costDisplay, costEntry, costMethodLabel, currentPriceLine, formatRate,
  heroLine, matchSavedPlace, nearestIndexByMinute, parseAmount, phaseKw, phaseSentence, placeRateBlock, placeRatePlan,
  pointContext, timeRangeText,
} from './chargeDetailModel';

const base: ChargeSession = {
  id: 's1', vehicle_id: 'v1', started_at: '2026-11-06T00:08:00Z', ended_at: '2026-11-06T00:30:00Z',
  location_name: 'Bellevue', charger_type: 'dc', energy_added_kwh: 28.6, duration_min: 22, soc_start: 40, soc_end: 71,
  peak_power_kw: 108, cost_usd: null, cost_method: 'unknown', network_vendor: 'Tesla', is_public: true,
  rivian_city: 'Bellevue', location_lat: 47.61, location_lng: -122.2,
};

const place = (overrides: Partial<Place> = {}): Place => ({
  id: 'p1', name: 'Bellevue Supercharger', latitude: 47.6101, longitude: -122.2001, radius_m: 75,
  is_home: false, is_work: false, address: null, charging: null, ...overrides,
});

describe('header', () => {
  it('uses the network name for a public charge and moves the place to the subtitle', () => {
    expect(chargeTitle(base, null)).toEqual({ title: 'Tesla Supercharger', where: 'Bellevue' });
    expect(chargeTitle({ ...base, network_vendor: null, is_public: false, location_name: 'Home' }, null)).toEqual({ title: 'Home', where: 'Bellevue' });
    expect(chargeTitle(base, place()).title).toBe('Bellevue Supercharger');
  });

  it('marks DC FAST and AC charges', () => {
    expect(chargeKind(base)).toBe('dc');
    expect(chargeKind({ ...base, charger_type: null, network_vendor: null, peak_power_kw: 11 })).toBe('ac');
    expect(chargeKind({ ...base, charger_type: null, network_vendor: null, peak_power_kw: null })).toBeNull();
  });

  it('writes a compact time range', () => {
    expect(timeRangeText('4:08 PM', '4:30 PM')).toBe('4:08–4:30 PM');
    expect(timeRangeText('11:50 PM', '12:20 AM')).toBe('11:50 PM–12:20 AM');
  });

  it('writes the hero line with an arrow, the time and the range', () => {
    expect(heroLine({ socStart: 40, socEnd: 71, minutes: 22, rangeKm: 148.06, distanceUnit: 'miles', live: false }))
      .toBe('40% → 71% in 22 min · about +92 mi of range');
    expect(heroLine({ socStart: 40, socEnd: 55, minutes: 9, rangeKm: null, distanceUnit: 'miles', live: true })).toBe('40% → 55% 9 min so far');
  });
});

describe('cost', () => {
  it('labels every cost method', () => {
    expect(costMethodLabel('rivian_paid_total', false)).toBe('Billed by Rivian');
    expect(costMethodLabel('profile', true)).toBe('Your home rate');
    expect(costMethodLabel('profile', false)).toBe('Place rate');
    expect(costMethodLabel('user_manual', false)).toBe('Your price');
    for (const method of ['user_free', 'network_free', 'rivian_free_session']) expect(costMethodLabel(method, false)).toBe('Free');
    expect(costMethodLabel('unknown', false)).toBe('No price');
    expect(costMethodLabel('profile_pending', false)).toBe('Price pending');
  });

  it('formats rates in cents below a dollar', () => {
    expect(formatRate(0.5)).toBe('50¢');
    expect(formatRate(0.485)).toBe('48.5¢');
    expect(formatRate(1.2)).toBe('$1.20');
  });

  it('previews a total as a rate, and a rate as a total rounded to cents', () => {
    expect(costEntry('total', '14.30', 28.6)).toMatchObject({ total: 14.3, preview: '= 50¢ per kWh for 28.6 kWh', error: null });
    expect(costEntry('rate', '0.50', 28.6)).toMatchObject({ total: 14.3, rate: 0.5, preview: '= $14.30' });
    expect(costEntry('rate', '0.479', 28.6).total).toBe(13.7);
    expect(costEntry('rate', '45¢', 28.6).total).toBe(12.87);
    expect(costEntry('total', '-1', 28.6).error).toMatch(/0 or more/);
    expect(costEntry('rate', '0.5', null).error).toMatch(/Enter the total paid/);
    expect(costEntry('total', '', 28.6)).toMatchObject({ total: null, error: null });
  });

  it('reads one decimal comma and rejects other commas', () => {
    expect(parseAmount('14,30', 'dollars')).toBe(14.3);
    expect(parseAmount('$14,3', 'dollars')).toBe(14.3);
    expect(parseAmount('0,45', 'rate')).toBe(0.45);
    expect(parseAmount('48,5¢', 'rate')).toBe(0.485);
    expect(costEntry('total', '14,30', 28.6)).toMatchObject({ total: 14.3, error: null });
    expect(costEntry('rate', '0,50', 28.6)).toMatchObject({ total: 14.3, rate: 0.5, error: null });
    for (const input of ['1,430', '1,430.00', '14,30,1', ',30', '14,']) {
      expect(parseAmount(input, 'dollars')).toBeNull();
      expect(costEntry('total', input, 28.6)).toMatchObject({
        total: null, error: 'Do not use a comma for thousands. Use a period or one comma for the decimal, for example 14.30.',
      });
    }
    expect(costEntry('rate', '0,479', 28.6).error).toBe('Do not use a comma for thousands. Use a period or one comma for the decimal, for example 0.50.');
  });

  it('shows the method on the Cost tile', () => {
    expect(costDisplay(base, false)).toMatchObject({ value: 'No price', attention: true });
    const manual = { ...base, cost_method: 'user_manual', cost_usd: 14.3, cost_rate_per_kwh: 0.5 } as ChargeSession;
    expect(costDisplay(manual, false)).toMatchObject({ value: '$14.30', context: 'Your price · 50¢/kWh' });
    expect(currentPriceLine(manual, false)).toBe('Your price $14.30 · 50¢/kWh');
    expect(costDisplay({ ...base, cost_method: 'rivian_paid_total', cost_usd: 9.12 }, false)).toMatchObject({ value: '$9.12', context: 'Billed by Rivian' });
    expect(costDisplay({ ...base, cost_method: 'network_free', cost_usd: 0 }, false)).toMatchObject({ value: '$0.00', context: 'Free' });
    expect(costDisplay({ ...base, cost_method: 'profile_pending' }, false).value).toBe('Price pending');
    expect(costDisplay({ ...base, ended_at: null, live_current_price: 3.2 }, false)).toMatchObject({ value: 'Pending', context: 'About $3.20 so far' });
  });
});

describe('saved place rate', () => {
  it('matches the saved place that holds the charge', () => {
    expect(matchSavedPlace(base, [place({ id: 'far', latitude: 47.7 }), place()])?.id).toBe('p1');
    expect(matchSavedPlace({ ...base, location_lat: null, location_lng: null, location_name: 'Bellevue Supercharger' }, [place()])?.id).toBe('p1');
    expect(matchSavedPlace(base, [place({ latitude: 47.7 })])).toBeNull();
  });

  it('updates the matched place, keeps its fee and name, and says that past automatic charges are priced again', () => {
    const plan = placeRatePlan(base, place({ is_work: true, charging: {
      id: 'c', name: 'Old', billing_type: 'per_kwh', rate: 0.2, session_fee: 1, currency: 'USD', timezone: 'America/Los_Angeles', tou_periods: [],
    } }), 0.5, 'UTC', 'Tesla Supercharger')!;
    expect(plan.placeId).toBe('p1');
    expect(plan.body).toMatchObject({ name: 'Bellevue Supercharger', is_work: true, charging: { billing_type: 'per_kwh', rate: 0.5, session_fee: 1, timezone: 'America/Los_Angeles' } });
    expect(plan.confirm).toBe('This sets Bellevue Supercharger to 50¢/kWh. Past automatic charges at Bellevue Supercharger are priced again with this rate.');
    expect(plan.description).toBe(plan.confirm);
  });

  it('does not offer the rate for the Home place or a time-of-use place', () => {
    const tou = { id: 'c', name: 'TOU', billing_type: 'tou' as const, rate: 0, session_fee: 0, currency: 'USD', timezone: 'UTC', tou_periods: [] };
    expect(placeRateBlock(place({ is_home: true }), true)).toBe('Change the Home price in Settings → Places.');
    expect(placeRateBlock(null, true)).toBe('Change the Home price in Settings → Places.');
    expect(placeRateBlock(place({ charging: tou }), false)).toBe('Bellevue Supercharger has time-of-use rates. Change its price in Settings → Places.');
    expect(placeRateBlock(place(), false)).toBeNull();
    expect(placeRateBlock(null, false)).toBeNull();
  });

  it('creates a place at the charge location, and needs a location', () => {
    const plan = placeRatePlan(base, null, 0.5, 'UTC', 'Tesla Supercharger')!;
    expect(plan.placeId).toBeNull();
    expect(plan.confirm).toBeNull();
    expect(plan.body).toMatchObject({ name: 'Tesla Supercharger', radius_m: 75, address: { latitude: 47.61, longitude: -122.2, osm_id: null }, charging: { billing_type: 'per_kwh', rate: 0.5 } });
    expect(plan.description).toBe('Adds the saved place “Tesla Supercharger” at this charger with 50¢ per kWh. Automatic charges at this place use this rate.');
    expect(placeRatePlan({ ...base, location_lat: null }, null, 0.5, 'UTC', 'X')).toBeNull();
  });
});

describe('curve', () => {
  const points = (rows: Array<[number, number, number]>, method = 'recorded'): ChargeCurvePoint[] =>
    rows.map(([minutes_elapsed, power_kw, soc_pct]) => ({ minutes_elapsed, power_kw, soc_pct, power_method: method }));

  it('fills a missing battery % by time and finds the peak and the lowest point after it', () => {
    const curve = buildChargeCurve(points([[0, 60, 40], [2, 108, 0], [4, 106, 46], [22, 52, 71]]), base)!;
    expect(curve.points[1]!.soc).toBe(43);
    expect(curve.socAxis).toBe(true);
    expect(curve.peakIndex).toBe(1);
    expect(curve.lowIndex).toBe(3);
    expect(curve.estimated).toBe(false);
  });

  it('counts minutes from the session start, the same origin as the API shape', () => {
    // The first reading is 3 minutes after the session start; the API peak_minute is 5 (from the start).
    const curve = buildChargeCurve(points([[3, 60, 40], [5, 108, 44], [10, 90, 55], [25, 52, 71]]), base)!;
    expect(curve.points.map((point) => point.minute)).toEqual([3, 5, 10, 25]);
    expect(curve.startMin).toBe(0);
    expect(curve.durationMin).toBe(25);
    expect(nearestIndexByMinute(curve.points, 5)).toBe(curve.peakIndex);
    // A reading before the session start keeps its minute below 0, and the axis starts there.
    const early = buildChargeCurve(points([[-1.5, 20, 40], [2, 108, 44], [22, 52, 71]]), base)!;
    expect(early.points[0]!.minute).toBe(-1.5);
    expect(early.startMin).toBe(-1.5);
  });

  it('prefers measured points and uses estimates only when nothing was measured', () => {
    expect(buildChargeCurve([...points([[0, 50, 40]], 'soc_delta'), ...points([[0, 60, 40], [5, 90, 50]])], base)!.points).toHaveLength(2);
    expect(buildChargeCurve(points([[0, 11, 40], [60, 11, 60]], 'soc_delta'), base)!.estimated).toBe(true);
    expect(buildChargeCurve(points([[0, 11, 40]]), base)).toBeNull();
  });

  it('writes one sentence per phase', () => {
    const shape = { peak_kw: 108, peak_soc: 44, peak_minute: 2, low_kw: 52, low_soc: 71, phases: [
      { name: 'climb' as const, start_min: 0, end_min: 2, from_kw: 60, to_kw: 108 },
      { name: 'peak' as const, start_min: 2, end_min: 6, from_kw: 108, to_kw: 105 },
      { name: 'taper' as const, start_min: 6, end_min: 22, from_kw: 105, to_kw: 52 },
    ] };
    expect(phaseSentence(shape.phases[0]!, shape, false)).toBe('Power climbed from 60 to 108 kW in the first 2 min.');
    expect(phaseSentence(shape.phases[1]!, shape, false)).toBe('Peak 108 kW at 44%. Power stayed above 105 kW for 4 min.');
    expect(phaseSentence(shape.phases[2]!, shape, false)).toBe('Then a steady taper to 52 kW at 71% as the battery filled.');
    expect(phaseSentence(shape.phases[2]!, shape, true)).toMatch(/charger probably limited/);
    expect(phaseKw(shape.phases[1]!)).toBe('105–108 kW');
    expect(phaseKw(shape.phases[2]!)).toBe('105 → 52 kW');
    expect(pointContext(44, 2, false)).toBe('At 44%, 2 min in');
    expect(pointContext(71, 22, true)).toBe('At 71%, at the end');
  });
});

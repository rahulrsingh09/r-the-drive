import { describe, expect, it } from 'vitest';
import type { Trip, VehicleHealth, VehicleStatus } from '@panda/types';
import {
  batteryHealthPct,
  closures,
  missedCharges,
  sampleBars,
  statusFlag,
  tireReadings,
  tripCoverage,
  vehicleHealthRows,
} from './carModel';

const status = (fields: Partial<VehicleStatus>) => ({ vehicle_id: 'v', is_online: true, ...fields }) as VehicleStatus;
const trip = (started: string, distance: number, start: number | null, end: number | null) =>
  ({ id: started, vehicle_id: 'v', started_at: started, distance_mi: distance, start_odometer_mi: start, end_odometer_mi: end }) as Trip;

describe('Car tab model', () => {
  it('reads yes/no fields, and treats unseen or unknown values as not reported', () => {
    expect(statusFlag(status({ brake_fluid_low: false }), 'brake_fluid_low')).toBe(false);
    expect(statusFlag(status({ charger_derate_active: 'active' }), 'charger_derate_active')).toBe(true);
    expect(statusFlag(status({ charger_derate_active: 'mystery' }), 'charger_derate_active')).toBeNull();
    expect(statusFlag(status({
      alarm_active: false,
      field_availability: { alarm_active: { availability: 'never_seen', reason_code: null, ever_seen: false, last_seen_at: null, latest_event_at: null } },
    } as Partial<VehicleStatus>), 'alarm_active')).toBeNull();
  });

  it('lists the vehicle health rows in trial order with Not reported for missing fields', () => {
    const tires = tireReadings(status({ tire_fl_psi: 40.6, tire_fr_psi: 41, tire_rl_psi: 40.6, tire_rr_psi: 40.6 }), null, 40);
    const rows = vehicleHealthRows(status({ service_mode: false, charger_derate_active: true, ota_current_version: '2026.36.42' }), null, tires);
    expect(rows.map(row => [row.label, row.value])).toEqual([
      ['Battery thermal', 'Not reported'],
      ['12 V battery', 'Not reported'],
      ['Tires', 'All OK'],
      ['Brake fluid', 'Not reported'],
      ['Washer fluid', 'Not reported'],
      ['Alarm', 'Not reported'],
      ['Service mode', 'Off'],
      ['Charging power limit', 'Reported on'],
      ['Software', 'Up to date'],
    ]);
    expect(rows.find(row => row.key === 'derate')?.detail).toBe('Last report. It only matters while charging.');
    expect(rows.find(row => row.key === 'software')?.detail).toBe('2026.36.42');
  });

  it('says which software version is ready', () => {
    const rows = vehicleHealthRows(status({ ota_current_version: '2026.36.42', ota_available_version: '2026.40.1' }), null, []);
    expect(rows.find(row => row.key === 'software')).toMatchObject({ state: 'warn', value: '2026.40.1 ready' });
  });

  it('marks a low tire and a missing sensor', () => {
    const readings = tireReadings(status({ tire_fl_psi: 34, tire_fr_psi: 40, tire_rl_psi: 40, tire_rr_valid: false, tire_rr_psi: 40 }), null, 40);
    expect(readings.map(tire => tire.note)).toEqual(['Low', 'OK', 'OK', 'Not reported']);
  });

  it('reports closures, lock first', () => {
    const health = { closures: { closure_liftgate_closed: false } } as unknown as VehicleHealth;
    const items = closures(status({ door_front_left_closed: true, closure_frunk_closed: true, window_fl_closed: true }), health, true);
    expect(items.map(item => `${item.label}: ${item.value}`)).toEqual(['Doors: Locked', 'Frunk: Closed', 'Liftgate: Open', 'Windows: Closed']);
    expect(closures(status({}), null, null).map(item => item.value)).toEqual(['Not reported', 'Not reported', 'Not reported', 'Not reported']);
  });

  it('computes battery health from the first and the latest day', () => {
    expect(batteryHealthPct([
      { ts: '2026-10-06T01:00:00Z', usable_kwh: 87 },
      { ts: '2026-10-06T05:00:00Z', usable_kwh: 88 },
      { ts: '2026-10-06T09:00:00Z', usable_kwh: 88 },
      { ts: '2026-11-07T09:00:00Z', usable_kwh: 87.8 },
    ], 'UTC')).toBeCloseTo(99.77, 1);
    expect(batteryHealthPct([])).toBeNull();
    // A non-list answer gives no reading instead of an error.
    expect(batteryHealthPct({} as unknown as [])).toBeNull();
  });

  it('compares the odometer with recorded trips', () => {
    const coverage = tripCoverage([
      trip('2026-10-08T00:00:00Z', 9, 110.5, 119.5),
      trip('2026-10-07T00:00:00Z', 10, 100, 110),
    ], 121);
    expect(coverage).toMatchObject({ odometerMiles: 21, tripMiles: 19, unrecordedMiles: 2, shortMoves: 1, activityMoves: 0 });
    expect(coverage?.coveredPct).toBeCloseTo(90.48, 1);
    expect(tripCoverage([trip('2026-10-07T00:00:00Z', 10, null, null)], 121)).toBeNull();
  });

  it('caps the samples bars and marks quiet days', () => {
    const bars = sampleBars([
      { day: '2026-11-02', samples: 1000 }, { day: '2026-11-01', samples: 1000 },
      { day: '2026-11-03', samples: 100 }, { day: '2026-11-04', samples: 9000 },
    ]);
    expect(bars.map(bar => bar.day)).toEqual(['2026-11-01', '2026-11-02', '2026-11-03', '2026-11-04']);
    expect(bars.map(bar => bar.low)).toEqual([false, false, true, false]);
    expect(bars[3]!.height).toBe(1);
    expect(bars[0]!.height).toBeCloseTo(0.4);
  });

  it('says None found only after a successful empty check', () => {
    expect(missedCharges(undefined, false).value).toBe('Checking…');
    expect(missedCharges(undefined, true).value).toBe('Not checked');
    expect(missedCharges({ vehicle_id: 'v', lookback_days: 30, gaps: [], parked_periods_checked: 69 }, false))
      .toEqual({ state: 'ok', value: 'None found', checked: 69 });
    expect(missedCharges({ vehicle_id: 'v', lookback_days: 30, gaps: [
      { id: 'a', started_at: '', ended_at: '', soc_start: 40, soc_end: 60 },
    ] }, false)).toMatchObject({ state: 'warn', value: '1 possible' });
  });
});

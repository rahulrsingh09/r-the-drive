/**
 * Car tab model: pure functions that turn the vehicle status, the health
 * snapshot and the data-health reads into rows for the page. No React here,
 * so each rule has a unit test (carModel.test.ts).
 */
import type {
  ChargeGapsResponse,
  DailySampleCount,
  Trip,
  VehicleHealth,
  VehicleStatus,
} from '@panda/types';
import { getTireHealthTone } from '@panda/ui/lib/vehicleTires';
import { getAppTimezone } from '@panda/ui/lib/dateTime';
import { batteryHealthPct as sharedBatteryHealthPct, capacityByDay } from '../../lib/batteryHealth';

type TireHealthTone = ReturnType<typeof getTireHealthTone>;

export type RowState = 'ok' | 'warn' | 'none';

export interface StatusRow {
  key: string;
  label: string;
  state: RowState;
  value: string;
  detail?: string | undefined;
}

const NOT_REPORTED = 'Not reported';

function unavailable(status: VehicleStatus | null | undefined, field: string) {
  const availability = status?.field_availability?.[field];
  return availability?.availability === 'never_seen' || availability?.reason_code === 'invalid_sensor';
}

/** A yes/no reading from the status. Strings such as "active" or "off" count; anything else is not reported. */
export function statusFlag(status: VehicleStatus | null | undefined, field: keyof VehicleStatus): boolean | null {
  if (unavailable(status, field)) return null;
  const value = status?.[field];
  if (typeof value === 'boolean') return value;
  if (typeof value !== 'string') return null;
  if (/^(true|on|active|yes|1)$/i.test(value.trim())) return true;
  if (/^(false|off|inactive|no|0|none)$/i.test(value.trim())) return false;
  return null;
}

function titleCase(value: string) {
  return value.replace(/[_-]+/g, ' ').trim().replace(/\b\w/g, letter => letter.toUpperCase());
}

/** Rows for a yes/no warning field: null = Not reported, true = the bad text, false = the good text. */
function flagRow(key: string, label: string, flag: boolean | null, okText: string, badText: string): StatusRow {
  if (flag === null) return { key, label, state: 'none', value: NOT_REPORTED };
  return flag ? { key, label, state: 'warn', value: badText } : { key, label, state: 'ok', value: okText };
}

export interface TireReading {
  position: 'fl' | 'fr' | 'rl' | 'rr';
  label: string;
  psi: number | null;
  tone: TireHealthTone;
  /** Short status under the pressure: OK, Low, Check, or Not reported. */
  note: string;
}

const TIRES = [['fl', 'Front left'], ['fr', 'Front right'], ['rl', 'Rear left'], ['rr', 'Rear right']] as const;

function finite(value: unknown) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function tireReadings(
  status: VehicleStatus | null | undefined,
  tires: VehicleHealth['tires'] | null | undefined,
  targetPsi: number,
): TireReading[] {
  return TIRES.map(([position, label]) => {
    const psiField = `tire_${position}_psi` as const;
    const statusField = `tire_${position}_status` as const;
    const invalid = status?.[`tire_${position}_valid`] === false || unavailable(status, psiField);
    const psi = invalid ? null : finite(status?.[psiField]) ?? finite(tires?.[psiField]);
    const reported = status?.[statusField] ?? tires?.[statusField] ?? null;
    const tone = getTireHealthTone({ psi, status: invalid ? 'invalid_sensor' : reported, targetPsi });
    // The shared tone rule flags only pressure below the target.
    const note = tone === 'success' ? 'OK'
      : tone === 'danger' || tone === 'warning' ? 'Low'
        : psi === null ? NOT_REPORTED : 'Check';
    return { position, label, psi, tone, note };
  });
}

export function tiresRow(readings: TireReading[]): StatusRow {
  const base = { key: 'tires', label: 'Tires' };
  if (readings.some(tire => tire.tone === 'warning' || tire.tone === 'danger')) return { ...base, state: 'warn', value: 'Check' };
  if (readings.every(tire => tire.tone === 'success')) return { ...base, state: 'ok', value: 'All OK' };
  if (readings.every(tire => tire.psi === null)) return { ...base, state: 'none', value: NOT_REPORTED };
  return { ...base, state: 'none', value: 'Partly reported' };
}

export interface SoftwareState {
  current: string | null;
  update: string | null;
}

export function softwareState(status: VehicleStatus | null | undefined, health: VehicleHealth | null | undefined): SoftwareState {
  const history = health?.software_history ?? [];
  const current = status?.ota_current_version?.trim()
    || health?.current_software_version?.trim()
    || health?.latest?.ota_current_version?.trim()
    || history.find(entry => !entry.observed_until)?.version
    || null;
  const available = (status?.ota_available_version ?? health?.latest?.ota_available_version ?? '').trim();
  const update = available && !/^0+(\.0+)*$/.test(available) && available !== current ? available : null;
  return { current, update };
}

/** The "Vehicle health" list, in the trial order. Missing fields show "Not reported". */
export function vehicleHealthRows(
  status: VehicleStatus | null | undefined,
  health: VehicleHealth | null | undefined,
  tires: TireReading[],
): StatusRow[] {
  const thermalValue = (unavailable(status, 'hv_thermal_event') ? null : status?.hv_thermal_event) ?? health?.latest?.hv_thermal_event ?? null;
  const thermalEvents = health?.thermal_events_30d ?? 0;
  const thermal: StatusRow = !thermalValue?.trim()
    ? { key: 'thermal', label: 'Battery thermal', state: 'none', value: NOT_REPORTED }
    : /^(off|none|inactive|normal|ok|good|no[ _]?event)$/i.test(thermalValue.trim())
      ? { key: 'thermal', label: 'Battery thermal', state: 'ok', value: 'No event' }
      : { key: 'thermal', label: 'Battery thermal', state: 'warn', value: 'Event reported' };
  if (thermalEvents > 0) thermal.detail = thermalEvents === 1 ? '1 event in 30 days' : `${thermalEvents} events in 30 days`;

  const twelveVoltValue = (unavailable(status, 'twelve_volt_health') ? null : status?.twelve_volt_health) ?? health?.latest?.twelve_volt_health ?? null;
  const twelveVolt: StatusRow = !twelveVoltValue?.trim()
    ? { key: 'twelve-volt', label: '12 V battery', state: 'none', value: NOT_REPORTED }
    : { key: 'twelve-volt', label: '12 V battery', state: /ok|good|normal/i.test(twelveVoltValue) ? 'ok' : 'warn', value: titleCase(twelveVoltValue) };

  const derate = statusFlag(status, 'charger_derate_active');
  const derateRow = flagRow('derate', 'Charging power limit', derate, 'Off', 'Reported on');
  if (derate) derateRow.detail = 'Last report. It only matters while charging.';

  const software = softwareState(status, health);
  const softwareRow: StatusRow = !software.current
    ? { key: 'software', label: 'Software', state: 'none', value: NOT_REPORTED }
    : software.update
      ? { key: 'software', label: 'Software', state: 'warn', value: `${software.update} ready`, detail: software.current }
      : { key: 'software', label: 'Software', state: 'ok', value: 'Up to date', detail: software.current };

  return [
    thermal,
    twelveVolt,
    tiresRow(tires),
    flagRow('brake-fluid', 'Brake fluid', statusFlag(status, 'brake_fluid_low'), 'OK', 'Low'),
    flagRow('washer-fluid', 'Washer fluid', statusFlag(status, 'wiper_fluid_low'), 'OK', 'Low'),
    flagRow('alarm', 'Alarm', statusFlag(status, 'alarm_active'), 'Quiet', 'Active'),
    flagRow('service-mode', 'Service mode', statusFlag(status, 'service_mode'), 'Off', 'On'),
    derateRow,
    softwareRow,
  ];
}

export interface Closure {
  key: string;
  label: string;
  state: RowState;
  value: string;
}

function allOf(values: Array<boolean | null>): boolean | null {
  const known = values.filter((value): value is boolean => value !== null);
  if (known.length === 0) return null;
  return known.every(Boolean);
}

/** Doors (lock and open state), frunk, liftgate and windows. */
export function closures(status: VehicleStatus | null | undefined, health: VehicleHealth | null | undefined, locked: boolean | null): Closure[] {
  const read = (field: keyof VehicleStatus & keyof NonNullable<VehicleHealth['closures']>) =>
    statusFlag(status, field) ?? (typeof health?.closures?.[field] === 'boolean' ? health.closures[field] as boolean : null);
  const doorsClosed = allOf([
    read('door_front_left_closed'), read('door_front_right_closed'),
    read('door_rear_left_closed'), read('door_rear_right_closed'),
  ]);
  const doors: Closure = doorsClosed === false
    ? { key: 'doors', label: 'Doors', state: 'warn', value: 'Open' }
    : locked === true ? { key: 'doors', label: 'Doors', state: 'ok', value: 'Locked' }
      : locked === false ? { key: 'doors', label: 'Doors', state: 'warn', value: 'Unlocked' }
        : doorsClosed ? { key: 'doors', label: 'Doors', state: 'ok', value: 'Closed' }
          : { key: 'doors', label: 'Doors', state: 'none', value: NOT_REPORTED };
  const openClosed = (key: string, label: string, closed: boolean | null): Closure => closed === null
    ? { key, label, state: 'none', value: NOT_REPORTED }
    : { key, label, state: closed ? 'ok' : 'warn', value: closed ? 'Closed' : 'Open' };
  return [
    doors,
    openClosed('frunk', 'Frunk', read('closure_frunk_closed')),
    openClosed('liftgate', 'Liftgate', read('closure_liftgate_closed') ?? read('closure_tailgate_closed')),
    openClosed('windows', 'Windows', allOf([
      statusFlag(status, 'window_fl_closed'), statusFlag(status, 'window_fr_closed'),
      statusFlag(status, 'window_rl_closed'), statusFlag(status, 'window_rr_closed'),
    ])),
  ];
}

/** Battery health in %, with the shared rule of Car and Insights (median per app-timezone day, latest ÷ first). */
export function batteryHealthPct(rows: Array<{ ts: string; usable_kwh: number }> | null | undefined, timezone = getAppTimezone()): number | null {
  return sharedBatteryHealthPct(capacityByDay(rows, timezone));
}

export interface TripCoverage {
  /** Odometer miles since the first trip with an odometer reading. */
  odometerMiles: number;
  tripMiles: number;
  coveredPct: number;
  unrecordedMiles: number;
  /** Odometer moves between consecutive trips (more than 0.3 mi). */
  shortMoves: number;
  /** Moves of 1 mi or more; Activity lists these. */
  activityMoves: number;
  largestMoveMiles: number;
}

/** Odometer against recorded trips, the way the trial counts it. Trips can come in any order. */
export function tripCoverage(trips: Trip[] | null | undefined, odometerNow: number | null): TripCoverage | null {
  const ordered = (trips ?? []).slice().sort((a, b) => a.started_at.localeCompare(b.started_at));
  const firstIndex = ordered.findIndex(trip => finite(trip.start_odometer_mi) !== null);
  if (firstIndex < 0 || odometerNow === null) return null;
  const counted = ordered.slice(firstIndex);
  const odometerMiles = odometerNow - finite(counted[0]!.start_odometer_mi)!;
  if (!(odometerMiles > 0)) return null;
  const tripMiles = counted.reduce((sum, trip) => sum + (finite(trip.distance_mi) ?? 0), 0);
  let shortMoves = 0;
  let activityMoves = 0;
  let largestMoveMiles = 0;
  for (let index = 1; index < counted.length; index += 1) {
    const end = finite(counted[index - 1]!.end_odometer_mi);
    const start = finite(counted[index]!.start_odometer_mi);
    if (end === null || start === null) continue;
    const move = start - end;
    if (move > 0.3) {
      shortMoves += 1;
      if (move >= 1) activityMoves += 1;
      largestMoveMiles = Math.max(largestMoveMiles, move);
    }
  }
  return {
    odometerMiles,
    tripMiles,
    coveredPct: Math.min(100, (tripMiles / odometerMiles) * 100),
    unrecordedMiles: Math.max(0, odometerMiles - tripMiles),
    shortMoves,
    activityMoves,
    largestMoveMiles,
  };
}

export interface SampleBar {
  day: string;
  samples: number;
  /** Bar height, 0–1 of the chart. */
  height: number;
  /** Few samples: trips or charges on that day can be incomplete. */
  low: boolean;
}

/** Samples per day bars. The top is capped at 2.5 × the median so one busy day does not flatten the rest. */
export function sampleBars(days: DailySampleCount[] | null | undefined): SampleBar[] {
  const rows = (days ?? []).filter(day => Number.isFinite(day.samples) && day.samples >= 0)
    .slice().sort((a, b) => a.day.localeCompare(b.day));
  if (rows.length === 0) return [];
  const sorted = rows.map(day => day.samples).sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)]!;
  const top = Math.max(1, Math.min(sorted[sorted.length - 1]!, median * 2.5));
  return rows.map(day => ({
    day: day.day,
    samples: day.samples,
    height: Math.min(day.samples, top) / top,
    low: day.samples < median * 0.3,
  }));
}

export type MissedChargesState =
  | { state: 'ok'; value: 'None found'; checked: number | null }
  | { state: 'warn'; value: string; checked: number | null }
  | { state: 'none'; value: 'Not checked' | 'Checking…'; checked: null };

/** "None found" only when the call succeeded with an empty list. */
export function missedCharges(data: ChargeGapsResponse | undefined, failed: boolean): MissedChargesState {
  if (!data) return { state: 'none', value: failed ? 'Not checked' : 'Checking…', checked: null };
  const checked = finite(data.parked_periods_checked);
  const count = data.gaps?.length ?? 0;
  if (count === 0) return { state: 'ok', value: 'None found', checked };
  return { state: 'warn', value: `${count} possible`, checked };
}

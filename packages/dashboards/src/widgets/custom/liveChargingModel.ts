import type { VehicleStatus } from '@panda/types';
import type { useLiveSession } from '@panda/hooks';

type Session = ReturnType<typeof useLiveSession>['data'];
const VEHICLE_FIELDS = {
  soc: 'battery_level',
  restingSoc: 'battery_level',
  time: 'time_to_end_of_charge_min',
  limit: 'battery_limit',
  range: 'range_miles',
} as const;
const SESSION_FIELDS = {
  power: 'power_kw',
  energy: 'energy_kwh',
  soc: 'soc_pct',
  time: 'time_remaining_min',
} as const;
type Readout = Record<keyof typeof VEHICLE_FIELDS | keyof typeof SESSION_FIELDS, number | null>;

export function liveChargingModel(session: Session, status: VehicleStatus | null | undefined) {
  const result: Readout = {
    power: null,
    energy: null,
    soc: null,
    time: null,
    limit: null,
    range: null,
    restingSoc: null,
  };
  for (const key of Object.keys(VEHICLE_FIELDS) as Array<keyof typeof VEHICLE_FIELDS>)
    result[key] = status?.[VEHICLE_FIELDS[key]] ?? null;
  for (const key of Object.keys(SESSION_FIELDS) as Array<keyof typeof SESSION_FIELDS>) {
    const reading = session?.[SESSION_FIELDS[key]];
    if (reading != null) result[key] = reading;
  }
  return result;
}

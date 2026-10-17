import type { ChargeSessionRow } from './chargingTypes';
import { formatAppCalendarDate, formatAppDate, formatAppTime } from '../lib/dateTime';
const CHARGE_TYPES = new Map([['dc', 'dc'], ['dcfc', 'dc'], ['ac', 'ac'], ['ac_l2', 'ac']] as const);
const DC_NETWORKS = new Set(['tesla', 'rivian', 'electrify america', 'evgo']);
export function chargeType(row: ChargeSessionRow): 'ac' | 'dc' | null {
  const explicit = CHARGE_TYPES.get(row.charger_type?.toLowerCase() as 'dc' | 'dcfc' | 'ac' | 'ac_l2');
  if (explicit) return explicit;
  if (DC_NETWORKS.has((row.network_vendor ?? '').trim().toLowerCase())) return 'dc';
  const power = row.ended_at == null ? row.live_power_kw ?? row.peak_power_kw : row.peak_power_kw;
  return power != null && Number.isFinite(power) ? power < 20 ? 'ac' : 'dc' : null;
}
const SAMPLES = {
  energy: ['energy_added_kwh', 'live_total_charged_kwh', 1],
  soc: ['soc_end', 'live_soc_pct', 1],
  power: ['peak_power_kw', 'live_power_kw', 1],
  duration: ['duration_min', 'live_time_elapsed_seconds', 60],
} as const;
export function chargeSample(row: ChargeSessionRow, kind: keyof typeof SAMPLES) {
  const [historical, current, divisor] = SAMPLES[kind];
  const live = row.ended_at == null && row[current] != null;
  return { value: live ? row[current]! / divisor : row[historical], suffix: live ? ' · Live' : '' };
}
export function chargeTimes(row: ChargeSessionRow) {
  const start = new Date(row.started_at);
  const duration = chargeSample(row, 'duration').value;
  return {
    day: row.session_day_local ? formatAppCalendarDate(row.session_day_local) : formatAppDate(row.started_at),
    start: formatAppTime(start),
    end: duration == null ? null : formatAppTime(new Date(start.getTime() + duration * 60000)),
  };
}

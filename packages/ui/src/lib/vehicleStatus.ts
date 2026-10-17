import { statusLabel } from './statusLabel';
import type {
  VehicleStatus,
  VehicleStatusAvailabilityReason,
  VehicleStatusAvailabilityState,
} from '@panda/types';
import { formatAppDateTime } from './dateTime';

export type StatusTone = 'default' | 'success' | 'warning' | 'danger' | 'info';

export interface StatusAvailabilitySummary {
  availability: VehicleStatusAvailabilityState;
  reasonCode: VehicleStatusAvailabilityReason | null;
  lastSeenAt: string | null;
  latestEventAt: string | null;
  everSeen: boolean;
}

export interface PresentedVehicleStatusValue {
  label: string;
  variant: StatusTone;
  availability: StatusAvailabilitySummary;
  tooltip: string | null;
  lastUpdatedLabel: string | null;
  secondaryText: string | null;
  renderUnavailableChip: boolean;
}

const DEFINITION_FIELDS: Record<string, string[]> = {
  hv_thermal: ['hv_thermal_event'],
  twelve_volt_health: ['twelve_volt_health'],
  ota_current_version: ['ota_current_version'],
  ota_available_version: ['ota_available_version'],
  charge_port_open: ['charge_port_open'],
  charger_derate_active: ['charger_derate_active'],
  cabin_precon: ['cabin_precon_status', 'cabin_precon_type'],
  defrost_active: ['defrost_active'],
  pet_mode: ['pet_mode_active', 'pet_mode_temp_ok'],
  seat_fl_heat: ['seat_fl_heat'],
  seat_fr_heat: ['seat_fr_heat'],
  seat_rl_heat: ['seat_rl_heat'],
  seat_rr_heat: ['seat_rr_heat'],
  seat_fl_vent: ['seat_fl_vent'],
  seat_fr_vent: ['seat_fr_vent'],
  steering_wheel_heat: ['steering_wheel_heat'],
  tonneau_status: ['tonneau_closed', 'tonneau_locked'],
  gear_guard_locked: ['gear_guard_locked', 'gear_guard_video_status'],
  brake_fluid_warning: ['brake_fluid_low'],
  wiper_fluid_warning: ['wiper_fluid_low'],
  alarm_status: ['alarm_active'],
  service_mode: ['service_mode'],
  window_status: ['window_fl_closed', 'window_fr_closed', 'window_rl_closed', 'window_rr_closed'],
};

export function getStatusDefinitionFields(definitionId: string): string[] {
  return DEFINITION_FIELDS[definitionId] ?? [];
}

export function summarizeStatusAvailability(status: VehicleStatus | null | undefined, fields: string[]): StatusAvailabilitySummary {
  const summary: StatusAvailabilitySummary = { availability: 'never_seen', reasonCode: 'never_seen', lastSeenAt: null, latestEventAt: null, everSeen: false };
  let invalid = false;
  for (const field of fields) {
    const entry = status?.field_availability?.[field];
    if (!entry) continue;
    summary.latestEventAt ||= entry.latest_event_at || null;
    summary.everSeen ||= Boolean(entry.ever_seen);
    if (entry.last_seen_at && (!summary.lastSeenAt || new Date(entry.last_seen_at).getTime() > new Date(summary.lastSeenAt).getTime())) summary.lastSeenAt = entry.last_seen_at;
    if (entry.availability === 'current' || (entry.availability === 'historical' && summary.availability !== 'current')) summary.availability = entry.availability;
    invalid ||= entry.reason_code === 'invalid_sensor';
  }
  summary.reasonCode = invalid ? 'invalid_sensor' : summary.availability === 'historical' ? 'missing_recent_payload' : summary.availability === 'never_seen' ? 'never_seen' : null;
  return summary;
}

export function buildAvailabilityTooltip(label: string, availability: StatusAvailabilitySummary): string | null {
  const state = availability.reasonCode === 'invalid_sensor' ? 'invalid' : availability.availability;
  switch (state) {
    case 'invalid': return `${label} is currently unavailable because Rivian marked the sensor as invalid.`;
    case 'historical': return availability.lastSeenAt ? `${label} is showing the last known value from ${formatDateTimeShort(availability.lastSeenAt)} because the latest vehicle event did not include a new reading.` : null;
    case 'never_seen': return `${label} is unavailable because Panda has not stored a reading for this field yet. This may mean the vehicle has not emitted it or Rivian does not expose it for this model.`;
    default: return null;
  }
}
export function formatAvailabilityLastUpdated(availability: StatusAvailabilitySummary): string | null {
  return availability.availability === 'historical' && availability.lastSeenAt
    ? `Last updated ${formatDateTimeShort(availability.lastSeenAt)}` : null;
}

type Reading = { label: string; variant: StatusTone; secondaryText?: string | null };
type BoolRule = readonly [field: keyof VehicleStatus, active: string, inactive: string, activeTone: StatusTone, inactiveTone: StatusTone];
const BOOLEAN_READINGS: Record<string, BoolRule> = {
  charge_port_open: ['charge_port_open', 'Open', 'Closed', 'warning', 'success'],
  charger_derate_active: ['charger_derate_active', 'Active', 'Off', 'warning', 'success'],
  defrost_active: ['defrost_active', 'Active', 'Off', 'info', 'success'],
  pet_mode: ['pet_mode_active', 'Active', 'Off', 'info', 'success'],
  tonneau_status: ['tonneau_closed', 'Closed', 'Open', 'success', 'warning'],
  gear_guard_locked: ['gear_guard_locked', 'Locked', 'Unlocked', 'success', 'warning'],
  brake_fluid_warning: ['brake_fluid_low', 'Warning', 'OK', 'warning', 'success'],
  wiper_fluid_warning: ['wiper_fluid_low', 'Warning', 'OK', 'warning', 'success'],
  alarm_status: ['alarm_active', 'Triggered', 'Armed', 'danger', 'success'],
  service_mode: ['service_mode', 'In Service', 'OK', 'warning', 'success'],
};
const LEVEL_READINGS = new Set(['seat_fl_heat', 'seat_fr_heat', 'seat_rl_heat', 'seat_rr_heat', 'seat_fl_vent', 'seat_fr_vent', 'steering_wheel_heat']);
const TRUE_WORDS = new Set(['true', 'on', 'open', 'active', 'locked', 'triggered', 'yes']);
const FALSE_WORDS = new Set(['false', 'off', 'closed', 'inactive', 'disabled', 'unlocked', 'no']);
function boolValue(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value;
  if (typeof value !== 'string') return null;
  const word = value.toLowerCase();
  return TRUE_WORDS.has(word) ? true : FALSE_WORDS.has(word) ? false : null;
}
function readDefinition(id: string, status: VehicleStatus | null | undefined): Reading | null {
  const boolean = BOOLEAN_READINGS[id];
  if (boolean) {
    const [field, active, inactive, activeTone, inactiveTone] = boolean;
    const value = boolValue(status?.[field]);
    if (value === null) return null;
    const reading: Reading = { label: value ? active : inactive, variant: value ? activeTone : inactiveTone };
    if (id === 'pet_mode') {
      const ok = boolValue(status?.pet_mode_temp_ok);
      reading.secondaryText = ok === null ? null : `Temp ${ok ? 'OK' : 'Check'}`;
    }
    if (id === 'tonneau_status') reading.secondaryText = status?.tonneau_locked == null ? null : `Locked: ${status.tonneau_locked ? 'Yes' : 'No'}`;
    if (id === 'gear_guard_locked') reading.secondaryText = status?.gear_guard_video_status ? titleCase(status.gear_guard_video_status) : null;
    return reading;
  }
  const value = status?.[id as keyof VehicleStatus];
  if (LEVEL_READINGS.has(id)) {
    return typeof value === 'number' && Number.isFinite(value)
      ? { label: value <= 0 ? 'Off' : `Level ${value}`, variant: value <= 0 ? 'success' : 'info' } : null;
  }
  if (id === 'window_status') {
    const windows = DEFINITION_FIELDS.window_status!.reduce((counts, field) => {
      const closed = status?.[field as keyof VehicleStatus];
      if (typeof closed === 'boolean') { counts.known += 1; if (!closed) counts.open += 1; }
      return counts;
    }, { known: 0, open: 0 });
    if (!windows.known) return null;
    return windows.open ? { label: `${windows.open} open`, variant: 'warning' }
      : { label: windows.known === 4 ? 'Closed' : `${windows.known}/4 closed`, variant: windows.known === 4 ? 'success' : 'info' };
  }
  if (id === 'hv_thermal') {
    const thermal = status?.hv_thermal_event;
    if (!thermal) return null;
    const rules: Array<readonly [RegExp, string, StatusTone]> = [
      [/fault|fail|critical|error|overheat|warning/i, 'Warning', 'warning'],
      [/^(off|none|inactive|normal|ok|good)$/i, 'Nominal', 'success'],
    ];
    const match = rules.find(([pattern]) => pattern.test(thermal));
    return match ? { label: match[1], variant: match[2] } : { label: titleCase(thermal), variant: 'warning' };
  }
  if (id === 'twelve_volt_health') {
    const health = status?.twelve_volt_health;
    if (!health) return null;
    return { label: titleCase(health), variant: /normal|good|ok/i.test(health) ? 'success' : /critical|fault|fail/i.test(health) ? 'danger' : 'warning' };
  }
  if (id === 'ota_current_version' || id === 'ota_available_version') return value ? { label: value as string, variant: id === 'ota_current_version' ? 'default' : 'info' } : null;
  if (id === 'cabin_precon' && status?.cabin_precon_status) {
    return { label: titleCase(status.cabin_precon_status), variant: /off|none|inactive/i.test(status.cabin_precon_status) ? 'success' : 'info', secondaryText: status.cabin_precon_type ? titleCase(status.cabin_precon_type) : null };
  }
  return null;
}
export function presentVehicleStatusDefinition(definitionId: string, status: VehicleStatus | null | undefined): PresentedVehicleStatusValue {
  const availability = summarizeStatusAvailability(status, getStatusDefinitionFields(definitionId));
  const invalid = availability.reasonCode === 'invalid_sensor';
  const reading = invalid ? null : readDefinition(definitionId, status);
  return {
    label: reading?.label ?? (invalid ? 'Invalid Sensor' : 'Unavailable'),
    variant: reading?.variant ?? 'info', availability,
    tooltip: buildAvailabilityTooltip(titleCase(definitionId.replace(/_/g, ' ')), availability),
    lastUpdatedLabel: formatAvailabilityLastUpdated(availability),
    secondaryText: reading?.secondaryText ?? null,
    renderUnavailableChip: reading === null,
  };
}
function formatDateTimeShort(value: string) {
  return Number.isNaN(Date.parse(value)) ? 'an unknown time' : formatAppDateTime(value, { month: 'short', day: 'numeric' });
}
function titleCase(value: string) {
  return statusLabel(value, /[_-]+/);
}

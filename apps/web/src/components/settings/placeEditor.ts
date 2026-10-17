import type { Dispatch, SetStateAction } from 'react';
import type { Place, PlaceAddress, PlaceChargingInput, TouPeriod, UpsertPlaceBody } from '@panda/types';
import type { UnitSystem } from '@panda/ui/lib/utils';
import { getAppTimezone } from '@panda/ui/lib/dateTime';

export interface ScheduleDraft { label: string; start: string; end: string; rate: string }
export interface PlaceDraft {
  name: string; radius_m: string; placeType: 'home' | 'work' | 'poi'; chargingEnabled: boolean;
  planType: 'per_kwh' | 'tou'; energyRate: string; sessionFee: string; timezone: string; schedule: ScheduleDraft[];
}
export const METERS_TO_FEET = 3.28084;
export function emptyDraft(unit: UnitSystem): PlaceDraft {
  return { name: '', radius_m: unit === 'metric' ? '75' : '250', placeType: 'poi', chargingEnabled: false, planType: 'per_kwh', energyRate: '0.13', sessionFee: '0', timezone: getAppTimezone(), schedule: [{ label: 'All day', start: '00:00', end: '24:00', rate: '0.13' }] };
}
export function parseTimeToMinute(value: string, allow24Hour: boolean) {
  const parts = value.trim().split(':');
  if (parts.length !== 2 || !/^\d{1,2}$/.test(parts[0]!) || !/^\d{2}$/.test(parts[1]!)) return null;
  const hour = Number(parts[0]), minute = Number(parts[1]);
  if (hour === 24 && minute === 0 && allow24Hour) return 1440;
  return hour < 24 && minute < 60 ? hour * 60 + minute : null;
}
export function minutesToTime(minutes: number) {
  if (minutes >= 1440) return '24:00';
  return [Math.floor(minutes / 60), minutes % 60].map((part) => String(part).padStart(2, '0')).join(':');
}
export function normalizeScheduleEdges(schedule: ScheduleDraft[]): ScheduleDraft[] {
  const result = schedule.length ? schedule.map(({ label, start, end, rate }) => ({ label, start, end, rate })) : [{ label: 'All day', start: '00:00', end: '24:00', rate: '0' }];
  result[0]!.start = '00:00';
  result[result.length - 1]!.end = '24:00';
  return result;
}
export function buildNextSchedule(schedule: ScheduleDraft[]) {
  if (!schedule.length) return [{ label: 'Period 1', start: '00:00', end: '24:00', rate: '0' }];
  const last = schedule[schedule.length - 1];
  if (!last) return normalizeScheduleEdges([]);
  const start = parseTimeToMinute(last.start, false), end = parseTimeToMinute(last.end, true);
  const split = start !== null && end !== null && end - start >= 120
    ? Math.max(start + 60, Math.min(end - 60, Math.ceil((start + end) / 120) * 60)) : null;
  const prefix = split === null ? schedule : [...schedule.slice(0, -1), { ...last, end: minutesToTime(split) }];
  return normalizeScheduleEdges([...prefix, { label: `Period ${schedule.length + 1}`, start: split === null ? last.end === '24:00' ? '23:00' : last.end : minutesToTime(split), end: split === null ? '24:00' : minutesToTime(end!), rate: last.rate || '0' }]);
}
export function scheduleDraftToPeriods(schedule: ScheduleDraft[]): TouPeriod[] | null {
  const decoded = schedule.map((period) => ({ label: period.label.trim(), start_minute: parseTimeToMinute(period.start, false), end_minute: parseTimeToMinute(period.end, true), rate: Number(period.rate) }));
  return decoded.some((period) => period.start_minute === null || period.end_minute === null || !Number.isFinite(period.rate)) ? null : decoded as TouPeriod[];
}
export function validateScheduleDraft(schedule: ScheduleDraft[]) {
  const periods = scheduleDraftToPeriods(schedule);
  if (!periods?.length) return 'Each TOU period needs a valid start, end, and rate.';
  const errors = periods.map((period, index) => {
    if (!period.label) return 'Each TOU period needs a label.';
    if (period.start_minute !== (periods[index - 1]?.end_minute ?? 0)) return 'TOU periods must be contiguous and begin at 00:00.';
    if (period.end_minute <= period.start_minute) return 'Each TOU period must end after it starts.';
    return period.rate < 0 ? 'Each TOU period needs a non-negative rate.' : null;
  });
  return errors.find(Boolean) ?? (periods[periods.length - 1]!.end_minute === 1440 ? null : 'TOU periods must cover the entire day through midnight.');
}
export function buildChargingPayload(draft: PlaceDraft): PlaceChargingInput | null {
  if (!draft.chargingEnabled) return null;
  const shared = { rate: Number(draft.energyRate || '0'), session_fee: Number(draft.sessionFee || '0'), currency: 'USD' };
  if (draft.planType === 'per_kwh') return { ...shared, billing_type: 'per_kwh' };
  const periods = scheduleDraftToPeriods(draft.schedule);
  return periods ? { ...shared, billing_type: 'tou', timezone: draft.timezone.trim() || getAppTimezone(), tou_periods: periods } : null;
}

export interface PlaceEditorState { draft: PlaceDraft; editingPlaceId: string | null; selectedAddress: PlaceAddress | null; addressQuery: string; submittedAddressQuery: string }
type PlaceEditorEvent =
  | { type: 'reset'; unit: UnitSystem }
  | { type: 'edit'; place: Place; unit: UnitSystem }
  | { type: 'address'; address: PlaceAddress }
  | { type: 'clearAddress' }
  | { type: 'period'; index: number; updates: Partial<ScheduleDraft>; syncNextStart: boolean }
  | { type: 'draft'; update: SetStateAction<PlaceDraft> }
  | { type: 'query'; value: string; submitted?: boolean };
export type PlaceDraftSetter = Dispatch<SetStateAction<PlaceDraft>>;
export function placeEditorInitial(unit: UnitSystem): PlaceEditorState {
  return { draft: emptyDraft(unit), editingPlaceId: null, selectedAddress: null, addressQuery: '', submittedAddressQuery: '' };
}
export function placeEditorTransition(state: PlaceEditorState, event: PlaceEditorEvent): PlaceEditorState {
  switch (event.type) {
    case 'reset': return placeEditorInitial(event.unit);
    case 'period': {
      const schedule = state.draft.schedule.slice();
      if (schedule[event.index]) schedule[event.index] = { ...schedule[event.index]!, ...event.updates };
      if (event.syncNextStart && event.updates.end && schedule[event.index + 1]) schedule[event.index + 1] = { ...schedule[event.index + 1]!, start: event.updates.end };
      return { ...state, draft: { ...state.draft, schedule: normalizeScheduleEdges(schedule) } };
    }
    case 'draft': return { ...state, draft: typeof event.update === 'function' ? event.update(state.draft) : event.update };
    case 'query': return { ...state, [event.submitted ? 'submittedAddressQuery' : 'addressQuery']: event.value };
    case 'clearAddress': return { ...state, selectedAddress: null };
    case 'address': return { ...state, selectedAddress: event.address, addressQuery: event.address.display_name, submittedAddressQuery: '' };
    case 'edit': {
      const { place, unit } = event;
      const charging = place.charging;
      const schedule = charging?.billing_type === 'tou' && charging.tou_periods.length ? charging.tou_periods.map((period) => ({ label: period.label, start: minutesToTime(period.start_minute), end: minutesToTime(period.end_minute), rate: String(period.rate) })) : [{ label: 'All day', start: '00:00', end: '24:00', rate: String(charging?.rate ?? 0.13) }];
      return { editingPlaceId: place.id, selectedAddress: place.address, addressQuery: place.address?.display_name ?? '', submittedAddressQuery: '', draft: { name: place.name, radius_m: String(Math.round(place.radius_m * (unit === 'metric' ? 1 : METERS_TO_FEET))), placeType: place.is_home ? 'home' : place.is_work ? 'work' : 'poi', chargingEnabled: !!charging, planType: charging?.billing_type === 'tou' ? 'tou' : 'per_kwh', energyRate: String(charging?.rate ?? 0.13), sessionFee: String(charging?.session_fee ?? 0), timezone: charging?.timezone ?? getAppTimezone(), schedule: normalizeScheduleEdges(schedule) } };
    }
  }
}

export function placeRequest(state: PlaceEditorState, unit: UnitSystem): UpsertPlaceBody | null {
  if (!state.selectedAddress) return null;
  const { draft } = state;
  const charging = buildChargingPayload(draft);
  if (draft.chargingEnabled && draft.planType === 'tou' && !charging) return null;
  const radius = Number(draft.radius_m || '0');
  return { name: draft.name.trim(), radius_m: unit === 'metric' ? radius : radius / METERS_TO_FEET, is_home: draft.placeType === 'home', is_work: draft.placeType === 'work', address: state.selectedAddress, charging };
}

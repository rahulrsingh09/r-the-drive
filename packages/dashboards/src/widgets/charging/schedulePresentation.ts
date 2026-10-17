import type { DepartureSchedule } from '@panda/hooks';
import { scheduleClock } from './scheduleState';

type ChargingSchedule = {
  enabled: boolean;
  start_time_minutes: number | null;
  duration_minutes: number | null;
  amperage: number | null;
  week_days: string[] | null;
};

export function chargingScheduleDraft(schedule: ChargingSchedule) {
  const numbers = {
    durationHours: schedule.duration_minutes,
    amperage: schedule.amperage,
  };
  const fields = { durationHours: '', amperage: '' };
  for (const key of ['durationHours', 'amperage'] as const) {
    const value = numbers[key];
    if (value === null) continue;
    fields[key] = String(key === 'durationHours' ? Math.round(value / 60 * 10) / 10 : value);
  }
  return {
    ...fields,
    enabled: schedule.enabled,
    startTime: scheduleClock(schedule.start_time_minutes),
    weekDays: schedule.week_days ?? [],
  };
}

export function departurePresentation(schedule: DepartureSchedule) {
  const occurrence = schedule.occurrence;
  const days = occurrence?.days ?? [];
  const time = scheduleClock(occurrence?.time_minutes);
  const dayLabel = days.length === 7 ? 'Every day' : days.map((day) => day.slice(0, 3)).join(', ');
  const occurrenceLabel = days.length ? `${dayLabel} at ${time}` : time || 'Not configured';
  const comfort = schedule.comfort_settings;
  const pills: Array<{ kind: 'temperature' | 'wind'; label: string }> = [];
  if (comfort) {
    for (const [field, label] of [['seat_fl_heat', 'FL'], ['seat_fr_heat', 'FR']] as const) {
      const heat = comfort[field];
      if (heat !== undefined && heat > 0) {
        pills.push({ kind: 'temperature', label: `${label} Seat: ${['Off', 'Low', 'Med', 'High'][heat]}` });
      }
    }
    if (comfort.cabin_temp_c !== undefined) pills.push({ kind: 'temperature', label: `Cabin: ${comfort.cabin_temp_c}°C` });
    if (comfort.defrost) pills.push({ kind: 'wind', label: 'Defrost' });
  }
  return {
    occurrence: occurrenceLabel,
    pills,
    hasComfort: comfort && (comfort.seat_fl_heat || comfort.seat_fr_heat || comfort.cabin_temp_c !== undefined || comfort.defrost),
    selectedDays: new Set(days),
  };
}

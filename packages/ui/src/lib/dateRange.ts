import type { BoundedPresetKey, DashboardTimeframe, DateRange } from '@panda/types';
import { subHours, startOfMonth, endOfMonth } from 'date-fns';
import { startOfAppDay, endOfAppDay, shiftAppCalendarDays, appDatePartsToDate, getAppDateParts } from './dateTime';
const PRESET_SPANS: Record<BoundedPresetKey, readonly [unit: 'hours' | 'days', amount: number]> = {
  '1h': ['hours', 1], '6h': ['hours', 6], '12h': ['hours', 12], '24h': ['hours', 24],
  '7d': ['days', 7], '30d': ['days', 30], '90d': ['days', 90], '1y': ['days', 365],
};
export function presetToRange(preset: BoundedPresetKey): DateRange {
  const now = new Date();
  const [unit, span] = PRESET_SPANS[preset];
  return unit === 'hours' ? { from: subHours(now, span), to: now }
    : { from: startOfAppDay(shiftAppCalendarDays(now, -span)), to: endOfAppDay(now) };
}
export function timeframeRange(timeframe: DashboardTimeframe): DateRange | null {
  if (timeframe.kind === 'lifetime') return null;
  if (timeframe.kind === 'preset') return presetToRange(timeframe.preset);
  const { from, to } = timeframe;
  return from <= to ? { from, to } : { from: to, to: from };
}
export function calendarCells(month: Date, selected: Date, from: Date, to: Date) {
  const first = startOfMonth(month);
  const offset = first.getDay();
  const selectedParts = getAppDateParts(selected);
  const low = startOfAppDay(from), high = startOfAppDay(to);
  return Array.from({ length: offset + endOfMonth(month).getDate() }, (_, index) => {
    if (index < offset) return null;
    const date = new Date(first.getFullYear(), first.getMonth(), index - offset + 1);
    const instant = appDatePartsToDate({ year: date.getFullYear(), month: date.getMonth() + 1, day: date.getDate(), hour: 0, minute: 0, second: 0 });
    const parts = getAppDateParts(instant);
    const selectedDay = parts?.year === selectedParts?.year && parts?.month === selectedParts?.month && parts?.day === selectedParts?.day;
    return { date, selectedDay, inRange: instant >= low && instant <= high };
  });
}

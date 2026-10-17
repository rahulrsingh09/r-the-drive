import { formatISO, isValid, parse, subHours } from 'date-fns';
import type { BoundedPresetKey, DashboardTimeframe, DateRange, PresetKey } from '@panda/types';
import {
  appDatePartsToDate,
  endOfAppDay,
  formatAppDateTime,
  shiftAppCalendarDays,
  startOfAppDay,
} from '@panda/ui/lib/dateTime';

export type { BoundedPresetKey, DashboardTimeframe, DateRange, PresetKey } from '@panda/types';

export const BOUNDED_PRESET_KEYS: readonly BoundedPresetKey[] = ['1h', '6h', '12h', '24h', '7d', '30d', '90d', '1y'];
export const PRESET_KEYS: readonly PresetKey[] = [...BOUNDED_PRESET_KEYS, 'lifetime'];
export const DEFAULT_PRESET: BoundedPresetKey = '30d';
export const DEFAULT_TIMEFRAME: DashboardTimeframe = { kind: 'preset', preset: DEFAULT_PRESET };

const DASHBOARD_TIMEFRAME_STORAGE_KEY = 'rm-dashboard-timeframe';

type StoredDashboardTimeframe =
  | { kind: 'preset'; preset: string }
  | { kind: 'custom'; from: string; to: string }
  | { kind: 'lifetime' }
  | { preset?: string; from?: string; to?: string };

const PRESET_OFFSETS: Record<BoundedPresetKey, { hours: number } | { days: number }> = {
  '1h': { hours: 1 }, '6h': { hours: 6 }, '12h': { hours: 12 }, '24h': { hours: 24 },
  '7d': { days: 7 }, '30d': { days: 30 }, '90d': { days: 90 }, '1y': { days: 365 },
};

export function presetToRange(preset: BoundedPresetKey): DateRange {
  const offset = PRESET_OFFSETS[preset];
  const anchor = new Date();
  if ('hours' in offset) return { from: subHours(anchor, offset.hours), to: anchor };
  const firstDay = shiftAppCalendarDays(anchor, -offset.days);
  return { from: startOfAppDay(firstDay), to: endOfAppDay(anchor) };
}

export function getTimeframeRange(timeframe: DashboardTimeframe): DateRange | null {
  const bounds = timeframeBounds(timeframe);
  return bounds === null ? null : normalizeDateRange(bounds);
}

function timeframeBounds(timeframe: DashboardTimeframe): DateRange | null {
  if ('preset' in timeframe) return presetToRange(timeframe.preset);
  if ('from' in timeframe) return { from: timeframe.from, to: timeframe.to };
  return null;
}

export function rangeToIso(range: DateRange): { from: string; to: string } {
  return {
    from: formatISO(range.from),
    to: formatISO(range.to),
  };
}

export function timeframeToQuery(timeframe: DashboardTimeframe): {
  from: string | null;
  to: string | null;
  lifetime: boolean;
  cacheKey: string;
} {
  const range = getTimeframeRange(timeframe);
  const result = { from: null, to: null, lifetime: true, cacheKey: 'lifetime' } as {
    from: string | null; to: string | null; lifetime: boolean; cacheKey: string;
  };
  if (range) {
    Object.assign(result, rangeToIso(range), { lifetime: false });
    result.cacheKey = timeframe.kind === 'preset'
      ? `preset:${timeframe.preset}`
      : `custom:${result.from}:${result.to}`;
  }
  return result;
}

export function getTimeframeLabel(timeframe: DashboardTimeframe): string {
  const label = timeframe.kind === 'custom'
    ? [timeframe.from, timeframe.to].map(formatRangeDisplay)
    : [PRESET_LABELS[timeframe.kind === 'preset' ? timeframe.preset : 'lifetime']];
  return label.join(' - ');
}

export function normalizeDateRange(range: DateRange): DateRange {
  return range.from <= range.to ? range : { from: range.to, to: range.from };
}

function decodeStoredTimeframe(value: unknown): DashboardTimeframe | null {
  if (!value || typeof value !== 'object') return null;
  const fields = value as Record<string, unknown>;
  const bounded = typeof fields.preset === 'string'
    && BOUNDED_PRESET_KEYS.includes(fields.preset as BoundedPresetKey)
    ? { kind: 'preset' as const, preset: fields.preset as BoundedPresetKey }
    : null;
  const hasDates = typeof fields.from === 'string' && typeof fields.to === 'string';
  const custom = () => {
    if (!hasDates) return null;
    const range = parseStoredCustomRange(fields.from as string, fields.to as string);
    return range ? { kind: 'custom' as const, ...range } : null;
  };
  switch (fields.kind) {
    case 'lifetime': return { kind: 'lifetime' };
    case 'preset': if (bounded) return bounded; break;
    case 'custom': if (hasDates) return custom(); break;
  }
  if (fields.preset === 'lifetime') return { kind: 'lifetime' };
  return bounded ?? custom();
}

export function loadDashboardTimeframe(): DashboardTimeframe | null {
  try {
    const raw = typeof window === 'undefined' ? null : window.sessionStorage.getItem(DASHBOARD_TIMEFRAME_STORAGE_KEY);
    return raw ? decodeStoredTimeframe(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

export function saveDashboardTimeframe(timeframe: DashboardTimeframe) {
  try {
    const storage = typeof window === 'undefined' ? null : window.sessionStorage;
    storage?.setItem(DASHBOARD_TIMEFRAME_STORAGE_KEY, JSON.stringify(serializeDashboardTimeframe(timeframe)));
  } catch {
    // Storage may be unavailable or full; the picker still works in-memory.
  }
}

export function serializeDashboardTimeframe(timeframe: DashboardTimeframe): StoredDashboardTimeframe {
  const record: StoredDashboardTimeframe = { kind: timeframe.kind } as StoredDashboardTimeframe;
  if (timeframe.kind === 'preset') Object.assign(record, { preset: timeframe.preset });
  if (timeframe.kind === 'custom') Object.assign(record, rangeToIso(getTimeframeRange(timeframe)!));
  return record;
}

function parseNumericTimeframe(value: string, fallback: Date): Date | null {
  const shape = /^\d{1,2}\/\d{1,2}\/(\d{1,4})(?: (.+))?$/.exec(value);
  if (!shape) return null;
  const year = shape[1]!.length <= 2 ? 'yy' : 'yyyy';
  const clock = shape[2];
  const clocks = !clock ? [''] : /[a-z]/i.test(clock) ? [' h:mm a', ' h:mma'] : [' HH:mm'];
  for (const suffix of clocks) {
    const candidate = parse(value, `M/d/${year}${suffix}`, fallback);
    if (isValid(candidate)) return candidate;
  }
  return null;
}

export function parseTimeframeInput(value: string, fallback: Date): Date | null {
  const text = value.trim();
  if (text === '') return null;
  const wallDate = parseNumericTimeframe(text, fallback);
  if (wallDate === null) {
    const native = new Date(text);
    return isValid(native) ? native : null;
  }
  return appDatePartsToDate({
    year: wallDate.getFullYear(), month: wallDate.getMonth() + 1, day: wallDate.getDate(),
    hour: wallDate.getHours(), minute: wallDate.getMinutes(), second: 0,
  });
}

export const PRESET_LABELS: Record<PresetKey, string> = {
  '1h': 'Last 1h',
  '6h': 'Last 6h',
  '12h': 'Last 12h',
  '24h': 'Last 24h',
  '7d': 'Last 7 days',
  '30d': 'Last 30 days',
  '90d': 'Last 90 days',
  '1y': 'Last year',
  lifetime: 'Lifetime',
};

function parseStoredCustomRange(fromRaw: string, toRaw: string): DateRange | null {
  const timestamps = [Date.parse(fromRaw), Date.parse(toRaw)];
  if (!timestamps.every(Number.isFinite)) return null;
  const [start, end] = timestamps.sort((left, right) => left - right);
  return { from: new Date(start!), to: new Date(end!) };
}

function formatRangeDisplay(value: Date) {
  return formatAppDateTime(value);
}

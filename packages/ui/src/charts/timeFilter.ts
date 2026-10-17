export const TIME_FILTER_OPTIONS = [
  { value: 'raw', label: 'Raw', milliseconds: 0 },
  { value: '15m', label: '15 min', milliseconds: 15 * 60 * 1000 },
  { value: '1h', label: '1 hr', milliseconds: 60 * 60 * 1000 },
  { value: '6h', label: '6 hr', milliseconds: 6 * 60 * 60 * 1000 },
  { value: '24h', label: '24 hr', milliseconds: 24 * 60 * 60 * 1000 },
  { value: '3d', label: '3 days', milliseconds: 3 * 24 * 60 * 60 * 1000 },
  { value: '7d', label: '7 days', milliseconds: 7 * 24 * 60 * 60 * 1000 },
] as const;

export type TimeFilterWindow = (typeof TIME_FILTER_OPTIONS)[number]['value'];

export const DEFAULT_SPRITE_TIME_FILTER: TimeFilterWindow = '24h';
export const DEFAULT_CHART_TIME_FILTER: TimeFilterWindow = '15m';

const FILTERS = new Map<string, (typeof TIME_FILTER_OPTIONS)[number]>(
  TIME_FILTER_OPTIONS.map(option => [option.value, option]),
);

export function isTimeFilterWindow(value: unknown): value is TimeFilterWindow {
  return typeof value === 'string' && FILTERS.has(value);
}

export function normalizeTimeFilter(value: unknown, fallback: TimeFilterWindow): TimeFilterWindow {
  return isTimeFilterWindow(value) ? value : fallback;
}

export function timeFilterMilliseconds(window: TimeFilterWindow): number {
  return FILTERS.get(window)?.milliseconds ?? 0;
}


/**
 * Largest interval a compact sparkline will render as one visual observation.
 * Rich dashboard history charts do not use this guard: their valid samples
 * remain connected across elapsed time.
 */
export function maximumContinuousTimeGapMilliseconds(window: TimeFilterWindow): number {
  return Math.max(2 * 60 * 60 * 1000, timeFilterMilliseconds(window) * 4);
}

export function timeFilterLabel(window: TimeFilterWindow): string {
  return FILTERS.get(window)?.label ?? 'Raw';
}

function toMilliseconds(value: string | number | Date): number | null {
  const time = typeof value === 'string' ? Date.parse(value) : Number(value);
  return Number.isFinite(time) ? time : null;
}

function validReading(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}


/**
 * Filters contiguous finite samples with a centered, timestamp-aware moving
 * average. It keeps the original timestamps and null gaps intact.
 */
export function filterTimeSeriesValues(
  timestamps: Array<string | number | Date>,
  values: Array<number | null | undefined>,
  window: TimeFilterWindow,
): Array<number | null> {
  const result = values.map(value => validReading(value) ? value : null);
  const radius = timeFilterMilliseconds(window) / 2;
  if (!radius || timestamps.length !== values.length) return result;
  const times = timestamps.map(toMilliseconds);
  const runs: Array<[number, number]> = [];
  let start: number | null = null;
  for (let index = 0; index <= values.length; index += 1) {
    const valid = index < values.length && result[index] != null && times[index] != null;
    const decreases = start != null && valid && times[index]! < times[index - 1]!;
    if (start != null && (!valid || decreases)) { runs.push([start, index]); start = null; }
    if (valid && start == null) start = index;
  }
  for (const [first, end] of runs) {
    let entered = first;
    let departed = first;
    let total = 0;
    for (let index = first; index < end; index += 1) {
      const lower = times[index]! - radius;
      const upper = times[index]! + radius;
      while (entered < end && times[entered]! <= upper) total += result[entered++]!;
      // Departures use the emitted reading, preserving the established filter recurrence.
      while (departed < entered && times[departed]! < lower) total -= result[departed++]!;
      if (entered > departed) result[index] = total / (entered - departed);
    }
  }
  return result;
}


export interface TimeBucketPoint {
  timestamp: string | number | Date;
  value: number;
}

/**
 * Bins finite samples into fixed, non-overlapping time windows and sums their
 * values. This is for compact bar displays, where retaining a bar per source
 * row obscures the magnitude of activity in a period. `Raw` is a strict
 * bypass that keeps every original finite sample.
 */
export function bucketTimeSeriesValues(
  timestamps: Array<string | number | Date>,
  values: Array<number | null | undefined>,
  window: TimeFilterWindow,
): TimeBucketPoint[] {
  const samples: TimeBucketPoint[] = [];
  const width = timeFilterMilliseconds(window);
  const buckets = new Map<number, number>();
  let dated = true;
  for (let index = 0; index < timestamps.length; index += 1) {
    if (!(index in timestamps)) continue;
    const value = values[index];
    if (!validReading(value)) continue;
    const timestamp = timestamps[index]!;
    samples.push({ timestamp, value });
    if (!width) continue;
    const time = toMilliseconds(timestamp);
    if (time == null) { dated = false; continue; }
    const start = Math.floor(time / width) * width;
    buckets.set(start, (buckets.get(start) ?? 0) + value);
  }
  if (!width || timestamps.length !== values.length || !dated) return samples;
  return Array.from(buckets.keys()).sort((a, b) => a - b).map(start => ({
    timestamp: new Date(start).toISOString(), value: buckets.get(start)!,
  }));
}

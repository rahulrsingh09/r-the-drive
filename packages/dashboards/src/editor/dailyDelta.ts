export interface SeriesPoint {
  ts?: string;
  value: number | null | undefined;
}

export interface DailyDeltaPoint {
  ts: string;
  value: number;
}

type DayBucket = { total: number; last: DailyDeltaPoint };
const DAY_MS = 86_400_000;

function collectDays(series: SeriesPoint[]): Map<string, DayBucket> {
  const days = new Map<string, DayBucket>();
  for (const point of series) {
    if (typeof point.value !== 'number' || !Number.isFinite(point.value) ||
        typeof point.ts !== 'string' || !point.ts) continue;
    const date = new Date(point.ts);
    if (Number.isNaN(date.getTime())) continue;
    const key = dayKey(date);
    const current = days.get(key);
    if (!current) {
      days.set(key, { total: point.value, last: { ts: point.ts, value: point.value } });
      continue;
    }
    current.total += point.value;
    // Saved series select by timestamp text, including the last equal timestamp.
    if (point.ts >= current.last.ts) current.last = { ts: point.ts, value: point.value };
  }
  return days;
}

export function seriesToDailyTotals(series: SeriesPoint[], windowDays: number): DailyDeltaPoint[] {
  const days = collectDays(series);
  const points = Array.from(days.keys()).sort().map(key => ({
    ts: `${key}T00:00:00.000Z`, value: days.get(key)!.total,
  }));
  return padDailyWindow(points, windowDays);
}

export function seriesToDailyDeltas(series: SeriesPoint[], windowDays: number): DailyDeltaPoint[] {
  const days = collectDays(series);
  const keys = Array.from(days.keys()).sort();
  const points = keys.map((key, index) => {
    const last = days.get(key)!.last;
    const previous = index ? days.get(keys[index - 1]!)!.last.value : last.value;
    const difference = last.value - previous;
    return { ts: last.ts, value: difference > 0 ? difference : 0 };
  });
  return padDailyWindow(points, windowDays);
}

function dayKey(date: Date) {
  const y = date.getUTCFullYear();
  const m = String(date.getUTCMonth() + 1).padStart(2, '0');
  const d = String(date.getUTCDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

function padDailyWindow(points: DailyDeltaPoint[], windowDays: number): DailyDeltaPoint[] {
  if (windowDays <= 0 || !points.length) return points;
  const last = new Date(points[points.length - 1]!.ts);
  const cutoff = new Date(Date.UTC(last.getUTCFullYear(), last.getUTCMonth(), last.getUTCDate()) - (windowDays - 1) * DAY_MS).getTime();
  const lookup = new Map<string, DailyDeltaPoint>();
  for (const point of points) {
    const date = new Date(point.ts);
    if (Number.isFinite(date.getTime()) && date.getTime() >= cutoff) lookup.set(dayKey(date), point);
  }
  const result: DailyDeltaPoint[] = [];
  for (let offset = 0; offset < windowDays; offset++) {
    const day = new Date(cutoff + offset * DAY_MS);
    const present = lookup.get(dayKey(day));
    result.push(present ?? { ts: day.toISOString(), value: 0 });
  }
  return result;
}

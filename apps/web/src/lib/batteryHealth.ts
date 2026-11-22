/**
 * One battery-health rule for Car and Insights:
 * - each calendar day (app timezone) has one usable capacity, the median of its readings;
 * - health = the latest day ÷ the first day, in %, at most 100;
 * - "usable capacity now" (also the divisor for full charge cycles) = the latest day.
 */

export interface CapacityReading { ts: string; usable_kwh: number }
export interface CapacityDay { day: string; kwh: number }

const formatters = new Map<string, Intl.DateTimeFormat>();
function dayOf(ts: string, timezone: string) {
  let formatter = formatters.get(timezone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat('en-CA', { timeZone: timezone, year: 'numeric', month: '2-digit', day: '2-digit' });
    formatters.set(timezone, formatter);
  }
  const date = new Date(ts);
  return Number.isNaN(date.getTime()) ? '' : formatter.format(date);
}

/** The median usable capacity of each day, oldest first. Readings of 0 or less are left out. */
export function capacityByDay(rows: ReadonlyArray<CapacityReading> | null | undefined, timezone: string): CapacityDay[] {
  const byDay = new Map<string, number[]>();
  // A server that answers with an object instead of a list gives no reading; pages must not fail on it.
  for (const row of Array.isArray(rows) ? rows : []) {
    if (!Number.isFinite(row.usable_kwh) || row.usable_kwh <= 0) continue;
    const day = dayOf(row.ts, timezone);
    if (day) byDay.set(day, [...(byDay.get(day) ?? []), row.usable_kwh]);
  }
  return [...byDay.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([day, values]) => {
    const sorted = values.slice().sort((a, b) => a - b);
    return { day, kwh: sorted[Math.floor(sorted.length / 2)]! };
  });
}

/** The usable capacity of the latest day; null without readings. */
export function usableCapacityNow(days: readonly CapacityDay[]): number | null {
  return days[days.length - 1]?.kwh ?? null;
}

/** Battery health in %: the latest day ÷ the first day, at most 100. Null without readings. */
export function batteryHealthPct(days: readonly CapacityDay[]): number | null {
  const first = days[0]?.kwh;
  const now = usableCapacityNow(days);
  return first && now != null ? Math.min(100, (now / first) * 100) : null;
}

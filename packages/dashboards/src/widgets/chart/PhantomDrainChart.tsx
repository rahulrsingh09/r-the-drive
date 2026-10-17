import * as React from 'react';
import type { PhantomDrainPeriod } from '@panda/types';
import { CHART_COLORS, RichTimeSeriesChart } from '@panda/ui/charts';
import { formatPercent } from '@panda/ui/lib/utils';

const SESSION_COLORS = [
  CHART_COLORS.accent,
  CHART_COLORS.emerald,
  CHART_COLORS.sky,
  CHART_COLORS.violet,
  CHART_COLORS.rose,
  CHART_COLORS.teal,
  CHART_COLORS.amber,
  CHART_COLORS.indigo,
] as const;

interface PhantomDrainDailySession {
  socLost: number;
  parkedHours: number;
  drainRate: number;
  startedAt: string;
}

export interface PhantomDrainDailyPoint {
  ts: string;
  drainRate: number;
  socLost: number;
  parkedHours: number;
  periodCount: number;
  sessions: PhantomDrainDailySession[];
}

export interface PhantomDrainChartProps {
  periods: PhantomDrainPeriod[];
  loading?: boolean;
  height?: number;
  yUnit?: string | undefined;
  emptyTitle?: string | undefined;
  yRange?: [number, number] | undefined;
  interactionMode?: 'standard' | 'touch-explore' | undefined;
  /** Optional definition-driven color. Omitted preserves the established session palette. */
  seriesColor?: string | undefined;
}

function isFiniteNumber(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function buildPhantomDrainDailySeries(
  periods: PhantomDrainPeriod[]
): PhantomDrainDailyPoint[] {
  const intervals = periods.flatMap((period, order) => {
    const start = period.period_start ? Date.parse(period.period_start) : NaN;
    const end = period.period_end ? Date.parse(period.period_end) : NaN;
    if (period.validation_status !== 'validated' || !Number.isFinite(start) || !Number.isFinite(end) || !isFiniteNumber(period.soc_lost_pct) || end <= start) return [];
    return [{ start, end, loss: period.soc_lost_pct, startedAt: period.period_start!, order }];
  });
  const days = new Map<number, number>();
  for (const interval of intervals) {
    const date = new Date(interval.start);
    let day = new Date(date.getFullYear(), date.getMonth(), date.getDate());
    while (day.getTime() < interval.end) {
      const next = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 1);
      days.set(day.getTime(), next.getTime()); day = next;
    }
  }
  const byStart = intervals.slice().sort((a, b) => a.start - b.start);
  const active = new Set<(typeof intervals)[number]>();
  let next = 0;
  const result: PhantomDrainDailyPoint[] = [];
  for (const [start, end] of [...days].sort((a, b) => a[0] - b[0])) {
    while (next < byStart.length && byStart[next]!.start < end) active.add(byStart[next++]!);
    active.forEach((interval) => { if (interval.end <= start) active.delete(interval); });
    const point: PhantomDrainDailyPoint = { ts: new Date(start).toISOString(), drainRate: 0, socLost: 0, parkedHours: 0, periodCount: 0, sessions: [] };
    for (const interval of [...active].sort((a, b) => a.order - b.order)) {
      const duration = Math.min(end, interval.end) - Math.max(start, interval.start);
      if (duration <= 0) continue;
      const parkedHours = duration / 3600000, socLost = interval.loss * (duration / (interval.end - interval.start));
      point.socLost += socLost; point.parkedHours += parkedHours; point.periodCount++;
      point.sessions.push({ socLost, parkedHours, drainRate: parkedHours > 0 ? socLost / parkedHours : 0, startedAt: interval.startedAt });
    }
    point.drainRate = point.parkedHours > 0 ? point.socLost / point.parkedHours : 0;
    point.sessions.sort((a, b) => a.startedAt.localeCompare(b.startedAt));
    if (Number.isFinite(point.drainRate)) result.push(point);
  }
  return result.sort((a, b) => a.ts.localeCompare(b.ts));
}

function withDailyHeadroom(maximum: number) {
  if (!Number.isFinite(maximum) || maximum <= 0) return 1;
  const padded = maximum * 1.25;
  const magnitude = 10 ** Math.floor(Math.log10(padded));
  return Math.ceil(padded / magnitude) * magnitude;
}
export function PhantomDrainChart({
  periods,
  loading = false,
  height = 280,
  yUnit = '% SoC lost',
  emptyTitle = 'No phantom drain data for this period',
  yRange,
  interactionMode = 'standard',
  seriesColor,
}: PhantomDrainChartProps) {
  const data = React.useMemo(() => buildPhantomDrainDailySeries(periods), [periods]);
  const { dailyXRange, dailyYRange, sessionCount } = React.useMemo(() => {
    let maximum = 0, sessionCount = 0;
    for (const point of data) { maximum = Math.max(maximum, point.socLost); sessionCount = Math.max(sessionCount, point.sessions.length); }
    return {
      dailyXRange: data.length ? [Date.parse(data[0]!.ts) / 1000 - 43200, Date.parse(data.at(-1)!.ts) / 1000 + 43200] as [number, number] : undefined,
      dailyYRange: yRange ?? (maximum > 0 ? [0, withDailyHeadroom(maximum)] as [number, number] : undefined),
      sessionCount,
    };
  }, [data, yRange]);
  const sessionSeries = Array.from({ length: sessionCount }, (_, sessionIndex) => ({
    key: `phantom-drain-session-${sessionIndex}`,
    label: `Drain session ${sessionIndex + 1}`,
    color: seriesColor ?? SESSION_COLORS[sessionIndex % SESSION_COLORS.length]!,
    values: data.map((point) => point.sessions[sessionIndex]?.socLost ?? null),
    mode: 'bar' as const,
    stackId: 'phantom-drain-day',
    showInLegend: false,
    tooltipFormatter: (value: number | null | undefined) =>
      value == null ? '-' : formatPercent(value, 2),
  }));
  return (
    <RichTimeSeriesChart
      rendererId="phantom-drain"
      points={data.map((point) => ({ ts: point.ts }))}
      series={[
        ...sessionSeries,
        {
          key: 'phantom-drain-rate',
          label: 'Daily average rate',
          values: data.map((point) => point.drainRate),
          tooltipOnly: true,
          tooltipFormatter: (value) => (value == null ? '-' : `${formatPercent(value, 2)} / h`),
        },
        {
          key: 'phantom-drain-hours',
          label: 'Parked',
          values: data.map((point) => point.parkedHours),
          tooltipOnly: true,
          tooltipFormatter: (value) => (value == null ? '-' : `${value.toFixed(1)} h`),
        },
        {
          key: 'phantom-drain-periods',
          label: 'Drain sessions',
          values: data.map((point) => point.periodCount),
          tooltipOnly: true,
          tooltipFormatter: (value) => (value == null ? '-' : String(Math.round(value))),
        },
      ]}
      loading={loading}
      emptyTitle={emptyTitle}
      height={height}
      yUnit={yUnit}
      yAxisValueFormatter={yUnit === '% SoC lost' ? (value) => value == null ? '' : formatPercent(value, 2) : undefined}
      xRange={dailyXRange}
      {...(dailyYRange ? { yRange: dailyYRange } : {})}
      mode="bar"
      interactionMode={interactionMode}
    />
  );
}

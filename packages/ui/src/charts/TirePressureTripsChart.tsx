import * as React from 'react';
import type { TirePressureTimelineSample, TirePressureTimelineTrip } from '@panda/types';
import { formatAppDateTime } from '../lib/dateTime';
import { formatDuration, formatMiles, formatNumber } from '../lib/utils';
import { CHART_COLORS } from './ChartProvider';
import { RichTimeSeriesChart, type PackedRichTimeInterval } from './RichTimeSeriesChart';

export interface TirePressureTripsChartProps {
  samples: TirePressureTimelineSample[];
  trips: TirePressureTimelineTrip[];
  targetPressure: number;
  pressureFactor?: number;
  pressureUnit?: string;
  height?: number;
  loading?: boolean;
  emptyTitle?: string;
  interactionMode?: 'standard' | 'touch-explore';
  onTripClick?: (tripId: string) => void;
  seriesStyles?: TirePressureSeriesStyle[];
  yRange?: [number, number];
  showLegend?: boolean;
  showGrid?: boolean;
  showTooltip?: boolean;
  showPoints?: boolean;
  yAxisLabel?: string;
}

export interface TirePressureSeriesStyle {
  field: 'tire_fl_psi' | 'tire_fr_psi' | 'tire_rl_psi' | 'tire_rr_psi';
  key: string;
  label: string;
  color: string;
  mode?: 'line' | 'area' | 'bar' | 'scatter';
  interpolation?: 'step';
  showInLegend?: boolean;
  pointSize?: number;
  strokeWidth?: number;
}

const TIRE_SERIES = [
  { key: 'tire_fl_psi', label: 'Front left', color: CHART_COLORS.accent },
  { key: 'tire_fr_psi', label: 'Front right', color: CHART_COLORS.sky },
  { key: 'tire_rl_psi', label: 'Rear left', color: CHART_COLORS.emerald },
  { key: 'tire_rr_psi', label: 'Rear right', color: CHART_COLORS.amber },
] as const;

function tripLabel(trip: TirePressureTimelineTrip) {
  const start = trip.start_place || trip.start_address;
  const end = trip.end_place || trip.end_address;
  if (start && end) return `${start} → ${end}`;
  if (start) return `${start} → Trip`;
  if (end) return `Trip → ${end}`;
  return `Trip ${trip.id.slice(0, 8)}`;
}

function tripDetails(trip: TirePressureTimelineTrip) {
  const duration = trip.duration_min != null ? formatDuration(trip.duration_min) : null;
  const distance = trip.distance_miles != null ? formatMiles(trip.distance_miles) : null;
  const when = `${formatAppDateTime(trip.started_at)}–${formatAppDateTime(trip.ended_at)}`;
  return [when, duration, distance].filter(Boolean).join(' · ');
}

function convertedPressure(value: number | null, factor: number) {
  return value == null ? null : value * factor;
}

export function TirePressureTripsChart({
  samples,
  trips,
  targetPressure,
  pressureFactor = 1,
  pressureUnit = 'psi',
  height = 320,
  loading = false,
  emptyTitle = 'No tire pressure or trip data for this period',
  interactionMode = 'standard',
  onTripClick,
  seriesStyles,
  yRange,
  showLegend,
  showGrid,
  showTooltip,
  showPoints,
  yAxisLabel,
}: TirePressureTripsChartProps) {
  const { points, series, intervals } = React.useMemo(() => {
    const events = new Map<string, { time: number; sample?: TirePressureTimelineSample }>();
    for (const sample of samples) events.set(sample.ts, { time: Date.parse(sample.ts), sample });
    const intervals = trips.map(trip => {
      for (const ts of [trip.started_at, trip.ended_at]) if (!events.has(ts)) events.set(ts, { time: Date.parse(ts) });
      return { id: trip.id, start: trip.started_at, end: trip.ended_at, label: tripLabel(trip), details: tripDetails(trip), color: CHART_COLORS.violet };
    });
    const timeline = [...events].sort((a, b) => a[1].time - b[1].time);
    const styles: TirePressureSeriesStyle[] = seriesStyles ?? TIRE_SERIES.map(tire => ({ ...tire, field: tire.key }));
    const columns = styles.map(() => [] as Array<number | null>);
    const points = timeline.map(([ts, event]) => {
      styles.forEach((style, column) => columns[column]!.push(convertedPressure(event.sample?.[style.field] ?? null, pressureFactor)));
      return { ts };
    });
    const series = styles.map((tire, index) => ({ ...tire, strokeWidth: tire.strokeWidth ?? 2.5, values: columns[index]!,
      tooltipFormatter: (value: number | null | undefined) => value == null ? '—' : `${formatNumber(value, 0)} ${pressureUnit}`,
    }));
    return { points, series, intervals };
  }, [samples, trips, seriesStyles, pressureFactor, pressureUnit]);

  return (
    <RichTimeSeriesChart
      rendererId="tire-pressure-timeline"
      points={points}
      series={series}
      height={height}
      loading={loading}
      emptyTitle={emptyTitle}
      yUnit={pressureUnit}
      yRange={yRange}
      showLegend={showLegend}
      showGrid={showGrid}
      showTooltip={showTooltip}
      showPoints={showPoints}
      yAxisLabel={yAxisLabel}
      yValueFormatter={(value, unit) => value == null ? '—' : `${formatNumber(value, 0)} ${unit ?? pressureUnit}`}
      mode="line"
      timeFilter="raw"
      smoothness="straight"
      connectGaps
      yRightUnit="Trips"
      intervalBandRatio={0.3}
      yRightAxisValueFormatter={(value) => value == null ? '' : `${Math.round(value)}`}
      intervals={intervals}
      referenceLines={[{ value: targetPressure * pressureFactor, label: 'Target', color: CHART_COLORS.muted }]}
      onIntervalClick={(interval: PackedRichTimeInterval) => onTripClick?.(interval.id)}
      interactionMode={interactionMode}
    />
  );
}

import { elapsedTripLabel as formatElapsed } from './chartPresentation';
import { tripTooltipFormatter } from './chartFormatters';
import * as React from 'react';
import {
  ResponsiveContainer,
  ComposedChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ReferenceLine,
} from 'recharts';
import { ChartTooltip } from './ChartTooltip';
import { CHART_COLORS, CHART_MARGINS, CHART_X_AXIS_DEFAULTS, TICK_STYLE, TOOLTIP_CURSOR_STYLE } from './ChartProvider';
import { formatPressure } from '../lib/utils';
import { useTripMeasurements } from './tripMeasurements';


export interface TripTirePressurePoint {
  elapsed_s: number;
  tire_fl_psi: number | null;
  tire_fr_psi: number | null;
  tire_rl_psi: number | null;
  tire_rr_psi: number | null;
}

export interface TripTirePressureChartProps {
  data: TripTirePressurePoint[];
  loading?: boolean;
  height?: number;
  activeElapsedS?: number | null;
  onActiveElapsedSChange?: (value: number | null) => void;
}



const formatTooltip = tripTooltipFormatter('tires');

const MARKERS = {
  tire_fl_psi: { r: 2, strokeWidth: 0, fill: CHART_COLORS.accent },
  tire_fr_psi: { r: 2, strokeWidth: 0, fill: CHART_COLORS.sky },
  tire_rl_psi: { r: 2, strokeWidth: 0, fill: CHART_COLORS.success },
  tire_rr_psi: { r: 2, strokeWidth: 0, fill: CHART_COLORS.warning },
};

export function TripTirePressureChart({
  data,
  loading = false,
  height = 240,
  activeElapsedS = null,
  onActiveElapsedSChange,
}: TripTirePressureChartProps) {
  const measurements = useTripMeasurements(data, MARKERS, activeElapsedS, onActiveElapsedSChange);

  if (loading) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        Loading tire pressure chart...
      </div>
    );
  }

  if (!measurements.measured.length) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        No tire pressure data for this trip.
      </div>
    );
  }

  return (
    <ResponsiveContainer width="100%" height={height}>
      <ComposedChart
        data={data}
        syncId="trip-timeline-sync"
        syncMethod="value"
        margin={CHART_MARGINS.withYAxis}
        onMouseMove={measurements.onMouseMove}
        onMouseLeave={measurements.onMouseLeave}
      >
        <CartesianGrid strokeDasharray="3 3" stroke={CHART_COLORS.grid} vertical={false} />
        <XAxis
          dataKey="elapsed_s"
          {...CHART_X_AXIS_DEFAULTS}
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          tickFormatter={formatElapsed}
          minTickGap={45}
        />
        <YAxis
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          tickFormatter={(value: number) => formatPressure(value)}
          width={34}
        />
        <Tooltip
          content={<ChartTooltip
            labelFormatter={(value) => formatElapsed(Number(value))}
            formatter={formatTooltip}
          />}
          cursor={TOOLTIP_CURSOR_STYLE}
        />
        <Legend
          verticalAlign="top"
          height={22}
          wrapperStyle={{ fontSize: 11, color: CHART_COLORS.muted, paddingTop: 8 }}
          iconType="line"
        />
        {activeElapsedS != null ? (
          <ReferenceLine x={activeElapsedS} stroke={CHART_COLORS.muted} strokeDasharray="4 4" />
        ) : null}
        <Line
          type="monotone"
          dataKey="tire_fl_psi"
          name="tire_fl_psi"
          stroke={CHART_COLORS.accent}
          strokeWidth={1.8}
          dot={measurements.dots.tire_fl_psi}
      activeDot={{ r: 3.8, strokeWidth: 2, stroke: 'var(--rm-bg-surface)', fill: CHART_COLORS.accent }}
          isAnimationActive={false}
          connectNulls
        />
        <Line
          type="monotone"
          dataKey="tire_fr_psi"
          name="tire_fr_psi"
          stroke={CHART_COLORS.sky}
          strokeWidth={1.8}
          dot={measurements.dots.tire_fr_psi}
      activeDot={{ r: 3.8, strokeWidth: 2, stroke: 'var(--rm-bg-surface)', fill: CHART_COLORS.sky }}
          isAnimationActive={false}
          connectNulls
        />
        <Line
          type="monotone"
          dataKey="tire_rl_psi"
          name="tire_rl_psi"
          stroke={CHART_COLORS.success}
          strokeWidth={1.8}
          dot={measurements.dots.tire_rl_psi}
      activeDot={{ r: 3.8, strokeWidth: 2, stroke: 'var(--rm-bg-surface)', fill: CHART_COLORS.success }}
          isAnimationActive={false}
          connectNulls
        />
        <Line
          type="monotone"
          dataKey="tire_rr_psi"
          name="tire_rr_psi"
          stroke={CHART_COLORS.warning}
          strokeWidth={1.8}
          dot={measurements.dots.tire_rr_psi}
      activeDot={{ r: 3.8, strokeWidth: 2, stroke: 'var(--rm-bg-surface)', fill: CHART_COLORS.warning }}
          isAnimationActive={false}
          connectNulls
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}

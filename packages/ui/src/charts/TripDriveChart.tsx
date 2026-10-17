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
import { useTripMeasurements } from './tripMeasurements';


export interface TripDrivePoint {
  elapsed_s: number;
  power_kw: number | null;
  regen_kw: number | null;
  speed_mph: number | null;
}

export interface TripDriveChartProps {
  data: TripDrivePoint[];
  loading?: boolean;
  height?: number;
  activeElapsedS?: number | null;
  onActiveElapsedSChange?: (value: number | null) => void;
}



const formatTooltip = tripTooltipFormatter('drive');

const MARKERS = {
  power_kw: { r: 2.1, strokeWidth: 0, fill: CHART_COLORS.accent },
  regen_kw: { r: 2.1, strokeWidth: 0, fill: CHART_COLORS.sky },
  speed_mph: { r: 2, strokeWidth: 0, fill: CHART_COLORS.warning },
};

export function TripDriveChart({
  data,
  loading = false,
  height = 280,
  activeElapsedS = null,
  onActiveElapsedSChange,
}: TripDriveChartProps) {
  const measurements = useTripMeasurements(data, MARKERS, activeElapsedS, onActiveElapsedSChange);

  if (loading) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        Loading drive chart...
      </div>
    );
  }

  if (data.length === 0) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        No drive profile data for this trip.
      </div>
    );
  }

  if (!measurements.measured.length) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        No power samples were captured for this trip.
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
          yAxisId="power"
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          tickFormatter={(value: number) => `${Math.round(value)}`}
          width={34}
        />
        <YAxis
          yAxisId="speed"
          orientation="right"
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          tickFormatter={(value: number) => `${Math.round(value)} mph`}
          width={36}
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
        {activeElapsedS != null ? <ReferenceLine x={activeElapsedS} stroke={CHART_COLORS.muted} strokeDasharray="4 4" /> : null}
        <Line
          type="monotone"
          dataKey="power_kw"
          name="Power"
          stroke={CHART_COLORS.accent}
          strokeWidth={2}
          dot={measurements.dots.power_kw}
          activeDot={{ r: 4.2, fill: CHART_COLORS.accent, stroke: 'var(--rm-bg-surface)', strokeWidth: 2 }}
          isAnimationActive={false}
          connectNulls
          yAxisId="power"
        />
        <Line
          type="monotone"
          dataKey="regen_kw"
          name="Regen"
          stroke={CHART_COLORS.sky}
          strokeWidth={1.6}
          dot={measurements.dots.regen_kw}
          activeDot={{ r: 4.2, fill: CHART_COLORS.sky, stroke: 'var(--rm-bg-surface)', strokeWidth: 2 }}
          isAnimationActive={false}
          connectNulls
          yAxisId="power"
        />
        <Line
          type="monotone"
          dataKey="speed_mph"
          name="Speed"
          stroke={CHART_COLORS.warning}
          strokeWidth={1.4}
          strokeDasharray="4 4"
          dot={measurements.dots.speed_mph}
          activeDot={{ r: 4, fill: CHART_COLORS.warning, stroke: 'var(--rm-bg-surface)', strokeWidth: 2 }}
          isAnimationActive={false}
          connectNulls
          yAxisId="speed"
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}

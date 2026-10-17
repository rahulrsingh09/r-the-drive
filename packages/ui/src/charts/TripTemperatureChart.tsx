import { elapsedTripLabel as formatElapsed } from './chartPresentation';
import { tripTooltipFormatter } from './chartFormatters';
import * as React from 'react';
import {
  ResponsiveContainer,
  ComposedChart,
  Line,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ReferenceLine,
} from 'recharts';
import { ChartTooltip } from './ChartTooltip';
import { CHART_COLORS, CHART_MARGINS, CHART_X_AXIS_DEFAULTS, TICK_STYLE, TOOLTIP_CURSOR_STYLE } from './ChartProvider';
import { formatTemp } from '../lib/utils';
import { useTripMeasurements } from './tripMeasurements';
import { getTripTemperatureDomain } from './TripChartRendering';

export interface TripTemperaturePoint {
  elapsed_s: number;
  outside_temp_c: number | null;
  cabin_temp_c: number | null;
  driver_temp_c?: number | null;
  hvac_active?: boolean | null;
}

export interface TripTemperatureChartProps {
  data: TripTemperaturePoint[];
  loading?: boolean;
  height?: number;
  activeElapsedS?: number | null;
  onActiveElapsedSChange?: (value: number | null) => void;
}



const formatTooltip = tripTooltipFormatter('temperature');

const MARKERS = {
  outside_temp_c: { r: 2, strokeWidth: 0, fill: CHART_COLORS.emerald },
  cabin_temp_c: { r: 2, strokeWidth: 0, fill: CHART_COLORS.orange },
  driver_temp_c: { r: 2, strokeWidth: 0, fill: CHART_COLORS.yellow },
};

export function TripTemperatureChart({
  data,
  loading = false,
  height = 240,
  activeElapsedS = null,
  onActiveElapsedSChange,
}: TripTemperatureChartProps) {
  const chartData = data.map((point) => ({
    ...point,
    hvac_on: point.hvac_active ? 1 : 0,
  }));
  const measurements = useTripMeasurements(data, MARKERS, activeElapsedS, onActiveElapsedSChange);
  const temperatureDomain = React.useMemo(() => {
    const readings: Array<number | null | undefined> = [];
    for (const point of data) readings.push(point.outside_temp_c, point.cabin_temp_c, point.driver_temp_c);
    return getTripTemperatureDomain(readings);
  }, [data]);

  if (loading) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        Loading temperature chart...
      </div>
    );
  }

  if (!measurements.measured.length) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        Outside temp unavailable for this trip.
      </div>
    );
  }

  return (
    <ResponsiveContainer width="100%" height={height}>
      <ComposedChart
        data={chartData}
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
          domain={temperatureDomain ?? ['auto', 'auto']}
          tickFormatter={(value: number) => formatTemp(value)}
          width={34}
        />
        <YAxis yAxisId="climate" domain={[0, 1]} hide />
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
        <Area
          yAxisId="climate"
          type="stepAfter"
          dataKey="hvac_on"
          name="Climate On"
          stroke="none"
          fill={CHART_COLORS.violet}
          fillOpacity={0.16}
          isAnimationActive={false}
        />
        <Line
          type="monotone"
          dataKey="outside_temp_c"
          name="Outside Temp"
          stroke={CHART_COLORS.emerald}
          strokeWidth={1.8}
          dot={measurements.dots.outside_temp_c}
          activeDot={{ r: 3.8, fill: CHART_COLORS.emerald, stroke: 'var(--rm-bg-surface)', strokeWidth: 2 }}
          isAnimationActive={false}
          connectNulls
        />
        <Line
          type="monotone"
          dataKey="cabin_temp_c"
          name="Cabin Temp"
          stroke={CHART_COLORS.orange}
          strokeWidth={1.8}
          dot={measurements.dots.cabin_temp_c}
          activeDot={{ r: 3.8, fill: CHART_COLORS.orange, stroke: 'var(--rm-bg-surface)', strokeWidth: 2 }}
          isAnimationActive={false}
          connectNulls
        />
        <Line
          type="monotone"
          dataKey="driver_temp_c"
          name="Driver Setpoint"
          stroke={CHART_COLORS.yellow}
          strokeWidth={1.6}
          strokeDasharray="4 4"
          dot={measurements.dots.driver_temp_c}
          activeDot={{ r: 3.8, fill: CHART_COLORS.yellow, stroke: 'var(--rm-bg-surface)', strokeWidth: 2 }}
          isAnimationActive={false}
          connectNulls
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}

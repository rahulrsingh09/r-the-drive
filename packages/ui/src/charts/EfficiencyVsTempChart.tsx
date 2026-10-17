import * as React from 'react';
import { thresholdColor } from './chartPresentation';
import {
  ResponsiveContainer, BarChart, Bar,
  XAxis, YAxis, CartesianGrid, Tooltip, Cell,
} from 'recharts';
import { ChartTooltip } from './ChartTooltip';
import { CHART_BAR_STYLE, CHART_COLORS, CHART_MARGINS, CHART_X_AXIS_DEFAULTS, TICK_STYLE, TOOLTIP_CURSOR_STYLE } from './ChartProvider';
import { ChartSkeleton } from '../primitives/Skeleton';

export interface EfficiencyVsTempPoint {
  temp_c_low: number;
  temp_c_high: number;
  avg_efficiency_wh_mi: number | null;
  trip_count: number;
}

export interface EfficiencyVsTempChartProps {
  data: EfficiencyVsTempPoint[];
  loading?: boolean;
  height?: number;
  unit?: 'c' | 'f';
}

function toF(c: number) { return Math.round(c * 9 / 5 + 32); }

const EFFICIENCY_BANDS = [[300, CHART_COLORS.success], [380, CHART_COLORS.warning]] as const;

export function EfficiencyVsTempChart({
  data,
  loading = false,
  height = 200,
  unit = 'f',
}: EfficiencyVsTempChartProps) {
  if (loading) return <ChartSkeleton className={`h-[${height}px]`} />;

  const chartData: Array<EfficiencyVsTempPoint & { label: string; efficiency: number | null }> = [];
  const cells: React.ReactNode[] = [];
  const temperature = unit === 'f' ? toF : (value: number) => value;
  for (const entry of data) {
    const efficiency = entry.avg_efficiency_wh_mi;
    const fill = efficiency === null ? CHART_COLORS.muted : thresholdColor(efficiency, EFFICIENCY_BANDS, 'below');
    cells.push(<Cell key={`cell-${cells.length}`} fill={fill} fillOpacity={CHART_BAR_STYLE.fillOpacity} />);
    chartData.push({ ...entry, label: `${temperature(entry.temp_c_low)}°`, efficiency });
  }

  return (
    <ResponsiveContainer width="100%" height={height}>
      <BarChart data={chartData} margin={CHART_MARGINS.withYAxis}>
        <CartesianGrid strokeDasharray="3 3" stroke={CHART_COLORS.grid} vertical={false} />
        <XAxis
          dataKey="label"
          {...CHART_X_AXIS_DEFAULTS}
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          label={{ value: unit === 'f' ? '°F' : '°C', position: 'insideBottomRight', offset: 0, style: TICK_STYLE }}
        />
        <YAxis
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          tickFormatter={(v: number) => `${v}`}
          unit=" Wh"
          width={52}
        />
        <Tooltip
          content={<ChartTooltip
            formatter={(v, _) => [
              v !== undefined ? `${v.toFixed(0)} Wh/mi` : '—',
              'Avg efficiency',
            ]}
          />}
          cursor={TOOLTIP_CURSOR_STYLE}
        />
        <Bar dataKey="efficiency" radius={[CHART_BAR_STYLE.radius, CHART_BAR_STYLE.radius, 0, 0]} fillOpacity={CHART_BAR_STYLE.fillOpacity} isAnimationActive={false}>
          {cells}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  );
}

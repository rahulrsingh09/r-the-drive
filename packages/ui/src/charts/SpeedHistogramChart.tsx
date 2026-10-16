import * as React from 'react';
import {
  ResponsiveContainer,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Cell,
} from 'recharts';
import { ChartTooltip } from './ChartTooltip';
import { CHART_BAR_STYLE, CHART_COLORS, CHART_MARGINS, resolveChartColor, TICK_STYLE, TOOLTIP_CURSOR_STYLE } from './ChartProvider';
import { formatDuration } from '../lib/utils';
import { useDocumentPalette } from '../hooks/useDocumentPalette';
import { useDocumentTheme } from '../hooks/useDocumentTheme';
import { useThemeRevision } from '../lib/themeRuntime';

function blendColor(fromColor: string, toColor: string, ratio: number) {
  const t = Math.max(0, Math.min(1, ratio));
  return `color-mix(in oklab, ${fromColor} ${(1 - t) * 100}%, ${toColor} ${t * 100}%)`;
}

export interface SpeedHistogramBin {
  label: string;
  min: number;
  max: number;
  count: number;
  duration_seconds: number;
  recorded_seconds?: number;
  estimated_seconds?: number;
  sample_elapsed_s: number | null;
}

export interface SpeedHistogramChartProps {
  bins: SpeedHistogramBin[];
  loading?: boolean;
  height?: number;
  activeBinLabel?: string | null;
  speedUnit?: 'mph' | 'km/h';
}

export function SpeedHistogramChart({
  bins,
  loading = false,
  height = 280,
  activeBinLabel = null,
  speedUnit = 'mph',
}: SpeedHistogramChartProps) {
  const palette = useDocumentPalette();
  const isDark = useDocumentTheme();
  const themeRevision = useThemeRevision();
  const hasEstimates = bins.some(bin => (bin.estimated_seconds ?? 0) > 0);
  const hasRecorded = bins.some(bin => (bin.recorded_seconds ?? 0) > 0);
  const successColor = React.useMemo(() => resolveChartColor(CHART_COLORS.success), [isDark, palette, themeRevision]);
  const accentColor = React.useMemo(() => resolveChartColor(CHART_COLORS.accent), [isDark, palette, themeRevision]);

  if (loading) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        Loading speed histogram...
      </div>
    );
  }

  if (bins.length === 0) {
    return (
      <div className="flex items-center justify-center rounded-lg border border-border bg-bg-elevated text-sm text-fg-tertiary" style={{ height }}>
        No speed data for this trip.
      </div>
    );
  }

  const maxDuration = Math.max(...bins.map((bin) => bin.duration_seconds));
  const minDuration = Math.min(...bins.map((bin) => bin.duration_seconds));
  const durationRange = Math.max(1, maxDuration - minDuration);
  const displayBins = bins.map(bin => ({
    ...bin,
    displayLabel: speedUnit === 'km/h' ? `${Math.round(bin.min * 1.609344)}-${Math.round(bin.max * 1.609344)}` : bin.label,
  }));

  return (
    <div>
    {hasEstimates && hasRecorded ? (
      <div className="mb-3 flex flex-wrap gap-x-4 gap-y-2 text-xs text-fg-secondary">
        <span className="inline-flex items-center gap-1.5"><span className="h-2 w-2 rounded-full" style={{ backgroundColor: CHART_COLORS.sky }} />Recorded</span>
        <span className="inline-flex items-center gap-1.5"><span className="h-2 w-2 rounded-full" style={{ backgroundColor: CHART_COLORS.success }} />GPS estimated</span>
      </div>
    ) : null}
    <ResponsiveContainer width="100%" height={height}>
      <BarChart
        data={displayBins}
        margin={CHART_MARGINS.withYAxis}
      >
        <CartesianGrid strokeDasharray="3 3" stroke={CHART_COLORS.grid} vertical={false} />
        <XAxis
          dataKey="displayLabel"
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          interval="preserveStartEnd"
          minTickGap={8}
          angle={-30}
          textAnchor="end"
          height={48}
        />
        <YAxis
          tick={TICK_STYLE}
          tickLine={false}
          axisLine={false}
          allowDecimals={false}
          width={52}
          tickFormatter={(value) => formatHistogramDuration(Number(value))}
        />
        <Tooltip
          content={<ChartTooltip
            formatter={(value, name) => [formatHistogramDuration(Number(value)), hasEstimates ? name : 'Time']}
            labelFormatter={(value) => `${String(value)} ${speedUnit}`}
          />}
          cursor={TOOLTIP_CURSOR_STYLE}
        />
        {hasEstimates ? (
          <>
            {hasRecorded ? <Bar dataKey="recorded_seconds" name="Recorded" stackId="time" fill={CHART_COLORS.sky} fillOpacity={CHART_BAR_STYLE.fillOpacity} /> : null}
            <Bar dataKey="estimated_seconds" name="GPS estimated" stackId="time" fill={CHART_COLORS.success} radius={[CHART_BAR_STYLE.radius, CHART_BAR_STYLE.radius, 0, 0]}>
              {bins.map(bin => <Cell key={bin.label} fillOpacity={bin.label === activeBinLabel ? CHART_BAR_STYLE.activeOpacity : CHART_BAR_STYLE.fillOpacity} />)}
            </Bar>
          </>
        ) : <Bar dataKey="duration_seconds" radius={[CHART_BAR_STYLE.radius, CHART_BAR_STYLE.radius, 0, 0]}>
          {bins.map((bin) => {
            const intensity = (bin.duration_seconds - minDuration) / durationRange;
            const fill = blendColor(successColor, accentColor, intensity);
            return (
              <Cell
                key={bin.label}
                fill={fill}
                fillOpacity={bin.label === activeBinLabel ? CHART_BAR_STYLE.activeOpacity : CHART_BAR_STYLE.fillOpacity}
              />
            );
          })}
        </Bar>}
      </BarChart>
    </ResponsiveContainer>
    </div>
  );
}

function formatHistogramDuration(seconds: number) {
  if (!Number.isFinite(seconds)) return '-';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  return formatDuration(seconds / 60);
}

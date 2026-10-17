import { prepareSparklineGeometry, paintSparkline } from './sparklineGeometry';
import React from 'react';
import { CHART_COLORS, getChartColor, resolveChartColor } from './ChartProvider';
import { useDocumentPalette } from '../hooks/useDocumentPalette';
import { useDocumentTheme } from '../hooks/useDocumentTheme';
import { useThemeRevision } from '../lib/themeRuntime';
import {
  bucketTimeSeriesValues,
  DEFAULT_SPRITE_TIME_FILTER,
  filterTimeSeriesValues,
  normalizeTimeFilter,
  type TimeFilterWindow,
} from './timeFilter';
import {
  DEFAULT_CURVE_SMOOTHNESS,
  normalizeCurveSmoothness,
  type CurveSmoothness,
} from './curveSmoothness';

export type MiniSparklineType = 'none' | 'line' | 'area' | 'bar';
export type MiniSparklineYDomain = { min: number; max: number };

export interface MiniSparklineProps {
  data: Array<{ ts?: string; value: number | null | undefined }>;
  type?: MiniSparklineType;
  height?: number;
  color?: string;
  showFallback?: boolean;
  timeFilter?: TimeFilterWindow;
  smoothness?: CurveSmoothness;
  yDomain?: MiniSparklineYDomain | undefined;
}

export function MiniSparkline({
  data,
  type = 'line',
  height = 42,
  color = CHART_COLORS.accent,
  showFallback = true,
  timeFilter = DEFAULT_SPRITE_TIME_FILTER,
  smoothness = DEFAULT_CURVE_SMOOTHNESS,
  yDomain,
}: MiniSparklineProps) {
  if (type === 'none') return null;

  const timestamps: string[] = [];
  const values: Array<number | null> = [];
  let hasValue = false;
  for (const [index, point] of data.entries()) {
    timestamps.push(point.ts ?? String(index));
    const value = Number.isFinite(point.value) ? point.value as number : null;
    values.push(value);
    hasValue ||= value != null;
  }
  if (!hasValue) return showFallback ? <EmptySparkline height={height} color={color} /> : null;
  const resolvedFilter = normalizeTimeFilter(timeFilter, DEFAULT_SPRITE_TIME_FILTER);
  const samples = type === 'bar'
    ? bucketTimeSeriesValues(timestamps, values, resolvedFilter).map(point => ({ x: String(point.timestamp), value: point.value }))
    : filterTimeSeriesValues(timestamps, values, resolvedFilter).map((value, index) => ({ x: timestamps[index]!, value }));
  return <CanvasSparkline data={samples} type={type === 'bar' ? 'bar' : 'line'} height={height} color={color}
    {...(type !== 'bar' ? { fill: type === 'area' } : {})}
    timeFilter={resolvedFilter} smoothness={normalizeCurveSmoothness(smoothness)} yDomain={yDomain} />;

}

function EmptySparkline({ height, color }: { height: number; color: string }) {
  return (
    <div
      style={{ height, width: '100%' }}
      className="overflow-hidden rounded"
      data-sparkline-state="empty"
    >
      <svg
        style={{ display: 'block', height: '100%', width: '100%' }}
        viewBox="0 0 100 36"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <path
          d="M0 28 C18 23 30 31 48 26 S78 22 100 27"
          fill="none"
          stroke={color}
          strokeWidth="1.2"
          strokeOpacity="0.22"
          strokeDasharray="3 4"
        />
      </svg>
    </div>
  );
}

export function resolveCanvasColor(canvas: HTMLCanvasElement, color: string) {
  return resolveChartColor(color, canvas, getChartColor('accent', 'classic'));
}

function CanvasSparkline({
  data,
  type,
  height,
  color,
  fill,
  timeFilter,
  smoothness,
  yDomain,
}: {
  data: Array<{ x: string; value: number | null }>;
  type: Exclude<MiniSparklineType, 'none' | 'area'>;
  height: number;
  color: string;
  fill?: boolean;
  timeFilter: TimeFilterWindow;
  smoothness: CurveSmoothness;
  yDomain?: MiniSparklineYDomain | undefined;
}) {
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);
  const palette = useDocumentPalette();
  const isDark = useDocumentTheme();
  const themeRevision = useThemeRevision();

  React.useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return undefined;

    const draw = () => {
      const width = Math.max(1, Math.round(canvas.clientWidth));
      const pixelRatio = window.devicePixelRatio || 1;
      canvas.width = Math.round(width * pixelRatio);
      canvas.height = Math.round(height * pixelRatio);

      const context = canvas.getContext('2d');
      if (!context) return;
      const canvasColor = resolveCanvasColor(canvas, color);
      context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
      context.clearRect(0, 0, width, height);

      const geometry = prepareSparklineGeometry(data, width, height, type, timeFilter, smoothness, yDomain);
      paintSparkline(context, geometry, canvasColor, height, fill === true);
    };

    draw();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(draw);
    observer?.observe(canvas);
    return () => observer?.disconnect();
  }, [color, data, fill, height, isDark, palette, themeRevision, smoothness, timeFilter, type, yDomain?.max, yDomain?.min]);

  return (
    <div
      style={{ height, width: '100%' }}
      className="overflow-hidden"
      data-sparkline-state={data.length === 1 ? 'single' : 'series'}
      data-sparkline-renderer="canvas"
      data-sparkline-filter={timeFilter}
      data-sparkline-point-count={data.length}
      data-sparkline-domain-min={yDomain?.min}
      data-sparkline-domain-max={yDomain?.max}
      data-sparkline-aggregation={type === 'bar' && timeFilter !== 'raw' ? 'sum' : 'none'}
      data-sparkline-smoothness={smoothness}
    >
      <canvas
        ref={canvasRef}
        style={{ display: 'block', height: '100%', width: '100%' }}
        aria-hidden="true"
      />
    </div>
  );
}

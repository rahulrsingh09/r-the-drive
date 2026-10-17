import { createRichCursorController, observeChartFrame } from './richCursor';
import * as React from 'react';
import uPlot from 'uplot';
import { RotateCcw } from 'lucide-react';
import type { AlignedData, Options, Series } from 'uplot';
import 'uplot/dist/uPlot.min.css';
import { cn } from '../lib/utils';
import { ChartSkeleton } from '../primitives/Skeleton';
import { CHART_BAR_STYLE, CHART_COLORS, CHART_FONT, resolveChartColor } from './ChartProvider';
import { formatNumber, formatSmartNumber } from '../lib/utils';
import { formatAppDate, formatAppDateTime, formatAppTime } from '../lib/dateTime';
import { filterTimeSeriesValues, type TimeFilterWindow } from './timeFilter';
import { DEFAULT_CURVE_SMOOTHNESS, normalizeCurveSmoothness, type CurveSmoothness } from './curveSmoothness';
import { useDocumentPalette } from '../hooks/useDocumentPalette';
import { useDocumentTheme } from '../hooks/useDocumentTheme';
import { useThemeRevision } from '../lib/themeRuntime';
import { attachChartScrubbing } from './attachChartScrubbing';

const NATIVE_SPLINE_PATH = uPlot.paths.spline!();

export interface RichSeries {
  key: string;
  label: string;
  color?: string;
  values: Array<number | null>;
  mode?: 'line' | 'area' | 'bar' | 'scatter';
  /** Optional per-series path interpolation. */
  interpolation?: 'step';
  /** Marker diameter for scatter series. */
  pointSize?: number;
  /** Stroke width for line and area series. */
  strokeWidth?: number;
  /** Dash pattern used to distinguish cycled colors in dense categorical charts. */
  dash?: number[];
  /** Include values in the hover tooltip without drawing a series or legend item. */
  tooltipOnly?: boolean;
  /** Draw the series without adding another legend item. */
  showInLegend?: boolean;
  /** Formats this series in the hover tooltip, independent of the chart axis unit. */
  tooltipFormatter?: (value: number | null | undefined) => string;
  /** Optional per-point context appended to this series' hover row. */
  tooltipDetails?: Array<string | null | undefined>;
  /** Which Y scale this series is drawn on. Default 'y' (left). Use 'y2' for a right axis. */
  yScale?: 'y' | 'y2';
  /** Keep cumulative or derived supporting series raw while filtering the primary line. */
  filterable?: boolean;
  /** Opt out supporting/derived line series from display curve shaping. */
  smoothable?: boolean;
  /** Override gap drawing and tooltip carry-forward for this series. */
  connectGaps?: boolean;
  /** Stack bar values with other bar series carrying the same ID. */
  stackId?: string;
}

export interface RichTimeInterval {
  id: string;
  start: string | number | Date;
  end: string | number | Date;
  label: string;
  details?: string;
  color?: string;
}

export interface RichReferenceLine {
  value: number;
  label?: string;
  color?: string;
}

export interface PackedRichTimeInterval extends RichTimeInterval {
  lane: number;
}

export interface RichTimeIntervalBandGeometry {
  bandTop: number;
  bandHeight: number;
  laneTop: number;
  laneHeight: number;
  top: number;
  height: number;
  hitTop: number;
  hitHeight: number;
}

/** Calculate the compact bottom lane used by interval overlays. */
export function getRichTimeIntervalBandGeometry(
  lane: number,
  laneCount: number,
  plotTop: number,
  plotHeight: number,
  bandRatio = 0.3,
): RichTimeIntervalBandGeometry {
  const plot = { top: plotTop, height: Math.max(0, plotHeight) };
  const count = Math.max(1, laneCount);
  const bandHeight = plot.height * Math.min(0.45, Math.max(0.2, bandRatio));
  const bandTop = plot.top + plot.height - bandHeight;
  const laneHeight = bandHeight / count;
  const laneTop = bandTop + Math.min(Math.max(0, lane), count - 1) * laneHeight;
  const sizes = { visual: Math.max(4, laneHeight * 0.72), hit: Math.max(44, laneHeight) };
  const positions = { visual: 0, hit: 0 };
  for (const kind of ['visual', 'hit'] as const) {
    const offset = (laneHeight - sizes[kind]) / 2;
    if (kind === 'visual') positions[kind] = laneTop + Math.max(0, offset);
    else if (plot.height < sizes[kind]) positions[kind] = plot.top;
    else positions[kind] = Math.min(plot.top + plot.height - sizes[kind], Math.max(plot.top, laneTop + offset));
  }
  return { bandTop, bandHeight, laneTop, laneHeight, top: positions.visual, height: sizes.visual, hitTop: positions.hit, hitHeight: sizes.hit };
}

/** Assign overlapping intervals to the smallest available horizontal lane. */
export function packRichTimeIntervals(intervals: RichTimeInterval[]): PackedRichTimeInterval[] {
  const ordered: Array<{ interval: RichTimeInterval; start: number; end: number; index: number }> = [];
  intervals.forEach((interval, index) => {
    const start = toSeconds(interval.start);
    const end = toSeconds(interval.end);
    if (Number.isFinite(start) && Number.isFinite(end) && end > start) ordered.push({ interval, start, end, index });
  });
  ordered.sort((a, b) => a.start - b.start || a.end - b.end || a.index - b.index);
  const lanes: Array<{ end: number; intervals: PackedRichTimeInterval[] }> = [];
  for (const entry of ordered) {
    let lane = 0;
    while (lane < lanes.length && lanes[lane]!.end > entry.start) lane += 1;
    const packed = { ...entry.interval, lane };
    if (lanes[lane]) { lanes[lane]!.end = entry.end; lanes[lane]!.intervals.push(packed); }
    else lanes.push({ end: entry.end, intervals: [packed] });
  }
  return lanes.flatMap(lane => lane.intervals).sort((a, b) => toSeconds(a.start) - toSeconds(b.start) || a.lane - b.lane);
}


export interface RichTimeSeriesChartProps {
  /** Stable production renderer identity for compatibility tests and diagnostics. */
  rendererId?: string;
  points: Array<{ ts: string | number | Date }>;
  series: RichSeries[];
  height?: number;
  loading?: boolean;
  emptyTitle?: string | undefined;
  emptyDescription?: string | undefined;
  /** Accessible/displayed titles for the chart axes. */
  xAxisLabel?: string | undefined;
  yAxisLabel?: string | undefined;
  yRightAxisLabel?: string | undefined;
  /** Display controls used by definition-driven charts. Omitted values preserve legacy behavior. */
  showLegend?: boolean | undefined;
  showGrid?: boolean | undefined;
  showTooltip?: boolean | undefined;
  showPoints?: boolean | undefined;
  yUnit?: string | undefined;
  /** Unit label for the right Y axis (only shown when any series has yScale='y2'). */
  yRightUnit?: string | undefined;
  className?: string | undefined;
  mode?: 'line' | 'area' | 'bar' | 'scatter' | undefined;
  xTime?: boolean;
  xUnit?: string | undefined;
  xValueFormatter?: ((value: number) => string) | undefined;
  /** Secondary X axis shown at the top of the chart (same scale as primary X, different label format). */
  xSecondaryFormatter?: ((value: number) => string) | undefined;
  /** Optional formatter for primary Y-axis tick labels. Tooltip formatting remains controlled by yValueFormatter. */
  yAxisValueFormatter?: ((value: number | null | undefined, unit?: string) => string) | undefined;
  /** Optional formatter for secondary Y-axis tick labels. Tooltip formatting remains controlled by yValueFormatter. */
  yRightAxisValueFormatter?: ((value: number | null | undefined, unit?: string) => string) | undefined;
  yValueFormatter?: ((value: number | null | undefined, unit?: string) => string) | undefined;
  timeFilter?: TimeFilterWindow | undefined;
  smoothness?: CurveSmoothness | undefined;
  xRange?: [number, number] | undefined;
  yRange?: [number, number] | undefined;
  yRightRange?: [number, number] | undefined;
  stepInterpolation?: boolean | undefined;
  xSplits?: number[] | undefined;
  /** Native uPlot cursor synchronization group for dense coordinated charts. */
  cursorSyncKey?: string | undefined;
  /** Shared sample selection; enables persistent inspection instead of drag-to-zoom. */
  activeCursorIndex?: number | null | undefined;
  /** Receives the aligned sample index without forcing the chart to re-render. */
  onCursorIndexChange?: ((index: number | null) => void) | undefined;
  /** Receives the currently rendered numeric Y-axis ranges for settings affordances. */
  onResolvedAxisRanges?: ((ranges: { y?: [number, number]; y2?: [number, number] }) => void) | undefined;
  /** Connect line/area paths across null samples and carry the last value in tooltips. */
  connectGaps?: boolean | undefined;
  /** Enables touch-first pan and pinch exploration for a dedicated mobile chart view. */
  interactionMode?: 'standard' | 'touch-explore' | undefined;
  /** Optional time-span marks rendered as accessible, clickable secondary-axis lane bars. */
  intervals?: RichTimeInterval[] | undefined;
  onIntervalClick?: ((interval: PackedRichTimeInterval) => void) | undefined;
  /** Fraction of the plot reserved for interval lanes at the bottom. */
  intervalBandRatio?: number | undefined;
  /** Optional horizontal reference lines on the primary Y axis. */
  referenceLines?: RichReferenceLine[] | undefined;
}

/** Build uPlot's aligned arrays from finite, ascending x values. */
export function buildRichTimeSeriesAlignedData(
  points: RichTimeSeriesChartProps['points'],
  series: RichSeries[],
  timeFilter: TimeFilterWindow = 'raw',
  xTime = true,
): AlignedData {
  const timestamps = points.map(point => point.ts);
  const x = timestamps.map(value => xTime ? toSeconds(value) : Number(value));
  const permutation = Array.from(x.keys()).filter(index => Number.isFinite(x[index]));
  permutation.sort((a, b) => x[a]! - x[b]! || a - b);
  const aligned: Array<Array<number | null>> = [permutation.map(index => x[index]!)];
  for (const item of series) {
    const source = xTime && item.filterable !== false
      ? filterTimeSeriesValues(timestamps, item.values, timeFilter) : item.values;
    aligned.push(permutation.map(index => source[index] ?? null));
  }
  return aligned as AlignedData;
}


export function clampExplorationRange(
  proposed: [number, number],
  bounds: [number, number],
  minimumSpan: number,
): [number, number] {
  const capacity = bounds[1] - bounds[0];
  if (capacity <= 0) return bounds;
  const span = Math.min(capacity, Math.max(minimumSpan, proposed[1] - proposed[0]));
  const left = proposed[0] < bounds[0] ? bounds[0] : proposed[0];
  const right = left + span;
  return right > bounds[1] ? [bounds[1] - span, bounds[1]] : [left, right];
}


export function isZoomedXRange(current: [number, number], full: [number, number]) {
  const tolerance = Math.max(1e-9, Math.abs(full[1] - full[0]) * 1e-6);
  return Math.abs(current[0] - full[0]) > tolerance || Math.abs(current[1] - full[1]) > tolerance;
}

function attachTouchExploration(root: HTMLDivElement, chart: uPlot, bounds: [number, number]) {
  type Point = { x: number; y: number };
  const contacts = new Map<number, Point>();
  const minimumSpan = Math.max((bounds[1] - bounds[0]) / 500, Number.EPSILON);
  const savedTouchAction = root.style.touchAction;
  root.style.touchAction = 'none';
  let tapTime = 0;
  let pan: { point: Point; range: [number, number] } | null = null;
  let pinch: { distance: number; center: number; range: [number, number] } | null = null;
  const range = (): [number, number] => [chart.scales.x?.min ?? bounds[0], chart.scales.x?.max ?? bounds[1]];
  const contactPair = () => {
    const iterator = contacts.values();
    const a = iterator.next().value!;
    const b = iterator.next().value!;
    return { distance: Math.max(1, Math.hypot(b.x - a.x, b.y - a.y)), center: (a.x + b.x) / 2 };
  };
  const dispatch = (event: PointerEvent) => {
    const point = { x: event.clientX, y: event.clientY };
    if (event.type === 'pointerdown') {
      if (event.pointerType !== 'touch') return;
      if (!contacts.size) {
        const now = Date.now();
        if (now - tapTime < 300) { chart.setScale('x', { min: bounds[0], max: bounds[1] }); tapTime = 0; }
        else tapTime = now;
      }
      contacts.set(event.pointerId, point);
      root.setPointerCapture?.(event.pointerId);
      pan = { point, range: range() };
      if (contacts.size > 1) pinch = { ...contactPair(), range: range() };
      return;
    }
    if (event.type !== 'pointermove') {
      contacts.delete(event.pointerId);
      if (contacts.size < 2) pinch = null;
      pan = contacts.size === 1 ? { point: contacts.values().next().value!, range: range() } : null;
      return;
    }
    if (!contacts.has(event.pointerId)) return;
    contacts.set(event.pointerId, point);
    const rect = root.getBoundingClientRect();
    if (rect.width <= 0) return;
    let proposed: [number, number];
    if (contacts.size > 1) {
      const pair = contactPair();
      pinch ??= { ...pair, range: range() };
      const span = pinch.range[1] - pinch.range[0];
      const nextSpan = span * (pinch.distance / pair.distance);
      const ratio = Math.min(1, Math.max(0, (pinch.center - rect.left) / rect.width));
      const focus = pinch.range[0] + span * ratio;
      proposed = [focus - nextSpan * ratio, focus + nextSpan * (1 - ratio)];
    } else {
      if (!pan) return;
      const delta = ((point.x - pan.point.x) / rect.width) * (pan.range[1] - pan.range[0]);
      proposed = [pan.range[0] - delta, pan.range[1] - delta];
    }
    const [min, max] = clampExplorationRange(proposed, bounds, minimumSpan);
    chart.setScale('x', { min, max });
    event.preventDefault();
  };
  const events = ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'] as const;
  events.forEach(event => root.addEventListener(event, dispatch, { passive: false }));
  return () => {
    root.style.touchAction = savedTouchAction;
    events.forEach(event => root.removeEventListener(event, dispatch));
  };
}


function toSeconds(value: string | number | Date) {
  if (typeof value === 'number') return value > 10_000_000_000 ? value / 1000 : value;
  return Math.floor(new Date(value).getTime() / 1000);
}

const DATE_LABEL_POLICIES = [
  { maximum: 86400, tooltip: formatAppTime, axis: formatAppTime },
  { maximum: 3 * 86400, tooltip: (date: Date) => formatAppDateTime(date, { month: 'short', day: 'numeric' }), axis: (date: Date) => formatAppDate(date, { month: 'short', day: 'numeric', year: undefined }) },
  { maximum: 90 * 86400, tooltip: (date: Date) => formatAppDate(date, { month: 'short', day: 'numeric' }), axis: (date: Date) => formatAppDate(date, { month: 'short', day: 'numeric', year: undefined }) },
  { maximum: Infinity, tooltip: (date: Date) => formatAppDate(date, { month: 'short', year: '2-digit' }), axis: (date: Date) => formatAppDate(date, { month: 'short', year: '2-digit', day: undefined }) },
];

function dateLabel(seconds: number, span: number, target: 'tooltip' | 'axis') {
  const policy = DATE_LABEL_POLICIES.find(option => span <= option.maximum) ?? DATE_LABEL_POLICIES[3]!;
  return policy[target](new Date(seconds * 1000));
}

function formatDateForSpan(seconds: number, spanSeconds: number) {
  return dateLabel(seconds, spanSeconds, 'tooltip');
}

export function getAdaptiveDecimalPrecision(values: number[], maxPrecision = 4) {
  const unique = new Set<number>();
  for (const value of values) if (Number.isFinite(value)) unique.add(Math.abs(value) < 1e-12 ? 0 : value);
  const ordered = Array.from(unique).sort((a, b) => a - b);
  let step = Infinity;
  for (let index = 1; index < ordered.length; index += 1) {
    const difference = Math.abs(ordered[index]! - ordered[index - 1]!);
    if (difference > 1e-9) step = Math.min(step, difference);
  }
  if (!Number.isFinite(step)) return 0;
  const scaledSteps = [step];
  let precision = 0;
  for (; precision < maxPrecision; precision += 1) {
    const scaled = scaledSteps[precision]!;
    if (!(Math.abs(scaled - Math.round(scaled)) > 1e-6)) return precision;
    scaledSteps.push(scaled * 10);
  }
  return precision;
}


/** Axis labels intentionally become calendar dates for multi-day views. */
export function formatAxisDateForSpan(seconds: number, spanSeconds: number) {
  return dateLabel(seconds, spanSeconds, 'axis');
}

/** Avoid repeated date labels from uPlot's hourly automatic splits. */
export function getCalendarDateSplits(startSeconds: number, endSeconds: number, maximum = 7) {
  const duration = endSeconds - startSeconds;
  if (duration <= 86400) return undefined;
  const limit = Math.max(2, Math.floor(maximum));
  const stride = Math.max(1, Math.ceil(Math.ceil(duration / 86400) / (limit - 1)));
  const day = new Date(startSeconds * 1000);
  day.setHours(0, 0, 0, 0);
  const ticks: number[] = [];
  for (let time = day.getTime() / 1000; time <= endSeconds; time = day.getTime() / 1000) {
    if (time < startSeconds) {
      day.setDate(day.getDate() + 1);
      continue;
    }
    ticks.push(time);
    // Local date increments preserve DST; elapsed 86400-second steps do not.
    day.setDate(day.getDate() + stride);
  }
  switch (ticks.length) {
    case 0: return undefined;
    case 1: return limit >= 2 && endSeconds > ticks[0]! ? [...ticks, endSeconds] : ticks;
    default: return ticks;
  }
}

export function formatAxisDateValuesForScale(
  values: number[],
  scaleMin: number | undefined,
  scaleMax: number | undefined,
  fallbackSpan: number
) {
  const endpoints = [scaleMin, scaleMax];
  const valid = endpoints.every(value => value != null && Number.isFinite(value)) && scaleMax! > scaleMin!;
  const span = valid ? scaleMax! - scaleMin! : fallbackSpan;
  return Array.from(values, value => formatAxisDateForSpan(value, span));
}


/** Keep calendar labels readable as the plot narrows or gains a second Y axis. */
export function getResponsiveCalendarTickMaximum(plotWidth: number) {
  return Math.max(2, Math.min(7, Math.floor(plotWidth / 96)));
}

export function formatChartNumber(value: number | null | undefined, unit?: string, precision = 0) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '-';
  const format = precision > 0 ? formatNumber : formatSmartNumber;
  const parts = [format(value, precision > 0 ? precision : 0)];
  if (unit) parts.push(unit);
  return parts.join(' ');
}

/** Carry the most recent finite reading across missing samples for tooltip display. */
export function carryForwardTooltipValues(values: Array<number | null | undefined>) {
  const result: Array<number | null> = Array(values.length).fill(null);
  let start = 0;
  while (start < values.length) {
    const reading = values[start];
    if (reading == null || !Number.isFinite(reading)) { start += 1; continue; }
    let end = start + 1;
    while (end < values.length && (values[end] == null || !Number.isFinite(values[end]))) end += 1;
    result.fill(reading, start, end);
    start = end;
  }
  for (let index = 0; index < values.length; index += 1) if (!(index in values)) delete result[index];
  return result;
}


export function buildRichTooltipValues(data: AlignedData, series: RichSeries[], connectGaps: boolean) {
  return data.slice(1).map((values, index) => (
    (series[index]?.connectGaps ?? connectGaps)
      ? carryForwardTooltipValues(values as Array<number | null | undefined>)
      : Array.from(values as Array<number | null | undefined>, value => value ?? null)
  ));
}

export function getExplicitScaleConfig(range?: [number, number], extra: Omit<uPlot.Scale, 'auto' | 'range'> = {}): uPlot.Scale {
  return {
    ...extra,
    auto: !range,
    ...(range ? { range: () => range } : {}),
  };
}

/** uPlot takes bar radii as a fraction of bar width; keep the shared radius in pixels. */
export function getUPlotBarRadius(maxBarPx: number): [number, number] {
  return [Math.min(0.5, CHART_BAR_STYLE.radius / Math.max(1, maxBarPx)), 0];
}

function buildStackedBarPaths(
  priorStackIndices: number[], followingStackIndices: number[], maxBarPx: number,
): uPlot.Series.PathBuilder {
  const fallback = uPlot.paths.bars!({ size: [CHART_BAR_STYLE.slotRatio, maxBarPx], radius: 0 });
  return (self, seriesIndex, idx0, idx1) => uPlot.orient(self, seriesIndex,
    (_series, dataX, dataY, scaleX, scaleY, valToPosX, valToPosY, xOff, yOff, xDim, yDim) => {
      if (scaleX.ori !== 0) return fallback(self, seriesIndex, idx0, idx1);
      const positions = dataX.map(value => Math.round(valToPosX(value, scaleX, xDim, xOff)));
      const spacings = positions.slice(1).map((x, index) => Math.abs(x - positions[index]!)).filter(distance => distance > 0);
      const spacing = spacings.reduce((minimum, value) => Math.min(minimum, value), Infinity);
      const width = Math.max(1, Math.min(maxBarPx, Number.isFinite(spacing) ? spacing * CHART_BAR_STYLE.slotRatio : maxBarPx));
      const bases = new Float64Array(dataY.length);
      const covered = new Uint8Array(dataY.length);
      for (const index of priorStackIndices) {
        const values = self.data[index]!;
        for (let point = idx0; point <= idx1; point += 1) {
          const value = values[point];
          if (value != null && Number.isFinite(value)) bases[point]! += value;
        }
      }
      for (const index of followingStackIndices) {
        const values = self.data[index]!;
        for (let point = idx0; point <= idx1; point += 1) {
          const value = values[point];
          if (value != null && Number.isFinite(value)) covered[point] = 1;
        }
      }
      const fill = new Path2D();
      for (let index = idx0; index <= idx1; index += 1) {
        const value = dataY[index];
        if (value == null || !Number.isFinite(value)) continue;
        const start = Math.round(valToPosY(bases[index]! + value, scaleY, yDim, yOff));
        const end = Math.round(valToPosY(bases[index]!, scaleY, yDim, yOff));
        const top = Math.min(start, end);
        const bottom = Math.max(start, end);
        const left = positions[index]! - width / 2;
        const right = positions[index]! + width / 2;
        const radius = covered[index] ? 0 : Math.min(CHART_BAR_STYLE.radius, width / 2, (bottom - top) / 2);
        fill.moveTo(left, bottom);
        fill.lineTo(left, top + radius);
        if (radius > 0) fill.quadraticCurveTo(left, top, left + radius, top);
        fill.lineTo(right - radius, top);
        if (radius > 0) fill.quadraticCurveTo(right, top, right, top + radius);
        fill.lineTo(right, bottom);
        fill.closePath();
      }
      return { fill, stroke: null };
    });
}


function estimateYLabelWidth(labels: string[]): number {
  const widths = labels.map(label => label.length * 7 + 16);
  return Math.min(110, Math.max(44, ...widths));
}

export function buildRichTimeSeriesUPlotSeries(
  items: RichSeries[],
  {
    mode = 'line',
    barCount = 0,
    hiddenKeys = new Set<string>(),
    connectGaps = false,
    stepInterpolation = false,
    smoothness = DEFAULT_CURVE_SMOOTHNESS,
    showPoints = false,
    resolveColor,
  }: {
    mode?: RichTimeSeriesChartProps['mode'];
    barCount?: number;
    hiddenKeys?: ReadonlySet<string>;
    connectGaps?: boolean;
    stepInterpolation?: boolean;
    smoothness?: CurveSmoothness;
    showPoints?: boolean;
    timeFilter?: TimeFilterWindow;
    resolveColor?: (color: string) => string;
  } = {},
): Series[] {
  const stacks = new Map<string, number[]>();
  items.forEach((item, index) => {
    if (!item.stackId || (item.mode ?? mode) !== 'bar') return;
    const stack = stacks.get(item.stackId);
    if (stack) stack.push(index + 1); else stacks.set(item.stackId, [index + 1]);
  });
  const maxBarPx = barCount > 30 ? 40 : barCount > 15 ? 60 : CHART_BAR_STYLE.maxWidth;
  const curved = normalizeCurveSmoothness(smoothness) !== 'straight';
  const configured: Series[] = [{}];
  for (const [index, item] of items.entries()) {
    const token = item.color ?? (index === 0 ? CHART_COLORS.accent : CHART_COLORS.emerald);
    const color = resolveColor ? resolveColor(token) : token;
    const kind = item.mode ?? mode;
    const next: Series = {
      label: item.label,
      show: !(hiddenKeys.has(item.key) || item.tooltipOnly === true),
      stroke: color,
      scale: item.yScale ?? 'y',
      width: kind === 'scatter' ? 0 : kind === 'bar' ? 1 : item.strokeWidth ?? 2,
      points: { show: kind === 'scatter' || showPoints, size: item.pointSize ?? 6, stroke: color, fill: color },
    };
    if (item.dash) next.dash = item.dash;
    switch (kind) {
      case 'bar': {
        next.fill = color;
        const stack = item.stackId ? stacks.get(item.stackId) : undefined;
        const position = stack?.indexOf(index + 1) ?? -1;
        next.paths = stack
          ? buildStackedBarPaths(stack.slice(0, position), stack.slice(position + 1), maxBarPx)
          : uPlot.paths.bars!({ size: [CHART_BAR_STYLE.slotRatio, maxBarPx], radius: getUPlotBarRadius(maxBarPx) });
        break;
      }
      case 'scatter': next.paths = () => null; break;
      default:
        if (kind === 'area') next.fill = `${color}22`;
        if (item.connectGaps ?? connectGaps) next.spanGaps = true;
        if (item.interpolation === 'step' || stepInterpolation) next.paths = uPlot.paths.stepped!({ align: 1 });
        else if (curved && item.smoothable !== false) next.paths = NATIVE_SPLINE_PATH;
    }
    configured.push(next);
  }
  return configured;
}

export function RichTimeSeriesChart({
  rendererId = 'rich-time-series',
  points,
  series,
  height = 280,
  loading = false,
  emptyTitle = 'No chart data',
  yUnit,
  yRightUnit,
  className,
  mode = 'line',
  xTime = true,
  xUnit,
  xValueFormatter,
  xSecondaryFormatter,
  yAxisValueFormatter,
  yRightAxisValueFormatter,
  yValueFormatter,
  timeFilter = 'raw',
  smoothness = DEFAULT_CURVE_SMOOTHNESS,
  xRange,
  yRange,
  yRightRange,
  stepInterpolation = false,
  xSplits,
  cursorSyncKey,
  activeCursorIndex,
  onCursorIndexChange,
  onResolvedAxisRanges,
  connectGaps = false,
  interactionMode = 'standard',
  intervals = [],
  onIntervalClick,
  intervalBandRatio = 0.3,
  referenceLines = [],
  emptyDescription,
  xAxisLabel,
  yAxisLabel,
  yRightAxisLabel,
  showLegend = true,
  showGrid = true,
  showTooltip = true,
  showPoints = false,
}: RichTimeSeriesChartProps) {
  const rootRef = React.useRef<HTMLDivElement | null>(null);
  const chartRef = React.useRef<uPlot | null>(null);
  const legendRef = React.useRef<HTMLDivElement | null>(null);
  const [legendHeight, setLegendHeight] = React.useState(34);
  const [tooltip, setTooltip] = React.useState<{ left: number; top: number; text: string } | null>(null);
  const [intervalHover, setIntervalHover] = React.useState<string | null>(null);
  const [overlay, setOverlay] = React.useState<{
    bars: Array<PackedRichTimeInterval & { left: number; top: number; width: number; height: number; hitTop: number; hitHeight: number }>;
    lines: Array<RichReferenceLine & { left: number; top: number; width: number }>;
    intervalBand: { top: number; height: number } | null;
  }>({ bars: [], lines: [], intervalBand: null });
  const [hiddenKeys, changeHiddenKeys] = React.useReducer((current: Set<string>, action: { toggle: string } | { valid: Set<string> }) => {
    if ('toggle' in action) {
      const next = new Set(current);
      if (next.has(action.toggle)) next.delete(action.toggle); else next.add(action.toggle);
      return next;
    }
    const obsolete = [...current].filter(key => !action.valid.has(key));
    if (!obsolete.length) return current;
    const next = new Set(current);
    obsolete.forEach(key => next.delete(key));
    return next;
  }, new Set<string>());
  const [isZoomed, setIsZoomed] = React.useState(false);
  const palette = useDocumentPalette();
  const isDark = useDocumentTheme();
  const themeRevision = useThemeRevision();

  const seriesRef = React.useRef(series);
  const yUnitRef = React.useRef(yUnit);
  const yRightUnitRef = React.useRef(yRightUnit);
  const yAxisValueFormatterRef = React.useRef(yAxisValueFormatter);
  const yRightAxisValueFormatterRef = React.useRef(yRightAxisValueFormatter);
  const yValueFormatterRef = React.useRef(yValueFormatter);
  const xTimeRef = React.useRef(xTime);
  const xUnitRef = React.useRef(xUnit);
  const xValueFormatterRef = React.useRef(xValueFormatter);
  const xSecondaryFormatterRef = React.useRef(xSecondaryFormatter);
  const onCursorIndexChangeRef = React.useRef(onCursorIndexChange);
  const activeCursorIndexRef = React.useRef(activeCursorIndex);
  const applyCursorSelectionRef = React.useRef<(() => void) | null>(null);
  const controlledCursor = activeCursorIndex !== undefined;
  const onResolvedAxisRangesRef = React.useRef(onResolvedAxisRanges);
  const onIntervalClickRef = React.useRef(onIntervalClick);
  const tooltipValuesRef = React.useRef<Array<Array<number | null>>>([]);
  const tooltipDetailsRef = React.useRef<Array<Array<string | null | undefined>>>([]);
  const yPrecisionRef = React.useRef(0);
  const yRightPrecisionRef = React.useRef(0);
  const alignedDataRef = React.useRef<AlignedData>([[], []]);
  seriesRef.current = series;
  yUnitRef.current = yUnit;
  yRightUnitRef.current = yRightUnit;
  yAxisValueFormatterRef.current = yAxisValueFormatter;
  yRightAxisValueFormatterRef.current = yRightAxisValueFormatter;
  yValueFormatterRef.current = yValueFormatter;
  xTimeRef.current = xTime;
  xUnitRef.current = xUnit;
  xValueFormatterRef.current = xValueFormatter;
  xSecondaryFormatterRef.current = xSecondaryFormatter;
  onCursorIndexChangeRef.current = onCursorIndexChange;
  activeCursorIndexRef.current = activeCursorIndex;
  onResolvedAxisRangesRef.current = onResolvedAxisRanges;
  onIntervalClickRef.current = onIntervalClick;

  const packedIntervals = React.useMemo(() => packRichTimeIntervals(intervals), [intervals]);
  const laneCount = packedIntervals.reduce((max, interval) => Math.max(max, interval.lane + 1), 0);

  const alignedData = React.useMemo<AlignedData>(() => {
    return buildRichTimeSeriesAlignedData(points, series, timeFilter, xTime);
  }, [points, series, timeFilter, xTime]);
  alignedDataRef.current = alignedData;
  const tooltipValues = React.useMemo<Array<Array<number | null>>>(
    () => buildRichTooltipValues(alignedData, series, connectGaps),
    [alignedData, series, connectGaps],
  );
  tooltipValuesRef.current = tooltipValues;
  tooltipDetailsRef.current = series.map((item) => item.tooltipDetails ?? []);

  const hasData = alignedData.length > 1 && (alignedData[0]?.length ?? 0) > 0;
  const legendSeries = series.filter((item) => !item.tooltipOnly && item.showInLegend !== false);
  const legendVisible = showLegend && legendSeries.length > 0;
  const chartHeight = Math.max(120, height - (legendVisible ? legendHeight : 0));

  React.useEffect(() => {
    const legend = legendRef.current;
    if (!legend) return;
    const measure = () => setLegendHeight(legend.getBoundingClientRect().height);
    measure();
    return observeChartFrame(legend, measure);
  }, [legendVisible, loading, hasData]);
  const hiddenKeySignature = React.useMemo(() => [...hiddenKeys].sort(), [hiddenKeys]);

  const structureKey = JSON.stringify({
    frame: [chartHeight, isDark, palette, themeRevision],
    axes: [xTime, xUnit ?? '', yRightUnit ?? '', xAxisLabel ?? '', yAxisLabel ?? '', yRightAxisLabel ?? ''],
    ranges: [xRange, yRange, yRightRange, xSplits].map(range => range?.join(',') ?? ''),
    rendering: [mode, timeFilter, smoothness, stepInterpolation, Boolean(xSecondaryFormatter), showLegend, showGrid, showTooltip, showPoints, connectGaps],
    interaction: [cursorSyncKey ?? '', controlledCursor, interactionMode, intervalBandRatio],
    intervals: packedIntervals.map(item => [item.id, String(item.start), String(item.end), item.lane]),
    references: referenceLines.map(line => [line.value, line.color ?? '']),
    series: series.map(item => [item.key, item.label, item.mode ?? '', item.color ?? '', item.strokeWidth ?? '', item.yScale ?? '', item.stackId ?? '', Boolean(item.tooltipOnly)]),
    hidden: hiddenKeySignature,
  }, (_key, value: unknown) => typeof value === 'number' && !Number.isFinite(value) ? String(value) : value);

  React.useEffect(() => {
    changeHiddenKeys({ valid: new Set(series.map(item => item.key)) });
  }, [series]);

  React.useEffect(() => {
    const root = rootRef.current;
    if (!root || loading || !hasData) return undefined;

    const resolveColor = (value: string) => resolveChartColor(value, root);
    const chartMuted = resolveColor(CHART_COLORS.muted);
    const chartGrid = resolveColor(CHART_COLORS.grid);
    const width = Math.max(1, root.clientWidth || 320);

    const xValues = alignedDataRef.current[0] as number[];
    const xSpan = xValues.length > 1 ? (xValues[xValues.length - 1]! - xValues[0]!) : 86400;
    const usesCalendarDateSplits = xTime && !xSplits && xSpan > 24 * 3600;
    const fullXRange: [number, number] = xRange ?? [xValues[0]!, xValues[xValues.length - 1]!];

    const hasRightDataAxis = seriesRef.current.some((s) => !s.tooltipOnly && s.yScale === 'y2');
    const hasLeftData = seriesRef.current.some(s => !s.tooltipOnly && s.yScale !== 'y2'
      && s.values.some(value => value != null && Number.isFinite(value)));

    const isBarChart = series.some((s) => (s.mode ?? mode) === 'bar');

    const yScaleConfig: uPlot.Scale = yRange
      ? getExplicitScaleConfig(yRange)
      : isBarChart
        ? { auto: true, range: (_u: uPlot, dmin: number, dmax: number) => [Math.min(0, dmin), Math.max(0, dmax)] as [number, number] }
        : { auto: true };

    const xScaleConfig = getExplicitScaleConfig(xRange, { time: xTime });

    const rightYScaleConfig: uPlot.Scale | undefined = hasRightDataAxis
      ? getExplicitScaleConfig(yRightRange)
      : undefined;

    const xAxisConfig: uPlot.Axis = {
      stroke: chartMuted,
      grid: showGrid ? { stroke: chartGrid, width: 1 } : { show: false },
      font: `${CHART_FONT.fontWeight} ${CHART_FONT.fontSize}px ${CHART_FONT.fontFamily}`,
      size: isBarChart ? 54 : 44,
      gap: 6,
      ...(xAxisLabel ? { label: xAxisLabel, labelSize: 24, labelGap: 4 } : {}),
      ...(xSplits
        ? { splits: () => xSplits }
        : usesCalendarDateSplits
          ? {
              splits: (u, _axisIdx, scaleMin, scaleMax) =>
                getCalendarDateSplits(
                  scaleMin,
                  scaleMax,
                  getResponsiveCalendarTickMaximum(u.bbox.width / uPlot.pxRatio)
                ) ?? [],
            }
        : {}),
      values: (u, vals) => {
        const custom = xValueFormatterRef.current;
        const numeric = (value: number) => formatChartNumber(value, xUnitRef.current, 0);
        if (custom || !xTimeRef.current) return vals.map(value => (custom ?? numeric)(value));
        return formatAxisDateValuesForScale(vals, u.scales.x?.min, u.scales.x?.max, xSpan);
      },
    };

    const xSecondaryAxisConfig: uPlot.Axis | null = xSecondaryFormatterRef.current
      ? {
          scale: 'x', // share the primary X scale
          side: 0,    // top
          stroke: chartMuted,
          grid: { show: false }, // avoid duplicate grid lines
          font: `${CHART_FONT.fontWeight} ${CHART_FONT.fontSize}px ${CHART_FONT.fontFamily}`,
          size: 40,
          gap: 6,
          splits: (u, _axisIdx, scaleMin, scaleMax) => {
            const count = getResponsiveCalendarTickMaximum(u.bbox.width / uPlot.pxRatio);
            const step = (scaleMax - scaleMin) / (count - 1);
            return Array.from({ length: count }, (_, i) => scaleMin + i * step);
          },
          values: (_u, vals) =>
            vals.map((v) => xSecondaryFormatterRef.current!(v)),
        }
      : null;

    const axisValues = (side: 'y' | 'y2', values: number[]) => {
      const precision = getAdaptiveDecimalPrecision(values);
      const right = side === 'y2';
      (right ? yRightPrecisionRef : yPrecisionRef).current = precision;
      const unit = (right ? yRightUnitRef : yUnitRef).current;
      const format = (right ? yRightAxisValueFormatterRef : yAxisValueFormatterRef).current ?? yValueFormatterRef.current;
      return values.map(value => format ? format(value, unit) : formatChartNumber(value, unit, precision));
    };

    const yAxisConfig: uPlot.Axis = {
      stroke: chartMuted,
      grid: showGrid ? { stroke: chartGrid, width: 1 } : { show: false },
      font: `${CHART_FONT.fontWeight} ${CHART_FONT.fontSize}px ${CHART_FONT.fontFamily}`,
      size: (_self, values) => {
        if (!values || values.length === 0) return 50;
        return estimateYLabelWidth(values as string[]);
      },
      gap: 8,
      ...(yAxisLabel ? { label: yAxisLabel, labelSize: 24, labelGap: 4 } : {}),
      values: (_u, vals) => axisValues('y', vals),
    };

    // Right Y axis — only added when at least one series uses scale 'y2'.
    const rightYAxisConfig: uPlot.Axis | null = hasRightDataAxis
      ? {
          scale: 'y2',
          side: 1, // right
          stroke: chartMuted,
          grid: { show: false },
          font: `${CHART_FONT.fontWeight} ${CHART_FONT.fontSize}px ${CHART_FONT.fontFamily}`,
          // uPlot supplies raw numeric splits here rather than our formatted
          // labels, so reserve enough room for values such as "19,200 mi".
          size: 86,
          gap: 8,
          ...(yRightAxisLabel ? { label: yRightAxisLabel, labelSize: 24, labelGap: 4 } : {}),
          values: (_u, vals) => axisValues('y2', vals),
        }
      : null;

    // When a secondary top axis is present we need extra top padding so its
    // labels are not clipped; otherwise keep the default small gutter.
    const topPadding = xSecondaryAxisConfig ? 44 : 10;

    const allAxes: uPlot.Axis[] = [xAxisConfig, yAxisConfig];
    if (xSecondaryAxisConfig) allAxes.push(xSecondaryAxisConfig);
    if (rightYAxisConfig) allAxes.push(rightYAxisConfig);

    const updateOverlay = () => {
      const chart = chartRef.current;
      if (!chart || !chart.bbox) return;
      const plotLeft = chart.bbox.left;
      const plotTop = chart.bbox.top;
      const plotWidth = chart.bbox.width;
      const plotHeight = chart.bbox.height;
      const lanes = new Map<number, RichTimeIntervalBandGeometry>();
      if (packedIntervals.length) lanes.set(0, getRichTimeIntervalBandGeometry(0, laneCount, plotTop, plotHeight, intervalBandRatio));
      const bars = [];
      for (const interval of packedIntervals) {
        let geometry = lanes.get(interval.lane);
        if (!geometry) {
          geometry = getRichTimeIntervalBandGeometry(interval.lane, laneCount, plotTop, plotHeight, intervalBandRatio);
          lanes.set(interval.lane, geometry);
        }
        const left = plotLeft + chart.valToPos(toSeconds(interval.start), 'x');
        const right = plotLeft + chart.valToPos(toSeconds(interval.end), 'x');
        bars.push({ ...interval, left: Math.min(left, right), width: Math.max(2, Math.abs(right - left)),
          top: geometry.top, height: geometry.height, hitTop: geometry.hitTop, hitHeight: geometry.hitHeight });
      }
      const intervalBand = lanes.get(0);
      const lines = referenceLines.map(line => ({ ...line, left: plotLeft, top: plotTop + chart.valToPos(line.value, 'y'), width: plotWidth }));
      setOverlay({
        bars,
        lines,
        intervalBand: intervalBand ? { top: intervalBand.bandTop, height: intervalBand.bandHeight } : null,
      });
    };

    const cursorController = createRichCursorController({
      controlled: controlledCursor, showTooltip,
      selectedValue: () => {
        const index = activeCursorIndexRef.current;
        return index == null ? undefined : alignedDataRef.current[0]?.[index];
      },
      onIndex: index => onCursorIndexChangeRef.current?.(index),
      clear: () => setTooltip(null),
      render: (u, idx) => {
            const data = alignedDataRef.current;
            const currentSeries = seriesRef.current;
            const timestamp = data[0]?.[idx];
            const rows: string[] = [];
            const axes = {
              y: { unit: yUnitRef.current, precision: yPrecisionRef.current },
              y2: { unit: yRightUnitRef.current, precision: yRightPrecisionRef.current },
            };
            for (const [seriesIndex, item] of currentSeries.entries()) {
              if (hiddenKeys.has(item.key)) continue;
              const value = tooltipValuesRef.current[seriesIndex]?.[idx] ?? null;
              const axis = axes[item.yScale ?? 'y'];
              const format = yValueFormatterRef.current;
              const text = item.tooltipFormatter ? item.tooltipFormatter(value) : format ? format(value, axis.unit) : formatChartNumber(value, axis.unit, axis.precision);
              const detail = tooltipDetailsRef.current[seriesIndex]?.[idx];
              rows.push(`${item.label}: ${text}${detail ? ` (${detail})` : ''}`);
            }

            let tooltipHeader = '';
            let tooltipSubHeader = '';
            if (timestamp != null) {
              if (xValueFormatterRef.current) {
                tooltipHeader = xValueFormatterRef.current(timestamp as number);
              } else if (xTimeRef.current) {
                // Use actual data span so tooltip granularity matches axis labels.
                const xs = u.data[0] as number[];
                const span = xs.length > 1 ? xs[xs.length - 1]! - xs[0]! : 86400;
                tooltipHeader = formatDateForSpan(timestamp as number, span);
              } else {
                tooltipHeader = formatChartNumber(timestamp as number, xUnitRef.current, 0);
              }
              if (xSecondaryFormatterRef.current) {
                tooltipSubHeader = xSecondaryFormatterRef.current(timestamp as number);
              }
            }

            const tooltipLineCount = rows.length + (tooltipHeader ? 1 : 0) + (tooltipSubHeader ? 1 : 0);
            setTooltip({
              left: Math.min(Math.max((u.cursor.left ?? 0) + 16, 12), Math.max(12, u.width - 180)),
              top: Math.max(12, Math.min((u.cursor.top ?? 0) + 12, u.height - tooltipLineCount * 18 - 16)),
              text: [tooltipHeader, tooltipSubHeader, ...rows].filter(Boolean).join('\n'),
            });
      },
    });
    const opts: Options = {
      width,
      height: chartHeight,
      data: alignedDataRef.current,
      // Keep a small left gutter so y-axis labels are not clipped at narrow widths.
      // Right gutter grows when a right axis is present so its labels aren't clipped.
      padding: [topPadding, hasRightDataAxis ? 4 : 24, 0, hasLeftData ? 10 : 24],
      cursor: {
        drag: { x: !controlledCursor && interactionMode !== 'touch-explore', y: false },
        points: { size: 6 },
        ...(controlledCursor ? { bind: {
          mousemove: () => null, mouseleave: () => null,
          mousedown: () => null, mouseup: () => null, dblclick: () => null,
        } } : {}),
        ...(cursorSyncKey && !controlledCursor ? { sync: { key: cursorSyncKey, scales: ['x', null] } } : {}),
      },
      legend: { show: false },
      scales: {
        x: xScaleConfig,
        y: yScaleConfig,
        ...(hasRightDataAxis && rightYScaleConfig ? { y2: rightYScaleConfig } : {}),
      },
      axes: allAxes,
      series: buildRichTimeSeriesUPlotSeries(seriesRef.current, {
        mode,
        barCount: xValues.length,
        hiddenKeys,
        connectGaps,
        stepInterpolation,
        smoothness: normalizeCurveSmoothness(smoothness),
        showPoints,
        timeFilter,
        resolveColor,
      }),
      hooks: {
        setScale: [
          (u, key) => {
            const limits = key === 'x' ? u.scales.x : undefined;
            const domain = limits?.min == null || limits.max == null ? null : [limits.min, limits.max] as [number, number];
            if (domain) {
              setIsZoomed(isZoomedXRange(domain, fullXRange));
              requestAnimationFrame(updateOverlay);
            }
          },
        ],
        setCursor: [cursorController.read],
      },
    };

    chartRef.current = new uPlot(opts, alignedDataRef.current, root);
    setIsZoomed(false);

    const chart = chartRef.current;
    applyCursorSelectionRef.current = () => cursorController.apply(chart);
    const resolved: { y?: [number, number]; y2?: [number, number] } = {};
    for (const side of ['y', 'y2'] as const) {
      const limits = [chart.scales[side]?.min, chart.scales[side]?.max];
      if (limits.every(value => value != null && Number.isFinite(value))) resolved[side] = limits as [number, number];
    }
    onResolvedAxisRangesRef.current?.(resolved);
    requestAnimationFrame(updateOverlay);
    const touchCleanup = controlledCursor
      ? attachChartScrubbing(chart, (index) => onCursorIndexChangeRef.current?.(index))
      : interactionMode === 'touch-explore'
      ? attachTouchExploration(root, chart, fullXRange)
      : attachChartScrubbing(chart, () => {}, true);

    const stopObserving = typeof ResizeObserver === 'undefined' ? undefined : observeChartFrame(root, () => {
      chart.setSize({ width: Math.max(1, root.clientWidth || 320), height: chartHeight });
      cursorController.apply(chart);
      updateOverlay();
    });

    return () => {
      stopObserving?.();
      touchCleanup?.();
      applyCursorSelectionRef.current = null;
      chart.destroy();
      chartRef.current = null;
      setTooltip(null);
      setIntervalHover(null);
      setOverlay({ bars: [], lines: [], intervalBand: null });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [structureKey, loading, hasData]);

  React.useEffect(() => {
    if (chartRef.current && !loading && hasData) {
      chartRef.current.setData(alignedData);
    }
  }, [alignedData, loading, hasData]);

  React.useEffect(() => {
    applyCursorSelectionRef.current?.();
  }, [activeCursorIndex, alignedData, structureKey, loading, hasData]);

  const intervalViews = overlay.bars.map(interval => ({
    interval,
    active: intervalHover === interval.id,
    color: interval.color ?? CHART_COLORS.violet,
    accessibleLabel: [interval.label, ...(interval.details ? [interval.details] : [])].join(': '),
    title: [interval.label, ...(interval.details ? [interval.details] : [])].join(' — '),
    tooltipText: interval.label + (interval.details ? `\n${interval.details}` : ''),
  }));
  const activeInterval = intervalHover ? intervalViews.find(view => view.interval.id === intervalHover) : undefined;
  const resetZoom = () => {
    const values = alignedDataRef.current[0] as number[];
    const available = values.length >= 2 ? [values[0]!, values[values.length - 1]!] : null;
    const range = available && (xRange ?? available);
    if (range) chartRef.current?.setScale('x', { min: range[0]!, max: range[1]! });
  };

  if (loading) return <ChartSkeleton height={height} />;

  if (!hasData) {
    return (
      <div
        data-chart-renderer={rendererId}
        className={cn(
          'flex items-center justify-center rounded-lg border border-border bg-bg-elevated/50 text-sm text-fg-tertiary',
          className,
        )}
        style={{ height }}
      >
        {emptyTitle}
        {emptyDescription ? <span className="mt-1 block text-xs text-fg-tertiary">{emptyDescription}</span> : null}
      </div>
    );
  }

  return (
    <div
      data-chart-renderer={rendererId}
      className={cn(
        'relative flex min-w-0 flex-col overflow-hidden rounded-lg border border-border',
        interactionMode === 'touch-explore' ? 'bg-bg-surface shadow-xl' : 'bg-bg-elevated/40',
        className,
      )}
      style={{ height }}
    >
      <div className="relative min-h-0 flex-1" style={{ height: chartHeight }}>
        {intervalViews.map(({ interval, active: isActive, color }) => {
          return (
            <span
              key={`interval-visual-${interval.id}`}
              aria-hidden="true"
              className="pointer-events-none absolute z-0 border"
              style={{
                left: interval.left,
                top: interval.top,
                width: interval.width,
                height: interval.height,
                borderColor: color,
                borderRadius: CHART_BAR_STYLE.radius,
                backgroundColor: color,
                opacity: isActive ? CHART_BAR_STYLE.activeOpacity : CHART_BAR_STYLE.fillOpacity,
              }}
            />
          );
        })}
        <div
          ref={rootRef}
          className="relative z-10 rich-uplot-chart h-full w-full"
          data-chart-smoothness={smoothness}
        />
        {overlay.lines.map((line) => (
          <div
            key={`reference-${line.value}-${line.label ?? ''}`}
            className="pointer-events-none absolute z-10 border-t border-dashed"
            style={{
              left: line.left,
              top: line.top,
              width: line.width,
              borderColor: line.color ?? CHART_COLORS.muted,
            }}
            aria-hidden="true"
          />
        ))}
        {overlay.lines.map((line) => line.label ? (
          <span
            key={`reference-label-${line.value}-${line.label}`}
            className="pointer-events-none absolute z-10 -translate-y-full rounded-sm bg-bg-surface/80 px-1 text-[10px] text-fg-tertiary"
            style={{ left: Math.max(8, line.left + line.width - 58), top: line.top - 2 }}
          >
            {line.label}
          </span>
        ) : null)}
        {overlay.intervalBand && yRightUnit ? (
          <span
            className="pointer-events-none absolute right-2 z-10 rounded-sm bg-bg-surface/80 px-1 text-[10px] text-fg-tertiary"
            style={{ top: overlay.intervalBand.top + 4 }}
          >
            {yRightUnit}
          </span>
        ) : null}
        {intervalViews.map(({ interval, active: isActive, accessibleLabel, title }) => {
          return (
            <button
              key={interval.id}
              type="button"
              className={cn(
                'absolute z-20 overflow-visible border-0 bg-transparent p-0 text-left transition-[filter,opacity,transform] focus:outline-none focus:ring-2 focus:ring-accent focus:ring-offset-1 focus:ring-offset-bg-surface',
                isActive ? 'brightness-110' : 'hover:brightness-110',
              )}
              style={{
                left: interval.left,
                top: interval.hitTop,
                width: interval.width,
                height: interval.hitHeight,
                minWidth: 12,
              }}
              aria-label={accessibleLabel}
              title={title}
              onMouseEnter={() => setIntervalHover(interval.id)}
              onMouseLeave={() => setIntervalHover(null)}
              onFocus={() => setIntervalHover(interval.id)}
              onBlur={() => setIntervalHover(null)}
              onClick={(event) => {
                event.stopPropagation();
                onIntervalClickRef.current?.(interval);
              }}
            >
              <span className="sr-only">{interval.label}</span>
            </button>
          );
        })}
        {activeInterval ? (
          <div
            className="pointer-events-none absolute z-30 max-w-[min(22rem,calc(100%-1rem))] whitespace-pre-wrap rounded-md border border-border-strong bg-bg-surface/95 px-2.5 py-1.5 text-[11px] leading-[1.5] text-fg shadow-xl"
            style={{ left: Math.min(activeInterval.interval.left, Math.max(8, (rootRef.current?.clientWidth ?? 320) - 180)), top: Math.max(8, activeInterval.interval.top - 48) }}
          >
            {activeInterval.tooltipText}
          </div>
        ) : null}
      </div>
      {isZoomed ? (
        <button
          type="button"
          onClick={resetZoom}
          className="absolute right-3 top-3 z-20 flex h-11 w-11 sm:h-8 sm:w-8 items-center justify-center rounded-md border border-border bg-bg-surface/95 text-fg-secondary shadow-sm transition-colors hover:border-border-strong hover:text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent"
          aria-label="Return to full chart view"
          title="Return to full chart view"
        >
          <RotateCcw className="h-4 w-4" aria-hidden="true" />
        </button>
      ) : null}
      {tooltip ? (
        <div
          role="tooltip"
          className="pointer-events-none absolute z-30 whitespace-pre-wrap break-words rounded-md border border-border-strong bg-bg-surface px-2.5 py-1.5 text-xs leading-[1.5] text-fg shadow-xl"
          style={{ left: tooltip.left, top: tooltip.top, maxWidth: `calc(100% - ${tooltip.left + 8}px)` }}
        >
          {tooltip.text}
        </div>
      ) : null}
      {legendVisible ? (
        <div ref={legendRef} className="flex min-h-[34px] max-h-[88px] shrink-0 flex-wrap items-center justify-center gap-x-3 overflow-y-auto border-t border-border/60 px-3 text-[11px] text-fg-tertiary">
          {legendSeries.map((item, index) => {
            const color = item.color ?? (index === 0 ? CHART_COLORS.accent : CHART_COLORS.emerald);
            const isHidden = hiddenKeys.has(item.key);
            return (
              <button
                key={item.key}
                type="button"
                onClick={() => {
                  changeHiddenKeys({ toggle: item.key });
                }}
                className={cn(
                  'flex min-h-11 sm:min-h-[32px] max-w-full items-center gap-1.5 rounded-md px-1.5 py-1 transition hover:bg-bg',
                  isHidden && 'opacity-45',
                )}
              >
                <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: color }} />
                <span>{item.label}</span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

import type { DailyChargeSessionsDay, DailyChargeSessionsSession } from './dailyChargingTypes';
export type { DailyChargeSessionsDay, DailyChargeSessionsSession } from './dailyChargingTypes';
import { prepareChargingDays, chargingDayGeometry, GROUP_ORDER, GROUP_META, type ChargerGroupKey, type PreparedDay } from './dailyChargingModel';
import * as React from 'react';
import { RotateCcw } from 'lucide-react';
import { ChartSkeleton } from '../primitives/Skeleton';
import { CHART_BAR_STYLE, CHART_COLORS, CHART_FONT } from './ChartProvider';
import { formatCurrency, formatSmartNumber } from '../lib/utils';
import { formatAppCalendarDate } from '../lib/dateTime';
import { useChartWidth } from './useChartWidth';
import {
  measureChartText,
  selectAdaptiveAxisLabelIndices,
  selectValueLabelIndices,
} from './chartLabelLayout';

export interface DailyChargeSessionsChartProps {
  daily: DailyChargeSessionsDay[];
  dailySessions: DailyChargeSessionsSession[];
  height?: number;
  loading?: boolean;
  emptyTitle?: string | undefined;
  selectedDayLocal?: string | null;
  onDayClick?: (dayLocal: string | null) => void;
  /** Enables touch-safe range selection in the expanded chart viewer. */
  interactionMode?: 'standard' | 'touch-explore' | undefined;
  /** Optional definition-driven color. Omitted preserves the established palette. */
  seriesColor?: string | undefined;
  /** Optional definition-driven fixed range. Omitted preserves automatic scaling. */
  yRange?: [number, number] | undefined;
}

export interface DailyEnergyBarChartProps {
  daily: DailyChargeSessionsDay[];
  height?: number;
  loading?: boolean;
  emptyTitle?: string | undefined;
  yRange?: [number, number] | undefined;
  interactionMode?: 'standard' | 'touch-explore' | undefined;
  seriesColor?: string | undefined;
}

export interface DailyChargingBarChartProps extends Omit<
  DailyChargeSessionsChartProps,
  'dailySessions'
> {
  dailySessions?: DailyChargeSessionsSession[];
  variant: 'total' | 'stacked';
  yRange?: [number, number] | undefined;
}

interface HoverState {
  dayLocal: string;
  x: number;
  y: number;
  containerWidth: number;
  containerHeight: number;
}

function formatDayLabel(dayLocal: string) {
  const formatted = formatAppCalendarDate(dayLocal);
  return formatted === 'Invalid date' ? dayLocal : formatted.replace(/, \d{4}$/, '');
}

function formatEnergy(value: number) {
  return `${formatSmartNumber(value, Math.abs(value) >= 100 ? 0 : 1)} kWh`;
}

function formatSessionCount(count: number) {
  return `${count} session${count === 1 ? '' : 's'}`;
}

function nextAxisMax(value: number) {
  if (value <= 0) return 1;
  return Math.ceil(value / 100) * 100;
}

export function DailyChargingBarChart({
  daily,
  dailySessions = [],
  height = 280,
  loading = false,
  emptyTitle = 'No charging sessions for this period',
  selectedDayLocal,
  onDayClick,
  variant,
  yRange,
  interactionMode = 'standard',
  seriesColor,
}: DailyChargingBarChartProps) {
  const chartRef = React.useRef<HTMLDivElement | null>(null);
  const { width, ref: measureWidth } = useChartWidth();
  const setChartRef = React.useCallback((element: HTMLDivElement | null) => {
    chartRef.current = element;
    measureWidth(element);
  }, [measureWidth]);
  const selectionRef = React.useRef<{ pointerId: number; start: number; current: number } | null>(
    null
  );
  const suppressDayClickRef = React.useRef(false);
  const [hoverState, updateHover] = React.useReducer((current: HoverState | null, event: HoverState | string) => {
    if (typeof event !== 'string') return event;
    return current?.dayLocal === event ? null : current;
  }, null);
  const [zoomRange, setZoomRange] = React.useReducer((current: [number, number] | null, action: [number, number] | number | null) => {
    if (typeof action !== 'number') return action;
    if (!current || action < 2) return null;
    const end = Math.min(current[1], action - 1);
    const start = Math.min(current[0], action - 1);
    return start < end ? [start, end] as [number, number] : null;
  }, null);
  const [selection, setSelection] = React.useState<[number, number] | null>(null);
  const clipPathPrefix = React.useId().replace(/:/g, '');

  const days = React.useMemo<PreparedDay[]>(
    () => prepareChargingDays(daily, dailySessions, seriesColor), [daily, dailySessions],
  );

  React.useEffect(() => { setZoomRange(days.length); }, [days.length]);

  const visibleStartIndex = zoomRange?.[0] ?? 0;
  const visibleDays = zoomRange ? days.slice(zoomRange[0], zoomRange[1] + 1) : days;
  const isZoomed = zoomRange != null;

  const legendItems = React.useMemo(() => {
    if (variant !== 'stacked') return [];
    const counts = new Map<ChargerGroupKey, number>();
    days.forEach(day => day.groups.forEach(group => counts.set(group.key, (counts.get(group.key) ?? 0) + 1)));
    return GROUP_ORDER.flatMap(key => counts.has(key) ? [GROUP_META[key]] : []);
  }, [days, variant]);

  const dayLookup = new Map<string, PreparedDay>();
  for (const day of visibleDays) if (!dayLookup.has(day.day_local)) dayLookup.set(day.day_local, day);
  const activeDay = hoverState ? dayLookup.get(hoverState.dayLocal) ?? null : null;
  const setDayHoverFromEvent = (dayLocal: string, event: React.MouseEvent<SVGElement> | React.PointerEvent<SVGElement>) => {
    const rect = chartRef.current?.getBoundingClientRect();
    if (rect) updateHover({ dayLocal, x: event.clientX - rect.left, y: event.clientY - rect.top, containerWidth: rect.width, containerHeight: rect.height });
  };
  const isDaySelected = (dayLocal: string) => selectedDayLocal != null && selectedDayLocal === dayLocal;
  const handleDaySelect = (dayLocal: string) => {
    const suppressed = suppressDayClickRef.current;
    suppressDayClickRef.current = false;
    if (!suppressed) onDayClick?.(dayLocal === selectedDayLocal ? null : dayLocal);
  };

  if (loading) {
    return <ChartSkeleton height={height} />;
  }

  if (days.length === 0) {
    return (
      <div
        className="flex items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary"
        style={{ height }}
      >
        {emptyTitle}
      </div>
    );
  }

  const chartHeight = Math.max(160, height - 26);
  const margin = width < 480
    ? { top: 36, right: 12, bottom: 48, left: 64 }
    : { top: 20, right: 24, bottom: 64, left: 70 };
  const innerWidth = width - margin.left - margin.right;
  const innerHeight = chartHeight - margin.top - margin.bottom;
  const maxEnergy = Math.max(1, ...visibleDays.map((day) => day.totalEnergyKwh));
  const axisMinEnergy = Math.min(0, yRange?.[0] ?? 0);
  const axisMaxEnergy = Math.max(
    axisMinEnergy + 1,
    maxEnergy,
    yRange?.[1] ?? nextAxisMax(maxEnergy)
  );
  const axisSpan = axisMaxEnergy - axisMinEnergy;
  const topPaddingPx = 8;
  const renderHeight = Math.max(1, innerHeight - topPaddingPx);
  const slotWidth = innerWidth / Math.max(visibleDays.length, 1);
  const barWidth = Math.min(CHART_BAR_STYLE.maxWidth, slotWidth * CHART_BAR_STYLE.slotRatio);
  const geometry = chargingDayGeometry(visibleDays, margin.left, margin.top, innerHeight, slotWidth, barWidth, axisSpan, renderHeight);
  const yTicks = [0, 0.25, 0.5, 0.75, 1];
  const tooltipHeight = activeDay
    ? variant === 'stacked'
      ? 96 +
        activeDay.groups.length * 28 +
        activeDay.groups.reduce((sum, group) => sum + Math.max(0, group.sessions.length - 1) * 6, 0)
      : 74
    : 0;
  const tooltipWidth = 288;
  const selectDayAt = (clientX: number, bounds: DOMRect) => {
    const localX = ((clientX - bounds.left) / bounds.width) * width;
    const projected = Math.floor((localX - margin.left) / slotWidth);
    return bounds.width > 0 && Number.isFinite(localX)
      ? Math.max(0, Math.min(visibleDays.length - 1, projected)) : null;
  };
  const selectionBounds = selection
    ? ([Math.min(selection[0], selection[1]), Math.max(selection[0], selection[1])] as [
        number,
        number,
      ])
    : null;
  const selectionX = selectionBounds ? margin.left + selectionBounds[0] * slotWidth : 0;
  const selectionWidth = selectionBounds
    ? (selectionBounds[1] - selectionBounds[0] + 1) * slotWidth
    : 0;
  const axisLabelIndices = selectAdaptiveAxisLabelIndices(
    geometry.map(day => formatDayLabel(day.day_local)), geometry.map(day => day.labelX),
  );
  const valueLabels = geometry.map((day, index) => ({
    index, value: day.totalEnergyKwh, x: day.labelX,
    y: Math.max(margin.top + 10, day.totalLabelY),
    width: measureChartText(formatSmartNumber(day.totalEnergyKwh, day.totalEnergyKwh >= 100 ? 0 : 1)),
  }));
  const valueLabelIndices = selectValueLabelIndices(valueLabels);
  const valueLabelIndexSet = new Set(valueLabelIndices);
  const axisLabelIndexSet = new Set(axisLabelIndices);

  const chartPointer = (event: React.PointerEvent<SVGSVGElement>) => {
    if (event.type === 'pointercancel') { selectionRef.current = null; setSelection(null); return; }
    const index = selectDayAt(event.clientX, event.currentTarget.getBoundingClientRect());
    if (event.type === 'pointerdown') {
      if (visibleDays.length < 2 || index == null) return;
      selectionRef.current = { pointerId: event.pointerId, start: index, current: index };
      setSelection([index, index]);
      event.currentTarget.setPointerCapture?.(event.pointerId);
      return;
    }
    const active = selectionRef.current;
    if (!active || active.pointerId !== event.pointerId) return;
    if (event.type === 'pointermove') {
      if (index == null || index === active.current) return;
      selectionRef.current = { ...active, current: index };
      setSelection([active.start, index]);
      event.preventDefault();
      return;
    }
    selectionRef.current = null;
    setSelection(null);
    const end = index ?? active.current;
    if (Math.abs(end - active.start) < 1) return;
    const selected: [number, number] = [Math.min(active.start, end), Math.max(active.start, end)];
    suppressDayClickRef.current = true;
    setZoomRange([visibleStartIndex + selected[0], visibleStartIndex + selected[1]]);
  };

  return (
    <div
      ref={setChartRef}
      data-chart-renderer={variant === 'total' ? 'daily-energy-bars' : 'daily-charge-sessions'}
      className="relative rounded-lg border border-border bg-bg-surface p-3 shadow-[inset_0_-1px_0_var(--rm-border)]"
    >
      {isZoomed ? (
        <button
          type="button"
          onClick={() => setZoomRange(null)}
          className="absolute right-3 top-3 z-30 flex h-8 w-8 items-center justify-center rounded-md border border-border bg-bg-surface/95 text-fg-secondary shadow-sm transition-colors hover:border-border-strong hover:text-fg focus:outline-none focus:ring-1 focus:ring-accent"
          aria-label="Return to full chart view"
          title="Return to full chart view"
        >
          <RotateCcw className="h-4 w-4" aria-hidden="true" />
        </button>
      ) : null}
      {legendItems.length > 0 ? (
        <div className="pointer-events-none absolute right-14 top-3 z-20 flex flex-wrap justify-end gap-2 rounded-full border border-border bg-bg-surface/95 px-2 py-1 text-[11px] text-fg-secondary shadow-sm backdrop-blur">
          {legendItems.map((item) => (
            <span key={item.key} className="inline-flex items-center gap-1.5 whitespace-nowrap">
              <span
                className="h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: item.color }}
                aria-hidden="true"
              />
              <span>{item.label}</span>
            </span>
          ))}
        </div>
      ) : null}

      {activeDay && hoverState ? (
        <div
          role="tooltip"
          aria-live="polite"
          className="pointer-events-none absolute z-30 w-72 rounded-lg border border-border bg-bg-surface/95 px-3 py-2 text-[11px] text-fg shadow-lg backdrop-blur"
          style={{
            left: Math.min(
              Math.max(hoverState.x + 14, 8),
              Math.max(8, hoverState.containerWidth - tooltipWidth - 8)
            ),
            top: Math.min(
              Math.max(hoverState.y + 14, 8),
              Math.max(8, hoverState.containerHeight - tooltipHeight - 8)
            ),
          }}
        >
          <div className="mb-1 font-medium text-fg">{formatDayLabel(activeDay.day_local)}</div>
          {variant === 'stacked' ? (
            <>
              <div className="mb-2 text-fg-tertiary">
                {formatEnergy(activeDay.totalEnergyKwh)} total,{' '}
                {formatSessionCount(
                  activeDay.groups.reduce((sum, group) => sum + group.sessionCount, 0)
                )}
              </div>
              <div className="space-y-1">
                {activeDay.groups.map((group) => {
                  return (
                    <div key={group.key} className="flex items-start gap-2">
                      <span
                        className="mt-1 h-2.5 w-2.5 shrink-0 rounded-full"
                        style={{ backgroundColor: group.color }}
                        aria-hidden="true"
                      />
                      <div className="min-w-0">
                        <div className="text-fg">
                          <span className="font-medium text-fg">{group.label}</span>
                          {group.costUsd != null ? (
                            <span className="text-fg-tertiary">
                              {' '}
                              · {formatCurrency(group.costUsd)}
                            </span>
                          ) : null}
                          <span className="text-fg-tertiary"> {formatEnergy(group.energyKwh)}</span>
                        </div>
                        <div className="text-fg-tertiary">
                          {formatSessionCount(group.sessionCount)}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </>
          ) : (
            <div className="text-fg-tertiary">
              Energy Charged: {formatEnergy(activeDay.totalEnergyKwh)}
            </div>
          )}
        </div>
      ) : null}

      <svg
        aria-label={variant === 'stacked' ? 'Daily charge sessions' : 'Daily energy charged'}
        role="img"
        viewBox={`0 0 ${width} ${chartHeight}`}
        width="100%"
        height={chartHeight}
        data-testid={variant === 'stacked' ? 'daily-charge-sessions-chart' : 'daily-energy-chart'}
        className={interactionMode === 'touch-explore' ? 'touch-none' : undefined}
        onPointerDown={chartPointer}
        onPointerMove={chartPointer}
        onPointerUp={chartPointer}
        onPointerCancel={chartPointer}
      >
        <defs>
          {geometry.map((day, dayIndex) => {
            const { x, y, barHeight } = day;
            if (barHeight <= 0) return null;
            const clipId = `${clipPathPrefix}-daily-charge-stack-${dayIndex}`;
            return (
              <clipPath key={clipId} id={clipId}>
                <rect x={x} y={y} width={barWidth} height={barHeight} rx={8} ry={8} />
              </clipPath>
            );
          })}
        </defs>

        {yTicks.map((fraction) => {
          const y = margin.top + innerHeight - fraction * innerHeight;
          const value = axisMinEnergy + axisSpan * fraction;
          return (
            <g key={`grid-${fraction}`}>
              <line
                x1={margin.left}
                y1={y}
                x2={width - margin.right}
                y2={y}
                stroke={CHART_COLORS.grid}
                strokeWidth={1}
              />
              <text
                x={margin.left - 12}
                y={y + 4}
                textAnchor="end"
                fill={CHART_COLORS.muted}
                fontFamily={CHART_FONT.fontFamily}
                fontSize={CHART_FONT.fontSize}
                fontWeight={CHART_FONT.fontWeight}
              >
                {formatSmartNumber(value, value >= 100 ? 0 : 1)}
              </text>
            </g>
          );
        })}

        {geometry.map((day, dayIndex) => {
          const { x, clippedBarHeight, totalLabelY, sessionCount } = day;
          const clipId = `${clipPathPrefix}-daily-charge-stack-${dayIndex}`;
          const isActive = hoverState?.dayLocal === day.day_local;
          const isSelected = isDaySelected(day.day_local);

          return (
            <g key={day.day_local}>
              <g data-testid="daily-charge-stack">
                <rect
                  x={x}
                  y={margin.top + innerHeight - clippedBarHeight}
                  width={barWidth}
                  height={clippedBarHeight}
                  fill="transparent"
                  pointerEvents="all"
                  role="button"
                  tabIndex={0}
                  onPointerEnter={(event) => setDayHoverFromEvent(day.day_local, event)}
                  onPointerMove={(event) => setDayHoverFromEvent(day.day_local, event)}
                  onPointerDown={(event) => setDayHoverFromEvent(day.day_local, event)}
                  onPointerLeave={() =>
                    updateHover(day.day_local)
                  }
                  onMouseEnter={(event) => setDayHoverFromEvent(day.day_local, event)}
                  onMouseMove={(event) => setDayHoverFromEvent(day.day_local, event)}
                  onMouseLeave={() =>
                    updateHover(day.day_local)
                  }
                  onClick={() => handleDaySelect(day.day_local)}
                  onKeyDown={(event) => {
                    if (event.key !== 'Enter' && event.key !== ' ') return;
                    event.preventDefault();
                    handleDaySelect(day.day_local);
                  }}
                  aria-label={
                    variant === 'stacked'
                      ? `${formatDayLabel(day.day_local)}: ${formatEnergy(day.totalEnergyKwh)} across ${formatSessionCount(sessionCount)}`
                      : `${formatDayLabel(day.day_local)}: ${formatEnergy(day.totalEnergyKwh)} energy charged`
                  }
                  className={onDayClick ? 'cursor-pointer' : undefined}
                />
                <g
                  clipPath={`url(#${clipId})`}
                  opacity={
                    isActive || isSelected
                      ? CHART_BAR_STYLE.activeOpacity
                      : CHART_BAR_STYLE.fillOpacity
                  }
                >
                  {variant === 'stacked' ? (
                    day.segments.map(({ group, y, height: segmentHeight }) => {
                      if (segmentHeight <= 0) return null;

                      return (
                        <rect
                          key={`${day.day_local}-${group.key}`}
                          data-testid="daily-charge-segment"
                          data-group-key={group.key}
                          x={x}
                          y={y}
                          width={barWidth}
                          height={Math.max(segmentHeight, group.energyKwh > 0 ? 2 : 0)}
                          fill={group.color}
                          pointerEvents="none"
                        />
                      );
                    })
                  ) : (
                    <rect
                      data-testid="daily-energy-bar"
                      x={x}
                      y={margin.top + innerHeight - clippedBarHeight}
                      width={barWidth}
                      height={clippedBarHeight}
                      fill={seriesColor ?? CHART_COLORS.accent}
                      pointerEvents="none"
                    />
                  )}
                </g>
              </g>
              {valueLabelIndexSet.has(dayIndex) ? (
                <text
                  data-testid="daily-charge-value-label"
                  x={x + barWidth / 2}
                  y={Math.max(margin.top + 10, totalLabelY)}
                  textAnchor="middle"
                  fill={CHART_COLORS.muted}
                  fontFamily={CHART_FONT.fontFamily}
                  fontSize={CHART_FONT.fontSize}
                  fontWeight={700}
                >
                  {formatSmartNumber(day.totalEnergyKwh, day.totalEnergyKwh >= 100 ? 0 : 1)}
                </text>
              ) : null}
              {axisLabelIndexSet.has(dayIndex) ? (
                <text
                  data-testid="daily-charge-axis-label"
                  x={x + barWidth / 2}
                  y={chartHeight - 28}
                  textAnchor="middle"
                  fill={CHART_COLORS.muted}
                  fontFamily={CHART_FONT.fontFamily}
                  fontSize={CHART_FONT.fontSize}
                  fontWeight={CHART_FONT.fontWeight}
                >
                  {formatDayLabel(day.day_local)}
                </text>
              ) : null}
            </g>
          );
        })}

        {selectionBounds ? (
          <rect
            x={selectionX}
            y={margin.top}
            width={selectionWidth}
            height={innerHeight}
            fill="var(--rm-accent)"
            fillOpacity={0.14}
            stroke="var(--rm-accent)"
            strokeDasharray="4 4"
            pointerEvents="none"
          />
        ) : null}

        <text
          x={24}
          y={margin.top + innerHeight / 2}
          fill={CHART_COLORS.muted}
          fontFamily={CHART_FONT.fontFamily}
          fontSize={CHART_FONT.fontSize + 1}
          fontWeight={700}
          textAnchor="middle"
          transform={`rotate(-90 24 ${margin.top + innerHeight / 2})`}
        >
          Energy charged (kWh)
        </text>
      </svg>
    </div>
  );
}

export function DailyChargeSessionsChart(props: DailyChargeSessionsChartProps) {
  return <DailyChargingBarChart {...props} variant="stacked" />;
}

export function DailyEnergyBarChart({
  daily,
  height,
  loading,
  emptyTitle = 'No charging energy for this period',
  yRange,
  interactionMode,
  seriesColor,
}: DailyEnergyBarChartProps) {
  return (
    <DailyChargingBarChart
      daily={daily}
      emptyTitle={emptyTitle}
      variant="total"
      {...(height !== undefined ? { height } : {})}
      {...(loading !== undefined ? { loading } : {})}
      {...(yRange !== undefined ? { yRange } : {})}
      {...(interactionMode !== undefined ? { interactionMode } : {})}
      {...(seriesColor !== undefined ? { seriesColor } : {})}
    />
  );
}

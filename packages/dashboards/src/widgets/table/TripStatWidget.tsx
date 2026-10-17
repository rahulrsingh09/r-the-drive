import React from 'react';
import { Icon } from '@iconify/react';
import { useEfficiencySummary, useMetricValue, useMetricSeries } from '@panda/hooks';
import { getChartColor, MiniSparkline } from '@panda/ui/charts';
import { Card } from '@panda/ui/primitives';
import { cn, formatMiles, formatDuration, formatEfficiency } from '@panda/ui/lib/utils';
import { registerWidget } from '../../registry';
import type { WidgetInstance, WidgetCtx } from '../../registry';
import {
  useDashboardDataAvailable,
  useDashboardDataSelector,
  useDashboardMetric,
} from '../../dashboardData';
import { resolveIconId } from '../../editor/iconMigration';
import type { SensorIconKey } from '../sensor/sensorDefinitions';
import { selectedTripRows } from './tableProjection';
import { resolveWidgetOptions } from '../widgetOptions';
import { useTripSelection, resetTripSelection } from './tripSelectionStore';
import { computeTripStat, formatTripSelectionLabel, formatTripStat, type TripStatKind } from './tripStatMetrics';

type TripStat = TripStatKind;

interface TripStatOptions {
  stat: TripStat;
  metric: string;
  icon: SensorIconKey;
  accentBorder: boolean;
}

function readOptions(instance: WidgetInstance): TripStatOptions {
  return resolveWidgetOptions<TripStatOptions>({ stat: 'count', metric: 'total_trips', icon: 'calendar', accentBorder: false }, instance.options ?? {});
}

function formatApiValue(value: number | null | undefined, unit: string | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return '-';
  const format = { mi: formatMiles, min: formatDuration, 'Wh/mi': formatEfficiency }[unit ?? ''];
  return format ? format(value) : value.toFixed((!unit && Number.isInteger(value)) || Math.abs(value) >= 100 ? 0 : 1);
}

function aggregateSeriesForStat(stat: TripStat, series: Array<{ value: number | null | undefined }>) {
  const summary = { total: 0, count: 0 };
  for (const point of series) if (typeof point.value === 'number' && Number.isFinite(point.value)) { summary.total += point.value; summary.count++; }
  const divisor = stat === 'efficiency' || stat === 'duration' ? summary.count : 1;
  return summary.count ? summary.total / divisor : null;
}

export function TripStatWidget({ instance, ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const options = readOptions(instance);
  const { selectedIds, tripRegistry } = useTripSelection();
  const dashboardDataAvailable = useDashboardDataAvailable();
  const dashboardMetric = useDashboardMetric(options.metric);
  const dashboardEfficiency = useDashboardDataSelector((snapshot) => snapshot.efficiencySummary);
  const { data: fetchedValue } = useMetricValue(
    dashboardDataAvailable ? null : ctx.vehicleId,
    dashboardDataAvailable ? null : options.metric,
  );
  const { data: fetchedSeries = [] } = useMetricSeries(
    dashboardDataAvailable ? null : ctx.vehicleId,
    dashboardDataAvailable ? null : options.metric,
    ctx.from,
    ctx.to,
  );
  const { data: fetchedEfficiencySummary } = useEfficiencySummary(
    !dashboardDataAvailable && options.stat === 'efficiency' ? ctx.vehicleId : null,
    ctx.from,
    ctx.to,
    ctx.tripTagFilter,
  );
  const value = dashboardDataAvailable ? dashboardMetric?.value : fetchedValue;
  const series = dashboardDataAvailable ? dashboardMetric?.series ?? [] : fetchedSeries;
  const efficiencySummary = dashboardDataAvailable ? dashboardEfficiency : fetchedEfficiencySummary;

  React.useEffect(() => {
    resetTripSelection(`${ctx.vehicleId}::${ctx.from}::${ctx.to}`, { force: true });
  }, [ctx.vehicleId, ctx.from, ctx.to]);

  const title = instance.title ?? options.metric;
  const iconId = resolveIconId(options.icon);
  const selectedTrips = selectedTripRows(selectedIds, tripRegistry);
  const hasSelection = selectedTrips.length > 0;
  const isLifetime = ctx.timeframe?.kind === 'lifetime';
  const aggregatedValue = aggregateSeriesForStat(options.stat, series);
  const fallbackValue = isLifetime ? value?.value : null;
  const summaryValue = options.stat === 'efficiency' ? efficiencySummary?.avg ?? null : null;

  const displayValue = hasSelection
    ? formatTripStat(options.stat, computeTripStat(options.stat, selectedTrips))
    : options.stat === 'efficiency'
      ? formatApiValue(summaryValue ?? aggregatedValue ?? fallbackValue, value?.unit)
      : formatApiValue(aggregatedValue ?? fallbackValue, value?.unit);

  const spriteData = series.filter(
    (p): p is { ts: string; value: number } =>
      typeof p.ts === 'string' && typeof p.value === 'number' && Number.isFinite(p.value),
  );

  return (
    <Card
      padding="none"
      className={cn(
        'relative flex h-full min-h-[72px] flex-col overflow-hidden border p-3',
        options.accentBorder
          ? 'border-accent/60 shadow-[inset_0_0_0_1px_var(--rm-border-accent)]'
          : 'border-border',
      )}
      data-testid="sensor-chip"
    >
      {!hasSelection && spriteData.length > 0 ? (
        <div
          className="pointer-events-none absolute h-9"
          style={{ left: 0, right: 0, bottom: 0, zIndex: 0, opacity: 0.82 }}
          data-testid="sensor-sprite-layer"
        >
          <MiniSparkline
            data={spriteData}
            type="line"
            height={36}
            color={getChartColor('accent')}
            showFallback
            timeFilter="24h"
          />
          <div className="absolute inset-x-0 bottom-[2px] h-px bg-accent/35" aria-hidden="true" />
        </div>
      ) : null}

      <div className={cn('relative z-10 flex flex-col', !hasSelection && spriteData.length === 0 && 'flex-1 justify-center')}>
        <div className="flex items-start justify-between gap-3">
          <p className="truncate text-xs font-medium uppercase tracking-wider text-fg-tertiary">
            {title}
          </p>
          <Icon icon={iconId} className="h-4 w-4 shrink-0 text-accent" />
        </div>
        <div className="mt-1.5 flex items-baseline gap-1">
          <span
            className="text-2xl font-mono font-semibold tabular-nums tracking-tight text-accent"
            style={{ textShadow: 'var(--rm-value-halo)' }}
          >
            {displayValue}
          </span>
        </div>
        {hasSelection ? (
          <p className="mt-0.5 text-[10px] text-fg-tertiary">
            {formatTripSelectionLabel(selectedTrips.length)}
          </p>
        ) : null}
      </div>
    </Card>
  );
}

registerWidget({
  componentType: 'custom',
  definitionId: 'trips.stat',
  title: 'Trip Stat',
  defaultSize: { w: 3, h: 2 },
  minSize: { w: 2, h: 2 },
  defaultOptions: {},
  editor: {
    category: 'Trips',
    fixedSize: true,
    deprecated: true,
    description: 'Legacy fixed-size trip stat chip retained for imported dashboards.',
  },
  dataRequirements: (instance) => {
    const options = readOptions(instance);
    return {
      metrics: [{ metric: options.metric, include_latest: true, include_series: true }],
      ...(options.stat === 'efficiency' ? { efficiencySummary: true } : {}),
    };
  },
  component: TripStatWidget,
});

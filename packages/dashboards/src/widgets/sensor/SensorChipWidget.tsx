import { sensorSamples, sensorSourceLoading } from './sensorReadings';
import { sensorColorSelection } from './sensorColor';
import { resolveWidgetOptions } from '../widgetOptions';
import React from 'react';
import type { ChartColorDefinition, ChartColorToken } from '@panda/types';
import { Icon } from '@iconify/react';
import { CircleHelp } from 'lucide-react';
import {
  useBatteryHealth,
  useChargingSummary,
  useCurrentVehicleStatus,
  useEfficiencySummary,
  useMetricSeries,
  useMetricValue,
} from '@panda/hooks';
import {
  DEFAULT_CURVE_SMOOTHNESS,
  DEFAULT_SPRITE_TIME_FILTER,
  MiniSparkline,
  normalizeCurveSmoothness,
  normalizeTimeFilter,
  type CurveSmoothness,
  type MiniSparklineType,
  type TimeFilterWindow,
} from '@panda/ui/charts';
import { Badge, Card, Tooltip } from '@panda/ui/primitives';
import { useDocumentPalette, useDocumentTheme } from '@panda/ui/hooks';
import {
  cn,
  formatCurrency,
  formatDuration,
  formatEfficiency,
  formatKwh,
  formatMiles,
  formatMph,
  formatPressure,
  formatTemp,
} from '@panda/ui/lib/utils';
import { presentVehicleStatusDefinition, type StatusTone } from '@panda/ui/lib/vehicleStatus';
import { registerWidget } from '../../registry';
import type { WidgetInstance, WidgetCtx } from '../../registry';
import {
  useDashboardDataAvailable,
  useDashboardDataSelector,
  useDashboardMetric,
  type DashboardDataRequirements,
} from '../../dashboardData';
import { resolveIconId } from '../../editor/iconMigration';
import { seriesToDailyDeltas, seriesToDailyTotals } from '../../editor/dailyDelta';
import { selectedTripRows } from '../table/tableProjection';
import { useTripSelection } from '../table/tripSelectionStore';
import {
  computeTripStat,
  formatTripSelectionLabel,
  formatTripStat,
  getTripStatKind,
} from '../table/tripStatMetrics';
import {
  getSensorDefinition,
  getSensorDataAccent,
  resolveSensorColorToken,
  resolveSensorValueColor,
  SENSOR_DEFINITIONS,
  type SensorDefinition,
  type SensorChartType,
  type SensorDataSource,
  type SensorIconKey,
  type SensorValueColor,
} from './sensorDefinitions';
import { resolveSafeExpression } from '../../charts/expressions';

interface SensorChipOptions {
  metric?: string;
  icon?: SensorIconKey;
  chartType?: SensorChartType;
  dataSource?: SensorDataSource;
  valuePath?: string;
  fallbackValuePath?: string;
  valueFormula?: string;
  unit?: string | null;
  inlineSecondaryPath?: string;
  inlineSecondaryFormula?: string;
  inlineSecondaryTemplate?: string;
  inlineSecondaryUnit?: string | null;
  inlineSecondaryPrefix?: string;
  secondaryTemplate?: string;
  helpText?: string;
  labelSuffix?: string;
  showSprite?: boolean;
  showSubtitle?: boolean;
  subtitle?: string;
  accentBorder?: boolean;
  valueSize?: 'sm' | 'md' | 'lg';
  valueColor?: SensorValueColor;
  valueMode?: 'latest' | 'sum' | 'avg' | 'count';
  timeFilter?: TimeFilterWindow;
  smoothness?: CurveSmoothness;
  curveSmoothing?: number | boolean;
  curveColor?: ChartColorToken | ChartColorDefinition;
  windowDays?: number;
  timeframeScope?: 'range' | 'current' | 'lifetime';
  tripSelectionAware?: boolean;
}

const DEFAULT_WINDOW_DAYS = 30;

function readOptions(instance: WidgetInstance, automaticColor: ChartColorToken = 'accent'): Required<SensorChipOptions> {
  const definition = getSensorDefinition(instance.definitionId) ?? SENSOR_DEFINITIONS[0]!;
  const options = (instance.options ?? {}) as SensorChipOptions;
  const defaults: Required<SensorChipOptions> = {
    metric: '', icon: definition.icon, chartType: definition.chartType, dataSource: 'metric',
    valuePath: '', fallbackValuePath: '', valueFormula: '', unit: null,
    inlineSecondaryPath: '', inlineSecondaryFormula: '', inlineSecondaryTemplate: '', inlineSecondaryUnit: null,
    inlineSecondaryPrefix: '', secondaryTemplate: '', helpText: '', labelSuffix: '',
    showSprite: true, showSubtitle: false, subtitle: '', accentBorder: definition.accent ?? false,
    valueSize: 'md', valueColor: 'accent', valueMode: definition.valueMode, curveColor: automaticColor,
    timeframeScope: 'range', tripSelectionAware: false, curveSmoothing: 0,
    timeFilter: 'raw', smoothness: 'straight', windowDays: DEFAULT_WINDOW_DAYS,
  };
  const resolved = resolveWidgetOptions(defaults, definition, options);
  resolved.timeFilter = normalizeTimeFilter(options.timeFilter, legacySmoothingToTimeFilter(options.curveSmoothing, resolved.chartType));
  resolved.smoothness = normalizeCurveSmoothness(options.smoothness, legacyCurveSmoothingToSmoothness(options.curveSmoothing, resolved.chartType));
  resolved.windowDays = typeof options.windowDays === 'number' && Number.isFinite(options.windowDays)
    ? Math.max(1, Math.min(365, Math.round(options.windowDays))) : DEFAULT_WINDOW_DAYS;
  return resolved;
}

export function SensorChipWidget({ instance, ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const definition = getSensorDefinition(instance.definitionId);
  const palette = useDocumentPalette();
  const rawOptions = (instance.options ?? {}) as SensorChipOptions;
  const automaticColor = resolveSensorColorToken(rawOptions.metric ?? definition?.metric, definition, palette);
  const options = readOptions(instance, automaticColor);
  const isDark = useDocumentTheme();
  const metricColor = resolveSensorVisualColor(rawOptions.curveColor, automaticColor, isDark);
  const metricValueColor = resolveSensorValueColor(rawOptions.curveColor, automaticColor, metricColor, isDark);
  const metricDataAccent = getSensorDataAccent(options.metric, definition);
  const hasVisualMetricColor = Boolean(metricDataAccent || rawOptions.curveColor);
  const metric = options.dataSource === 'metric' ? options.metric : null;
  const { selectedIds, tripRegistry } = useTripSelection();
  const needsHealth = options.dataSource === 'batteryHealth';
  const needsCharging = options.dataSource === 'chargingSummary';
  const needsEfficiency = metric === 'avg_efficiency' || options.dataSource === 'efficiencySummary';
  const needsStatus = options.dataSource === 'vehicleStatus' || usesStatus(options);
  const tripSelectionStat = (ctx.dashboardSlug === 'trips' || options.tripSelectionAware)
    ? getTripStatKind(metric)
    : null;
  const selectedTrips = tripSelectionStat
    ? selectedTripRows(selectedIds, tripRegistry)
    : [];
  const activeTripSelectionStat = tripSelectionStat != null && selectedTrips.length > 0 ? tripSelectionStat : null;
  const selectionLabel = activeTripSelectionStat ? formatTripSelectionLabel(selectedTrips.length) : null;
  const selectionValue = activeTripSelectionStat ? computeTripStat(activeTripSelectionStat, selectedTrips) : null;
  const dashboardDataAvailable = useDashboardDataAvailable();
  const dashboardMetric = useDashboardMetric(metric);
  const dashboardStatus = useDashboardDataSelector((snapshot) => snapshot.status);
  const dashboardHealth = useDashboardDataSelector((snapshot) => snapshot.batteryHealth);
  const dashboardCharging = useDashboardDataSelector((snapshot) => snapshot.chargingSummary);
  const dashboardEfficiency = useDashboardDataSelector((snapshot) => snapshot.efficiencySummary);
  const dashboardRefreshing = useDashboardDataSelector((snapshot) => snapshot.isRefreshing) ?? false;
  const { data: fetchedValue } = useMetricValue(
    dashboardDataAvailable ? null : ctx.vehicleId,
    dashboardDataAvailable ? null : metric,
    ctx.from,
    ctx.to,
  );
  const { data: fetchedSeries = [] } = useMetricSeries(
    dashboardDataAvailable ? null : ctx.vehicleId,
    dashboardDataAvailable ? null : metric,
    ctx.from,
    ctx.to,
  );
  const { data: fetchedEfficiencySummary } = useEfficiencySummary(
    !dashboardDataAvailable && needsEfficiency ? ctx.vehicleId : null,
    ctx.from,
    ctx.to,
    ctx.tripTagFilter,
  );
  const { data: fetchedHealth, isLoading: healthLoading } = useBatteryHealth(
    !dashboardDataAvailable && needsHealth ? ctx.vehicleId : null
  );
  const { data: fetchedChargingSummary, isLoading: chargingLoading } = useChargingSummary(
    !dashboardDataAvailable && needsCharging ? ctx.vehicleId : null,
    ctx.from,
    ctx.to
  );
  const { data: fetchedStatus, isLoading: statusLoading } = useCurrentVehicleStatus(
    !dashboardDataAvailable && needsStatus ? ctx.vehicleId : null
  );
  const value = dashboardDataAvailable ? dashboardMetric?.value : fetchedValue;
  const series = dashboardDataAvailable ? dashboardMetric?.series ?? [] : fetchedSeries;
  const efficiencySummary = dashboardDataAvailable ? dashboardEfficiency : fetchedEfficiencySummary;
  const health = dashboardDataAvailable ? dashboardHealth : fetchedHealth;
  const chargingSummary = dashboardDataAvailable ? dashboardCharging : fetchedChargingSummary;
  const status = dashboardDataAvailable ? dashboardStatus : fetchedStatus;
  const title = instance.title ?? definition?.title ?? value?.label ?? options.metric;
  const iconId = resolveIconId(options.icon);
  const sourceValues = buildSourceValues(options.dataSource, health, chargingSummary, efficiencySummary, status);
  const timeframeScope = options.timeframeScope;
  const isLifetimeTimeframe = ctx.timeframe?.kind === 'lifetime';
  const allowLatestFallback = timeframeScope !== 'range' || isLifetimeTimeframe;
  const isLoading = sensorSourceLoading(options.dataSource, dashboardDataAvailable, dashboardRefreshing, {
    health: healthLoading, charging: chargingLoading, status: statusLoading, needsStatus,
  });
  const samples = sensorSamples(series);
  const hasFiniteSeriesPoint = samples.available;
  const resolvedMetricValue = allowLatestFallback || hasFiniteSeriesPoint ? (value?.value ?? null) : null;
  const resolvedValue =
    options.dataSource === 'metric'
      ? metric === 'avg_efficiency'
        ? (efficiencySummary?.avg ?? (allowLatestFallback ? (value?.value ?? null) : null))
        : resolvedMetricValue
      : resolveConfiguredValue(options, sourceValues);
  const statusPresentation =
    options.dataSource === 'vehicleStatus'
      ? presentVehicleStatusDefinition(instance.definitionId, status)
      : null;
  const unit = options.dataSource === 'metric' ? value?.unit : options.unit;
  const { displayValue, inlineSecondary, secondary } = sensorReadout({
    presentation: statusPresentation, loading: isLoading, selection: activeTripSelectionStat,
    selectedValue: selectionValue, value: resolvedValue, unit, options, values: sourceValues,
  });
  const lastUpdatedLabel = activeTripSelectionStat ? null : statusPresentation?.lastUpdatedLabel ?? null;
  const valueToneClass = statusPresentation
    ? statusToneClass(statusPresentation.variant)
    : options.valueColor === 'accent'
      ? 'text-accent'
      : 'text-fg';
  const valueToneStyle = !statusPresentation && options.valueColor === 'data'
    ? { color: metricValueColor }
    : undefined;
  const helpContent = getHelpContent(options.helpText, metric, efficiencySummary);

  const isDailyDelta = options.chartType === 'daily_delta';
  const sparklineType: MiniSparklineType = isDailyDelta
    ? 'bar'
    : (options.chartType as MiniSparklineType);
  const spriteData = isDailyDelta
    ? isEventTotalMetric(metric ?? '')
      ? seriesToDailyTotals(series, options.windowDays)
      : seriesToDailyDeltas(series, options.windowDays)
    : samples.sprite(
        allowLatestFallback ? resolvedValue : null,
        value?.ts ?? ctx.to ?? new Date().toISOString()
      );
  const showSprite = options.showSprite && options.chartType !== 'none' && !activeTripSelectionStat;
  const spriteColor = metricColor;

  return (
    <Card
      padding="none"
      className={cn(
        'relative flex h-full min-h-[72px] flex-col overflow-hidden border p-3',
        options.accentBorder
          ? 'shadow-[inset_0_0_0_1px_var(--rm-border-accent)]'
          : 'border-border'
      )}
      style={options.accentBorder ? { borderColor: metricColor } : undefined}
      data-testid="sensor-chip"
    >
      {showSprite ? (
        <div
          className="pointer-events-none absolute h-9"
          style={{ left: 0, right: 0, bottom: 0, zIndex: 0, opacity: 0.82 }}
          data-testid="sensor-sprite-layer"
        >
          <MiniSparkline
            data={spriteData}
            type={sparklineType}
            height={36}
            color={spriteColor}
            showFallback
            timeFilter={options.timeFilter}
            smoothness={options.smoothness}
          />
          <div
            className="absolute inset-x-0 bottom-[2px] h-px"
            style={{ backgroundColor: spriteColor, opacity: 0.35 }}
            aria-hidden="true"
          />
        </div>
      ) : null}

      <div className={cn('relative z-10 flex flex-col', !activeTripSelectionStat && !showSprite && 'flex-1 justify-center')}>
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-1.5">
              <p className="min-w-0 truncate text-xs font-medium uppercase tracking-wider text-fg-tertiary">
                {title}
                {options.labelSuffix ? (
                  <span className="ml-1 text-[10px] font-normal normal-case tracking-normal">
                    ({options.labelSuffix})
                  </span>
                ) : null}
              </p>
              {helpContent ? (
                <Tooltip content={helpContent} align="start">
                  <button
                    type="button"
                    aria-label={`${title} help`}
                    className="inline-flex shrink-0 rounded text-fg-tertiary transition-colors hover:text-fg focus:outline-none focus-visible:ring-1 focus-visible:ring-accent"
                  >
                    <CircleHelp className="h-3.5 w-3.5" aria-hidden="true" />
                  </button>
                </Tooltip>
              ) : null}
              {timeframeScope === 'lifetime' ? (
                <Badge
                  size="sm"
                  variant="default"
                  className="shrink-0 rounded-md px-1.5 py-0.5 font-semibold"
                >
                  Lifetime
                </Badge>
              ) : null}
            </div>
            {options.showSubtitle && options.subtitle ? (
              <p className="mt-1 truncate text-xs text-fg-tertiary">{options.subtitle}</p>
            ) : null}
          </div>
          <Icon
            icon={iconId}
            className={cn('h-4 w-4 shrink-0', hasVisualMetricColor ? 'text-fg' : 'text-accent')}
            style={hasVisualMetricColor ? { color: metricColor } : undefined}
          />
        </div>

        <div className="mt-1.5 flex items-baseline gap-1">
          {renderStatusValue({
            presentation: statusPresentation,
            displayValue,
            valueToneClass,
            ...(valueToneStyle ? { valueToneStyle } : {}),
            valueSize: options.valueSize,
          })}
          {inlineSecondary ? (
            <span className="font-mono text-sm tabular-nums text-fg-tertiary">
              {inlineSecondary}
            </span>
          ) : null}
        </div>
        {selectionLabel ? (
          <p className="mt-0.5 text-[10px] text-fg-tertiary">{selectionLabel}</p>
        ) : lastUpdatedLabel ? (
          <p className="mt-0.5 truncate text-[11px] text-fg-tertiary">{lastUpdatedLabel}</p>
        ) : null}
        {secondary ? <p className="mt-0.5 truncate text-xs text-fg-tertiary">{secondary}</p> : null}
      </div>
    </Card>
  );
}

function sensorReadout(input: {
  presentation: ReturnType<typeof presentVehicleStatusDefinition> | null;
  loading: boolean; selection: ReturnType<typeof getTripStatKind>; selectedValue: number | null;
  value: number | null | undefined; unit: string | null | undefined;
  options: Required<SensorChipOptions>; values: Record<string, unknown>;
}) {
  const result = { displayValue: '', inlineSecondary: '', secondary: '' };
  if (input.presentation) return { ...result, displayValue: input.presentation.label, secondary: input.presentation.secondaryText ?? '' };
  if (input.loading) return { ...result, displayValue: '...' };
  if (input.selection) return { ...result, displayValue: formatTripStat(input.selection, input.selectedValue) };
  return {
    displayValue: formatMetricValue(input.value, input.unit),
    inlineSecondary: resolveInlineSecondary(input.options, input.values),
    secondary: resolveTemplate(input.options.secondaryTemplate, input.values),
  };
}

function resolveSensorVisualColor(
  curveColor: ChartColorToken | ChartColorDefinition | undefined,
  automaticColor: ChartColorToken,
  isDark: boolean,
) {
  return sensorColorSelection(curveColor, automaticColor, isDark).color;
}

function buildSourceValues(
  dataSource: SensorDataSource,
  health: unknown,
  chargingSummary: unknown,
  efficiencySummary: unknown,
  status: unknown
) {
  const sources = { batteryHealth: health, chargingSummary, efficiencySummary, vehicleStatus: status, metric: {} };
  const values: Record<string, unknown> = { ...objectValues(sources[dataSource]) };
  if (dataSource === 'chargingSummary') {
    const charging = objectValues(chargingSummary);
    values.away_kwh_including_unknown = ['away_kwh', 'unknown_location_kwh'].reduce((sum, key) => {
      const value = charging[key];
      return sum + (typeof value === 'number' && Number.isFinite(value) ? value : 0);
    }, 0);
  }
  return Object.assign(values, { health, battery: health, charging: chargingSummary, efficiency: efficiencySummary, status });
}

function getHelpContent(
  helpText: string,
  metric: string | null,
  efficiencySummary: { efficiency_miles: number; total_miles: number } | null | undefined,
) {
  if (!helpText) return null;
  if (metric !== 'avg_efficiency' || !efficiencySummary || efficiencySummary.total_miles <= 0) {
    return helpText;
  }
  return `${helpText} Based on ${formatMiles(efficiencySummary.efficiency_miles)} of ${formatMiles(efficiencySummary.total_miles)} driven.`;
}

function objectValues(value: unknown) {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
}

function resolveConfiguredValue(
  options: Required<SensorChipOptions>,
  values: Record<string, unknown>
) {
  if (options.valueFormula) return resolveFormula(options.valueFormula, values);
  const paths = [options.valuePath, options.fallbackValuePath];
  const selected = paths.map((path) => resolveNumberPath(values, path)).find((value) => value != null);
  return selected ?? null;
}

function resolveInlineSecondary(
  options: Required<SensorChipOptions>,
  values: Record<string, unknown>
) {
  const template = options.inlineSecondaryTemplate;
  if (template) return resolveTemplate(template, values);
  const projected = resolveConfiguredValue({ ...options, valueFormula: options.inlineSecondaryFormula, valuePath: options.inlineSecondaryPath, fallbackValuePath: '' }, values);
  return projected == null ? '' : options.inlineSecondaryPrefix + formatMetricValue(projected, options.inlineSecondaryUnit);
}

function resolveTemplate(template: string, values: Record<string, unknown>) {
  const slots = [...template.matchAll(/\[([^\]:]+)(?::([^\]]+))?\]/g)];
  const formatted: string[] = [];
  for (const slot of slots) {
    const number = resolveNumberPath(values, slot[1]!.trim());
    if (number == null) return '';
    formatted.push(formatTemplateValue(number, slot[2]));
  }
  let result = '', cursor = 0;
  slots.forEach((slot, index) => {
    result += template.slice(cursor, slot.index) + formatted[index];
    cursor = slot.index! + slot[0].length;
  });
  return result + template.slice(cursor);
}

function formatTemplateValue(value: number, format: string | undefined) {
  if (!format) return formatMetricValue(value, undefined);
  if (format === 'int' || format === 'integer') return value.toFixed(0);
  return formatMetricValue(value, format);
}

function resolveNumberPath(values: Record<string, unknown>, path: string) {
  if (!path) return null;
  const value = resolvePath(values, path);
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function resolvePath(values: Record<string, unknown>, path: string): unknown {
  if (Object.prototype.hasOwnProperty.call(values, path)) return values[path];
  let cursor: unknown = values;
  for (const segment of path.split('.')) {
    if (cursor === null || typeof cursor !== 'object') return undefined;
    cursor = (cursor as Record<string, unknown>)[segment];
  }
  return cursor;
}

function resolveFormula(formula: string, values: Record<string, unknown>) {
  return resolveSafeExpression(formula, values);
}

function usesStatus(options: Required<SensorChipOptions>) {
  return [
    options.valuePath,
    options.fallbackValuePath,
    options.valueFormula,
    options.inlineSecondaryPath,
    options.inlineSecondaryFormula,
    options.inlineSecondaryTemplate,
    options.secondaryTemplate,
  ].some((value) => value.includes('status.'));
}

function isEventTotalMetric(metric: string) {
  return metric === 'total_miles'
    || metric === 'total_trips'
    || metric === 'energy_charged'
    || metric === 'total_cost';
}

function formatMetricValue(value: number | null | undefined, unit: string | null | undefined) {
  if (value == null || !Number.isFinite(value)) return '-';
  const formatter = { mi: formatMiles, kWh: formatKwh, USD: formatCurrency, 'Wh/mi': formatEfficiency, min: formatDuration, psi: formatPressure, mph: formatMph, C: formatTemp }[unit ?? ''];
  if (formatter) return formatter(value);
  switch (unit) {
    case '%': return value.toFixed(value >= 10 ? 0 : 1) + '%';
    case 'kW': return value.toFixed(1) + ' kW';
    default: return value.toFixed((!unit && Number.isInteger(value)) || Math.abs(value) >= 100 ? 0 : 1);
  }
}

function renderStatusValue({
  presentation,
  displayValue,
  valueToneClass,
  valueToneStyle,
  valueSize,
}: {
  presentation: ReturnType<typeof presentVehicleStatusDefinition> | null;
  displayValue: string;
  valueToneClass: string;
  valueToneStyle?: React.CSSProperties;
  valueSize: Required<SensorChipOptions>['valueSize'];
}) {
  const unavailable = presentation?.renderUnavailableChip;
  const numericValue = presentation ? null : /^([+-]?[\d,.]+)\s+(.+)$/.exec(displayValue);
  const valueNode = unavailable ? (
      <Badge
        variant="info"
        className="rounded-full font-semibold"
        data-testid="sensor-unavailable-chip"
      >
        {displayValue}
      </Badge>
    ) : (
    <span
      data-sensor-value
      className={cn(
        'font-mono font-semibold tabular-nums tracking-tight',
        valueToneClass,
        valueSize === 'sm' ? 'text-xl' : valueSize === 'lg' ? 'text-3xl' : 'text-2xl'
      )}
      style={{ textShadow: 'var(--rm-value-halo)', ...valueToneStyle }}
    >
      {numericValue
        ? <><span>{numericValue[1]}</span>{' '}<span data-sensor-unit>{numericValue[2]}</span></>
        : displayValue}
    </span>
  );

  return presentation?.tooltip ? (
    <Tooltip content={presentation.tooltip}>{valueNode}</Tooltip>
  ) : (
    valueNode
  );
}

function statusToneClass(tone: StatusTone) {
  const tones: Partial<Record<StatusTone, string>> = { success: 'text-status-positive', warning: 'text-status-warning', danger: 'text-status-danger', info: 'text-status-info' };
  return tones[tone] ?? 'text-fg';
}

function legacySmoothingToTimeFilter(value: unknown, chartType: SensorChartType | 'none'): TimeFilterWindow {
  const policy = SENSOR_CHART_POLICIES[chartType] ?? SENSOR_CHART_POLICIES.none;
  const disabled = policy.curved && (value === false || value === 0);
  return policy.filtered && !disabled ? DEFAULT_SPRITE_TIME_FILTER : 'raw';
}

function legacyCurveSmoothingToSmoothness(value: unknown, chartType: SensorChartType | 'none'): CurveSmoothness {
  return SENSOR_CHART_POLICIES[chartType]?.curved ? normalizeCurveSmoothness(value, DEFAULT_CURVE_SMOOTHNESS) : 'straight';
}

function sensorDataRequirements(instance: WidgetInstance): DashboardDataRequirements {
  const options = readOptions(instance);
  const requirements: DashboardDataRequirements = {};
  const dataSource = options.dataSource;
  if (dataSource === 'metric' && options.metric) requirements.metrics = [{ metric: options.metric, include_latest: true, include_series: options.chartType !== 'none' }];
  if (dataSource === 'batteryHealth') requirements.batteryHealth = true;
  if (dataSource === 'chargingSummary') requirements.chargingSummary = true;
  if (requirements.metrics?.[0]?.metric === 'avg_efficiency' || dataSource === 'efficiencySummary') requirements.efficiencySummary = true;
  if (dataSource === 'vehicleStatus' || usesStatus(options)) requirements.status = true;
  return requirements;
}


const SENSOR_CHART_POLICIES: Record<SensorChartType | 'none', { filtered: boolean; curved: boolean }> = {
  none: { filtered: false, curved: false },
  line: { filtered: true, curved: true },
  area: { filtered: true, curved: true },
  bar: { filtered: true, curved: false },
  daily_delta: { filtered: true, curved: false },
};

function registeredSensorOptions(definition: SensorDefinition) {
  const defaults = {
    metric: '', icon: definition.icon, chartType: definition.chartType,
    dataSource: 'metric', valuePath: '', fallbackValuePath: '', valueFormula: '', unit: null,
    inlineSecondaryPath: '', inlineSecondaryFormula: '', inlineSecondaryTemplate: '',
    inlineSecondaryUnit: null, inlineSecondaryPrefix: '', secondaryTemplate: '', helpText: '',
    labelSuffix: '', valueMode: definition.valueMode, valueColor: 'accent',
  };
  const fields = resolveWidgetOptions(defaults, definition);
  const policy = SENSOR_CHART_POLICIES[definition.chartType];
  return {
    ...fields, showSprite: definition.chartType !== 'none',
    timeFilter: policy.filtered ? DEFAULT_SPRITE_TIME_FILTER : 'raw',
    showSubtitle: false, accentBorder: definition.accent ?? false, valueSize: 'md',
    ...(definition.cumulative === true ? { windowDays: DEFAULT_WINDOW_DAYS } : {}),
  };
}

for (const definition of SENSOR_DEFINITIONS) {
  registerWidget({
    componentType: 'sensor',
    definitionId: definition.id,
    title: definition.title,
    defaultSize: { w: 3, h: 2 },
    minSize: { w: 2, h: 2 },
    defaultOptions: registeredSensorOptions(definition),
    editor: {
      category: 'Sensors',
      description:
        definition.dataSource === 'metric'
          ? 'Resizable sensor chip backed by the metric catalog.'
          : 'Resizable sensor chip backed by dashboard summary data.',
    },
    dataRequirements: sensorDataRequirements,
    component: SensorChipWidget,
  });
}

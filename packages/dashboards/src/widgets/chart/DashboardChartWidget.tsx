import { widgetChartCapabilities } from './chartCapabilities';
import { scatterBuckets, scatterPalette } from './scatterProjection';
import { resolveWidgetOptions } from '../widgetOptions';
import { efficiencyProjection, efficiencyRows, efficiencyTimeline } from './efficiencyProjection';
import { useWidgetDialogDismiss, useCompatibleWidgetMedia } from '../widgetLifecycle';
import { mileageProjection } from './mileageProjection';
import { canonicalSeriesStyle, chartDomain, chartMark, projectTimestampValues } from './chartProjection';
import { prepareSessionCurve, sessionHourSplits } from './sessionCurve';
import React from 'react';
import { createPortal } from 'react-dom';
import { ChevronDown, Maximize2, SlidersHorizontal, X } from 'lucide-react';
import { useNavigate } from '@tanstack/react-router';
import {
  useBatteryMileage,
  useChargeCurve,
  useChargeCurveAnalysis,
  useChargingChartSeries,
  useDegradation,
  useDashboardChartFavorites,
  useChartDatasets,
  useEffectiveCharts,
  useEfficiencyByMode,
  useEfficiencyByTag,
  useEfficiencyTrend,
  useEfficiencyVsTemp,
  usePhantomDrainPeriods,
  useRangeHistory,
  useSocHistory,
  useTirePressureTimeline,
  useTripTags,
  useUpdateDashboardChartFavorite,
  useVehicles,
} from '@panda/hooks';
import {
  CHART_COLORS,
  CURVE_SMOOTHNESS_OPTIONS,
  DEFAULT_CHART_TIME_FILTER,
  DEFAULT_CURVE_SMOOTHNESS,
  DailyChargeSessionsChart,
  DailyEnergyBarChart,
  EfficiencyPillBarChart,
  formatChartNumber,
  getAdaptiveDecimalPrecision,
  getChartColor,
  normalizeTimeFilter,
  curveSmoothnessLabel,
  normalizeCurveSmoothness,
  RichTimeSeriesChart,
  TirePressureTripsChart,
  TIME_FILTER_OPTIONS,
  timeFilterLabel,
  type TimeFilterWindow,
  type CurveSmoothness,
  type RichSeries,
} from '@panda/ui/charts';
import { ChartPicker } from '@panda/ui/primitives';
import { useDocumentTheme } from '@panda/ui/hooks';
import { cn } from '@panda/ui/lib/utils';
import { formatDriveMode } from '@panda/ui/lib/driveMode';
import {
  formatMiles,
  formatTemp,
  getUnitPreferences,
} from '@panda/ui/lib/utils';
import { DEFAULT_TARGET_TIRE_PRESSURE_PSI } from '@panda/ui/lib/vehicleTires';
import type { ChargeCurveAnalysisPoint, ChargeCurvePoint } from '@panda/types';
import type { ChartDefinitionV1, ChartRecord } from '@panda/types';
import {
  getChartDefinition,
  getChartDefinitions,
  getChartOptions,
  type DashboardChartAxisCapability,
  type DashboardChartAxisId,
  type DashboardChartDefinition,
  type DashboardChartPage,
  type DashboardChartSettingsCapabilities,
} from '../../charts/catalog';
import { getChartRenderer } from '../../charts/rendererRegistry';
import { normalizeLegacyBundledChartRecord } from '../../charts/legacyBaseline';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';
import { useMeasuredWidgetHeight } from '../useMeasuredWidgetHeight';
import { PhantomDrainChart } from './PhantomDrainChart';
import { MobileChartViewer } from './MobileChartViewer';

export { buildPhantomDrainDailySeries } from './PhantomDrainChart';

interface DashboardChartOptions {
  chartId?: string;
  chartIds?: string[];
  page?: DashboardChartPage;
  showPicker?: boolean;
  /** The widget keeps its layout, but its available charts come from the page catalog. */
  managed?: boolean;
  catalogMode?: 'assigned' | 'fixed' | 'legacy';
  requiredChartSlugs?: string[];
  timeFilter?: TimeFilterWindow;
  smoothness?: CurveSmoothness;
  curveSmoothing?: number | boolean;
  chartSettings?: Record<string, DashboardChartDisplaySettings>;
  /** Optional subtitle shown in the compact header when showPicker is false. */
  headerSubtitle?: string;
}

type DashboardChartAxisMode = 'auto' | 'manual';

interface DashboardChartAxisRangeSetting {
  mode?: DashboardChartAxisMode;
  min?: number;
  max?: number;
}

type DashboardChartResolvedAxisRanges = Partial<Record<'y' | 'y2', [number, number]>>;

interface DashboardChartDisplaySettings {
  timeFilter?: TimeFilterWindow;
  smoothness?: CurveSmoothness;
  /** Legacy geometric interpolation setting, retained only while reading saved dashboards. */
  smoothing?: number;
  axes?: Partial<Record<DashboardChartAxisId, DashboardChartAxisRangeSetting>>;
}

interface ResolvedDashboardChartOptions {
  chartId: string;
  chartIds: string[];
  page?: DashboardChartPage;
  showPicker: boolean;
  legacyTimeFilter: TimeFilterWindow;
  legacySmoothness: CurveSmoothness;
  chartSettings: Record<string, DashboardChartDisplaySettings>;
  headerSubtitle?: string;
  catalogMode: 'assigned' | 'fixed' | 'legacy';
  requiredChartSlugs: string[];
}

const LEGACY_CHART_ID_ALIASES: Record<string, string> = {
  'range-history': 'soc-history',
};

function normalizeChartId(chartId: string) {
  return LEGACY_CHART_ID_ALIASES[chartId] ?? chartId;
}

function chartDefaultStorageKey(ctx: WidgetCtx, instance: WidgetInstance) {
  const slug = ctx.dashboardSlug ?? 'dashboard';
  const dashboardId = ctx.dashboardConfigId;
  return dashboardId ? `${slug}:${dashboardId}:${instance.id}` : `${slug}:${instance.id}`;
}

function readOptions(instance: WidgetInstance): ResolvedDashboardChartOptions {
  const options = (instance.options ?? {}) as DashboardChartOptions;
  const managed = instance.managed === true || options.managed === true;
  const page = isDashboardChartPage(options.page) ? options.page : undefined;
  const definitions = getChartDefinitions(page);
  const ids = definitions.map((definition) => definition.id);
  const valid = new Set(ids);
  const selected = new Set<string>();
  if (managed) ids.forEach((id) => selected.add(id));
  else if (Array.isArray(options.chartIds)) for (const id of options.chartIds) {
    if (typeof id !== 'string') continue;
    const canonical = normalizeChartId(id);
    if (valid.has(canonical)) selected.add(canonical);
  }
  const chartIds = selected.size ? [...selected] : ids;
  const configured = typeof options.chartId === 'string' ? normalizeChartId(options.chartId) : undefined;
  const preferred = page === 'overview' && chartIds.includes('projected-range-mileage') ? 'projected-range-mileage' : chartIds[0];
  return {
    chartId: configured && valid.has(configured) ? configured : preferred ?? getChartDefinitions()[0]?.id ?? 'projected-range-mileage',
    chartIds, showPicker: options.showPicker ?? chartIds.length > 1,
    legacyTimeFilter: normalizeTimeFilter(options.timeFilter, legacySmoothingToTimeFilter(options.curveSmoothing)),
    legacySmoothness: normalizeCurveSmoothness(options.smoothness, normalizeCurveSmoothness(options.curveSmoothing)),
    chartSettings: normalizeChartSettingsMap(options.chartSettings),
    ...(page ? { page } : {}), ...(typeof options.headerSubtitle === 'string' ? { headerSubtitle: options.headerSubtitle } : {}),
    catalogMode: options.catalogMode ?? (page ? 'assigned' : options.chartIds ? 'fixed' : managed ? 'assigned' : 'legacy'),
    requiredChartSlugs: Array.isArray(options.requiredChartSlugs) ? options.requiredChartSlugs.filter((slug): slug is string => typeof slug === 'string') : [],
  };
}

const AXIS_ORDER: DashboardChartAxisId[] = ['x', 'y', 'y2'];

type ChartChoice = { chartId: string; defaultChartId: string };
type ChartChoiceEvent = { type: 'hydrate'; reset: boolean; favorite: string } | { type: 'select' | 'default'; id: string };
function useChartChoice(initial: string, key: string, favorite: string, ready: boolean) {
  const initialized = React.useRef<string | null>(null);
  const [choice, dispatch] = React.useReducer((state: ChartChoice, event: ChartChoiceEvent): ChartChoice => {
    const next = event.type === 'hydrate'
      ? { chartId: event.reset ? event.favorite : state.chartId, defaultChartId: event.favorite }
      : { ...state, [event.type === 'select' ? 'chartId' : 'defaultChartId']: event.id };
    return next.chartId === state.chartId && next.defaultChartId === state.defaultChartId ? state : next;
  }, { chartId: initial, defaultChartId: initial });
  React.useEffect(() => {
    if (!ready) return;
    const reset = initialized.current !== key;
    initialized.current = key;
    dispatch({ type: 'hydrate', reset, favorite });
  }, [key, favorite, ready]);
  return { ...choice, setChartId: (id: string) => dispatch({ type: 'select', id }), setDefaultChartId: (id: string) => dispatch({ type: 'default', id }) };
}

function useChartDraft(initial: Record<string, DashboardChartDisplaySettings>, identity: unknown) {
  const [settings, publish] = React.useState(initial);
  const latest = React.useRef(initial);
  React.useEffect(() => { latest.current = initial; publish(initial); }, [identity]);
  const update = (id: string, transform: (value: DashboardChartDisplaySettings) => DashboardChartDisplaySettings, persist: (value: Record<string, DashboardChartDisplaySettings>) => void) => {
    const next = setChartSettingsEntry(latest.current, id, transform(latest.current[id] ?? {}));
    latest.current = next;
    publish(next);
    persist(next);
  };
  return { settings, update };
}

function changeAxisSetting(current: DashboardChartDisplaySettings, axis: DashboardChartAxisId, action: { mode: DashboardChartAxisMode } | { bound: 'min' | 'max'; value: number | undefined }, ranges: DashboardChartResolvedAxisRanges = {}) {
  const setting = { ...current.axes?.[axis] };
  if ('mode' in action) {
    setting.mode = action.mode;
    const suggested = axis === 'x' ? undefined : ranges[axis];
    if (action.mode === 'manual' && suggested) {
      for (const [index, bound] of ['min', 'max'].entries()) if (setting[bound as 'min' | 'max'] == null) setting[bound as 'min' | 'max'] = suggested[index]!;
    }
  } else {
    setting.mode ??= 'manual';
    Object.assign(setting, { [action.bound]: action.value });
  }
  return { ...current, axes: { ...current.axes, [axis]: setting } };
}

export function DashboardChartWidget({
  instance,
  ctx,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}) {
  const options = readOptions(instance);
  const assignedPlacement =
    options.page === 'overview' ? 'overview' : options.page ?? ctx.dashboardSlug ?? null;
  const assignedCatalog = useEffectiveCharts(
    options.catalogMode === 'assigned' ? assignedPlacement : null
  );
  const chartOptions = getChartOptions(options.page).filter((option) =>
    options.chartIds.includes(option.value)
  );
  const defaultStorageKey = chartDefaultStorageKey(ctx, instance);
  const { data: favoriteResponse, isSuccess: favoritesLoaded } = useDashboardChartFavorites();
  const updateFavorite = useUpdateDashboardChartFavorite();
  const chartIdsSignature = options.chartIds.join('|');
  const serverFavorite = favoriteResponse?.chart_favorites?.[defaultStorageKey];
  const resolvedFavorite =
    typeof serverFavorite === 'string' && options.chartIds.includes(serverFavorite)
      ? serverFavorite
      : options.chartId;
  const { chartId, defaultChartId, setChartId, setDefaultChartId } = useChartChoice(options.chartId, `${defaultStorageKey}|${chartIdsSignature}|${options.chartId}`, resolvedFavorite, favoritesLoaded);
  const [search, setSearch] = React.useState('');
  const draft = useChartDraft(options.chartSettings, JSON.stringify(options.chartSettings));
  const draftChartSettings = draft.settings;
  const [activeAxisRanges, setActiveAxisRanges] = React.useState<DashboardChartResolvedAxisRanges>(
    {}
  );



  const activeChartId = options.chartIds.includes(chartId) ? chartId : options.chartId;
  const activeChartDefinition = getChartDefinition(activeChartId);
  const activeCapabilities = widgetChartCapabilities(activeChartDefinition);
  const activeSettings = resolveChartDisplaySettings(
    draftChartSettings,
    activeChartId,
    options.legacyTimeFilter,
    options.legacySmoothness
  );
  const activeChartTitle = activeChartDefinition?.title ?? instance.title ?? 'Chart';

  React.useEffect(() => {
    setActiveAxisRanges((current) => (Object.keys(current).length === 0 ? current : {}));
  }, [activeChartId]);

  if (options.catalogMode === 'assigned') {
    if (assignedCatalog.isSuccess) {
      return (
        <AssignedChartRuntime
          instance={instance}
          ctx={ctx}
          options={options}
          charts={assignedCatalog.data}
        />
      );
    }
    return (
      <DashboardChartCatalogState
        instance={instance}
        message={
          assignedCatalog.isError ? 'Unable to load assigned charts.' : 'Loading assigned charts…'
        }
        error={assignedCatalog.isError}
      />
    );
  }

  function updateActiveChartSettings(
    updater: (current: DashboardChartDisplaySettings) => DashboardChartDisplaySettings
  ) {
    draft.update(activeChartId, updater, (chartSettings) => ctx.updateWidgetOptions?.(instance.id, { chartSettings }));
}

  function setChartAsDefault(nextChartId: string) {
    setDefaultChartId(nextChartId);
    updateFavorite.mutate({ key: defaultStorageKey, chartId: nextChartId });
  }

  return (
    <DashboardChartFrame
      instance={instance}
      options={options}
      chartId={activeChartId}
      chartOptions={chartOptions}
      onChartChange={setChartId}
      search={search}
      onSearchChange={setSearch}
      defaultChartId={defaultChartId}
      onSetDefault={setChartAsDefault}
      chartTitle={activeChartTitle}
      settings={activeSettings}
      capabilities={activeCapabilities}
      activeAxisRanges={activeAxisRanges}
      onUpdateSettings={updateActiveChartSettings}
      persistent={Boolean(ctx.updateWidgetOptions)}
      renderChart={(height, presentation) => (
        <DashboardChartRenderer
          chartId={activeChartId}
          ctx={ctx}
          height={height}
          settings={activeSettings}
          presentation={presentation ?? 'embedded'}
          onResolvedAxisRanges={(ranges) =>
            setActiveAxisRanges((current) =>
              sameResolvedAxisRanges(current, ranges) ? current : ranges
            )
          }
        />
      )}
    />
  );
}

function DashboardChartCatalogState({
  instance,
  message,
  error = false,
}: {
  instance: WidgetInstance;
  message: string;
  error?: boolean;
}) {
  return (
    <div className="relative flex h-full min-h-0 flex-col overflow-hidden">
      {instance.title ? (
        <div className="mb-2 shrink-0">
          <p className="text-sm font-medium uppercase tracking-wider text-fg-secondary">
            {instance.title}
          </p>
        </div>
      ) : null}
      <div
        role={error ? 'alert' : 'status'}
        className="flex min-h-32 flex-1 items-center justify-center rounded-lg border border-dashed border-border p-4 text-sm text-fg-tertiary"
      >
        {message}
      </div>
    </div>
  );
}

interface DashboardChartFrameProps {
  instance: WidgetInstance;
  options: ResolvedDashboardChartOptions;
  chartId: string;
  chartOptions: Array<{ value: string; label: string }>;
  onChartChange: (value: string) => void;
  search: string;
  onSearchChange: (value: string) => void;
  defaultChartId: string;
  onSetDefault: (value: string) => void;
  chartTitle: string;
  settings: ReturnType<typeof resolveChartDisplaySettings>;
  capabilities: DashboardChartSettingsCapabilities;
  activeAxisRanges: DashboardChartResolvedAxisRanges;
  persistent: boolean;
  onUpdateSettings: (
    updater: (current: DashboardChartDisplaySettings) => DashboardChartDisplaySettings
  ) => void;
  renderChart: (height: number, presentation?: 'embedded' | 'mobile-viewer') => React.ReactNode;
}

/** Single visual frame shared by assigned and fixed dashboard chart catalogs. */
function DashboardChartFrame({
  instance,
  options,
  chartId,
  chartOptions,
  onChartChange,
  search,
  onSearchChange,
  defaultChartId,
  onSetDefault,
  chartTitle,
  settings,
  capabilities,
  activeAxisRanges,
  persistent,
  onUpdateSettings,
  renderChart,
}: DashboardChartFrameProps) {
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [viewerOpen, setViewerOpen] = React.useState(false);
  const settingsTriggerRef = React.useRef<HTMLButtonElement | null>(null);
  const expandTriggerRef = React.useRef<HTMLButtonElement | null>(null);
  const { ref, height } = useMeasuredWidgetHeight(260, 160);
  const controls = (
    <div className="flex items-center gap-2">
      <button
        ref={settingsTriggerRef}
        type="button"
        aria-label="Chart settings"
        aria-haspopup="dialog"
        aria-expanded={settingsOpen}
        onClick={() => setSettingsOpen((value) => !value)}
        className={cn(
          'flex h-11 w-11 shrink-0 sm:h-9 sm:w-9 items-center justify-center rounded-lg border border-border bg-bg-surface text-fg-tertiary transition-colors',
          'hover:border-border-strong hover:text-fg focus:outline-none focus:ring-0 focus-visible:!outline-none focus-visible:!outline-offset-0 focus-visible:ring-0',
          settingsOpen && 'border-accent text-accent'
        )}
      >
        <SlidersHorizontal className="h-4 w-4" />
      </button>
      <button
        ref={expandTriggerRef}
        type="button"
        aria-label="Expand chart"
        onClick={() => setViewerOpen(true)}
        className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-border bg-bg-surface text-fg-tertiary transition-colors hover:border-border-strong hover:text-fg focus:outline-none focus:ring-0 focus-visible:!outline-none focus-visible:!outline-offset-0 focus-visible:ring-0 sm:hidden"
      >
        <Maximize2 className="h-4 w-4" />
      </button>
    </div>
  );
  return (
    <div className="relative flex h-full min-h-0 flex-col overflow-hidden">
      {options.showPicker && chartOptions.length > 1 ? (
        <ChartPicker
          value={chartId}
          options={chartOptions}
          onChange={onChartChange}
          searchValue={search}
          onSearchChange={onSearchChange}
          className="shrink-0"
          trailing={controls}
          defaultValue={defaultChartId}
          onSetDefault={onSetDefault}
        />
      ) : instance.title ? (
        <div className="mb-2 flex shrink-0 items-start justify-between gap-3">
          <div>
            <p className="text-sm font-medium uppercase tracking-wider text-fg-secondary">
              {instance.title}
            </p>
            {options.headerSubtitle ? (
              <p className="mt-0.5 text-xs text-fg-tertiary">{options.headerSubtitle}</p>
            ) : null}
          </div>
          <div className="shrink-0">{controls}</div>
        </div>
      ) : (
        <div className="absolute right-0 top-0 z-10">{controls}</div>
      )}
      {!viewerOpen ? (
        <div ref={ref} className="min-h-0 flex-1 overflow-hidden">
          {renderChart(height)}
        </div>
      ) : null}
      <ChartSettingsPanel
        open={settingsOpen}
        triggerRef={settingsTriggerRef}
        chartTitle={chartTitle}
        capabilities={capabilities}
        settings={settings}
        suggestedRanges={activeAxisRanges}
        persistent={persistent}
        onClose={() => setSettingsOpen(false)}
        onTimeFilterChange={(next) =>
          onUpdateSettings((current) => ({ ...current, timeFilter: next }))
        }
        onSmoothnessChange={(next) =>
          onUpdateSettings((current) => ({ ...current, smoothness: next }))
        }
        onAxisModeChange={(axis, mode) => onUpdateSettings((current) => changeAxisSetting(current, axis, { mode }, activeAxisRanges))}
        onAxisValueChange={(axis, bound, value) => onUpdateSettings((current) => changeAxisSetting(current, axis, { bound, value }))}
      />
      {viewerOpen ? (
        <MobileChartViewer
          chartId={chartId}
          chartTitle={chartTitle}
          chartOptions={chartOptions}
          onChartChange={onChartChange}
          defaultChartId={defaultChartId}
          onSetDefault={onSetDefault}
          onClose={() => {
            setViewerOpen(false);
            requestAnimationFrame(() => expandTriggerRef.current?.focus());
          }}
        >
          {(viewerHeight) => renderChart(viewerHeight, 'mobile-viewer')}
        </MobileChartViewer>
      ) : null}
    </div>
  );
}

function AssignedChartRuntime({
  instance,
  ctx,
  options,
  charts,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
  options: ResolvedDashboardChartOptions;
  charts: ChartRecord[];
}) {
  const available = charts.filter(
    (chart) => (chart.isEnabled || options.requiredChartSlugs.includes(chart.slug))
      && (options.showPicker || options.chartIds.length !== 1 || options.chartIds.includes(chart.slug))
  );
  const chartIdsSignature = available.map((chart) => chart.slug).join('|');
  const chartOptions = available.map((chart) => ({ value: chart.slug, label: chart.name }));
  const defaultChartId =
    options.chartId && available.some((chart) => chart.slug === options.chartId)
      ? options.chartId
      : (available[0]?.slug ?? '');
  const [search, setSearch] = React.useState('');
  const draft = useChartDraft(options.chartSettings, options.chartSettings);
  const draftChartSettings = draft.settings;
  const [activeAxisRanges, setActiveAxisRanges] = React.useState<DashboardChartResolvedAxisRanges>(
    {}
  );
  const { data: favoriteResponse, isSuccess: favoritesLoaded } = useDashboardChartFavorites();
  const updateFavorite = useUpdateDashboardChartFavorite();
  const storageKey = chartDefaultStorageKey(ctx, instance);
  const favorite = favoriteResponse?.chart_favorites?.[storageKey];
  const resolvedFavorite =
    favoritesLoaded && favorite && available.some((chart) => chart.slug === favorite)
      ? favorite
      : defaultChartId;
  const { chartId, defaultChartId: defaultChartIdState, setChartId, setDefaultChartId } = useChartChoice(defaultChartId, `${storageKey}|${chartIdsSignature}|${defaultChartId}`, resolvedFavorite, available.length > 0 && favoritesLoaded);




  const active = available.find((chart) => chart.slug === chartId) ?? available[0];
  const settings = resolveChartDisplaySettings(
    draftChartSettings,
    active?.slug ?? '',
    options.legacyTimeFilter,
    options.legacySmoothness
  );
  const activeDefinition = active ? getChartDefinition(active.slug) : undefined;
  const activeCapabilities = widgetChartCapabilities(activeDefinition, active);

  function setChartAsDefault(next: string) {
    setDefaultChartId(next);
    updateFavorite.mutate({ key: storageKey, chartId: next });
  }

  function updateChartSettings(
    updater: (current: DashboardChartDisplaySettings) => DashboardChartDisplaySettings
  ) {
    if (active) draft.update(active.slug, updater, (chartSettings) => ctx.updateWidgetOptions?.(instance.id, { chartSettings }));
}

  if (!active) {
    return (
      <div className="flex h-full min-h-32 items-center justify-center rounded-lg border border-dashed border-border p-4 text-sm text-fg-tertiary">
        No enabled charts are assigned to this dashboard.
      </div>
    );
  }

  const renderChart = (height: number, presentation: 'embedded' | 'mobile-viewer' = 'embedded') =>
    active ? (
      <ManagedChartRuntime
        chart={active}
        ctx={ctx}
        height={height}
        settings={settings}
        presentation={presentation}
        onResolvedAxisRanges={(ranges) =>
          setActiveAxisRanges((current) =>
            sameResolvedAxisRanges(current, ranges) ? current : ranges
          )
        }
      />
    ) : null;
  return (
    <DashboardChartFrame
      instance={instance}
      options={options}
      chartId={active.slug}
      chartOptions={chartOptions}
      onChartChange={setChartId}
      search={search}
      onSearchChange={setSearch}
      defaultChartId={defaultChartIdState}
      onSetDefault={setChartAsDefault}
      chartTitle={active.name}
      settings={settings}
      capabilities={activeCapabilities}
      activeAxisRanges={activeAxisRanges}
      persistent={Boolean(ctx.updateWidgetOptions)}
      onUpdateSettings={updateChartSettings}
      renderChart={renderChart}
    />
  );
}

export function usesBundledChartRenderer(chart: ChartRecord) {
  return getChartDefinition(chart.slug) != null;
}

export function ManagedChartRuntime({
  chart,
  ctx,
  height,
  settings,
  presentation = 'embedded',
  onResolvedAxisRanges,
}: {
  chart: ChartRecord;
  ctx: WidgetCtx;
  height: number;
  settings?: DashboardChartDisplaySettings;
  presentation?: 'embedded' | 'mobile-viewer';
  onResolvedAxisRanges?: (ranges: DashboardChartResolvedAxisRanges) => void;
}) {
  const route = resolveManagedChartRoute(chart);
  const shared = { ctx, height, presentation, ...(settings ? { settings } : {}) };
  return route.kind === 'bundled' ? (
    <DashboardChartRenderer {...shared} chartId={route.chart.slug} managedDefinition={route.chart.config} {...(onResolvedAxisRanges ? { onResolvedAxisRanges } : {})} />
  ) : <ManagedDefinitionRuntime {...shared} chart={route.chart} />;
}

function resolveManagedChartRoute(chart: ChartRecord) {
  const normalized = normalizeLegacyBundledChartRecord(chart);
  return { kind: usesBundledChartRenderer(normalized) ? 'bundled' as const : 'definition' as const, chart: normalized };
}

function ManagedDefinitionRuntime({
  chart,
  ctx,
  height,
  settings,
  presentation,
}: {
  chart: ChartRecord;
  ctx: WidgetCtx;
  height: number;
  settings?: DashboardChartDisplaySettings;
  presentation: 'embedded' | 'mobile-viewer';
}) {
  const display = { ...chart.config.display };
  const overrides = { timeFilter: settings?.timeFilter, curveSmoothness: settings?.smoothness };
  for (const key of ['timeFilter', 'curveSmoothness'] as const) if (overrides[key]) Object.assign(display, { [key]: overrides[key] });
  const draft = { ...chart.config, display };
  const datasetState = useChartDatasets(draft, {
    vehicleId: ctx.vehicleId,
    from: ctx.from,
    to: ctx.to,
    lifetime: draft.timeframe.mode === 'lifetime',
    ...(ctx.chargeSessionId !== undefined ? { chargeSessionId: ctx.chargeSessionId } : {}),
    ...(ctx.tripTagFilter ? { tripTagFilter: ctx.tripTagFilter } : {}),
  });
  const renderer = getChartRenderer(draft);
  return (
    <>
      {renderer?.render({
        definition: draft,
        datasets: datasetState.datasets,
        height,
        loading: datasetState.isLoading,
        partial: datasetState.isPartial,
        refreshing: datasetState.isFetching && !datasetState.isLoading,
        error: datasetState.errors.length > 0 && datasetState.datasets.length === 0,
        presentation,
      })}
    </>
  );
}

export function DashboardChartRenderer({
  chartId,
  managedDefinition,
  ctx,
  height,
  timeFilter,
  settings,
  presentation = 'embedded',
  onResolvedAxisRanges,
}: {
  chartId: string;
  managedDefinition?: ChartDefinitionV1;
  ctx: WidgetCtx;
  height: number;
  timeFilter?: TimeFilterWindow;
  settings?: DashboardChartDisplaySettings;
  presentation?: 'embedded' | 'mobile-viewer';
  onResolvedAxisRanges?: (ranges: DashboardChartResolvedAxisRanges) => void;
}) {
  const definition = getChartDefinition(normalizeChartId(chartId));
  if (!definition) {
    return (
      <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary">
        Unknown chart: {chartId}
      </div>
    );
  }
  const controls = resolveWidgetOptions({
    timeFilter: definition.config.display.timeFilter ?? 'raw',
    smoothness: definition.config.display.curveSmoothness ?? DEFAULT_CURVE_SMOOTHNESS,
  }, { timeFilter: managedDefinition?.display.timeFilter, smoothness: managedDefinition?.display.curveSmoothness }, { timeFilter }, settings ?? {});
  return <ActiveDashboardChartSource definition={definition} chartDefinition={managedDefinition ?? definition.config} ctx={ctx} height={height} {...controls} presentation={presentation} {...(onResolvedAxisRanges ? { onResolvedAxisRanges } : {})} {...(settings ? { settings } : {})} />;
}

const CHARGE_SESSION_DETAIL_CONFIG = {
  schemaVersion: 1,
  placements: [],
  timeframe: { mode: 'dashboard' },
  sources: [
    {
      id: 'main',
      sourceId: 'charging.charge-curve',
      params: {},
      filters: [],
      inherit: { vehicle: true, timeframe: true },
    },
  ],
  x: {
    field: { sourceBindingId: 'main', field: 'elapsed_minutes' },
    kind: 'number',
  },
  series: [
    {
      id: 'energy_kwh',
      label: 'Energy Added',
      y: { sourceBindingId: 'main', field: 'energy_kwh' },
      mark: 'line',
      fill: true,
      yAxis: 'y2',
      color: { mode: 'token', token: 'emerald' },
      transforms: [],
      visibleInLegend: true,
    },
    {
      id: 'power_kw',
      label: 'Charge Rate',
      y: { sourceBindingId: 'main', field: 'power_kw' },
      mark: 'line',
      fill: false,
      yAxis: 'y',
      color: { mode: 'token', token: 'accent' },
      transforms: [],
      visibleInLegend: true,
    },
  ],
  axes: {
    x: { scale: 'linear', domain: { mode: 'auto' } },
    y: { scale: 'linear', unit: 'kW', domain: { mode: 'auto' } },
    y2: { scale: 'linear', unit: 'kWh', domain: { mode: 'auto' } },
  },
  display: {
    legend: 'auto',
    grid: true,
    tooltip: true,
    timeFilter: 'raw',
    curveSmoothness: 'gentle',
    emptyTitle: 'No charging curve is available for this session',
  },
  interaction: { panZoom: true, touchExplore: true, connectGaps: false },
} satisfies ChartDefinitionV1;

const CHARGE_SESSION_DETAIL_DEFINITION: DashboardChartDefinition = {
  id: 'charge-session-detail',
  title: 'Session charging trace',
  description: 'Charge rate and cumulative energy for the selected session.',
  pages: [],
  source: 'charge_session_curve',
  config: CHARGE_SESSION_DETAIL_CONFIG,
  mode: 'line',
  yUnit: 'kW',
  emptyTitle: CHARGE_SESSION_DETAIL_CONFIG.display.emptyTitle,
};

/** Route-only selected-session visualization; intentionally absent from the managed chart catalog. */
export function ChargeSessionCurveDetail({
  ctx,
  height,
  presentation = 'embedded',
}: {
  ctx: WidgetCtx;
  height: number;
  presentation?: 'embedded' | 'mobile-viewer';
}) {
  return (
    <ChargeSessionCurveSource
      definition={CHARGE_SESSION_DETAIL_DEFINITION}
      chartDefinition={CHARGE_SESSION_DETAIL_CONFIG}
      ctx={ctx}
      height={height}
      timeFilter="raw"
      smoothness="gentle"
      presentation={presentation}
    />
  );
}

type ActiveDashboardChartSourceProps = {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  ctx: WidgetCtx;
  height: number;
  timeFilter: TimeFilterWindow;
  smoothness: CurveSmoothness;
  settings?: DashboardChartDisplaySettings;
  presentation: 'embedded' | 'mobile-viewer';
  onResolvedAxisRanges?: (ranges: DashboardChartResolvedAxisRanges) => void;
};

type CanonicalViewProps = {
  definition: DashboardChartDefinition; chartDefinition: ChartDefinitionV1; loading: boolean; height: number;
  timeFilter: TimeFilterWindow; smoothness: CurveSmoothness; interactionMode: 'standard' | 'touch-explore';
  xRange?: [number, number]; yRange?: [number, number]; yRightRange?: [number, number];
  onResolvedAxisRanges?: (ranges: DashboardChartResolvedAxisRanges) => void;
};
function bindCanonicalChart<Data, Projection extends object>(
  useData: (ctx: WidgetCtx) => { data?: Data | undefined; isLoading: boolean },
  project: (data: Data | undefined) => Projection,
  View: React.ComponentType<CanonicalViewProps & Projection>,
  axes: 'none' | 'y' | 'xy' | 'all',
) {
  return function CanonicalSource(props: ActiveDashboardChartSourceProps) {
    const query = useData(props.ctx);
    const ranges = sourceAxisRanges(props.settings);
    const display: CanonicalViewProps = {
      definition: props.definition, chartDefinition: props.chartDefinition, loading: query.isLoading, height: props.height,
      timeFilter: props.timeFilter, smoothness: props.smoothness, interactionMode: chartInteractionMode(props.presentation),
    };
    if (axes !== 'none' && ranges.yRange) display.yRange = ranges.yRange;
    if ((axes === 'xy' || axes === 'all') && ranges.xRange) display.xRange = ranges.xRange;
    if (axes === 'all') {
      if (ranges.yRightRange) display.yRightRange = ranges.yRightRange;
      if (props.onResolvedAxisRanges) display.onResolvedAxisRanges = props.onResolvedAxisRanges;
    }
    return <View {...display} {...project(query.data)} />;
  };
}

function ActiveDashboardChartSource(props: ActiveDashboardChartSourceProps) {
  const View = CANONICAL_SOURCES[props.definition.source];
  return View ? <View {...props} /> : undefined;
}

function chartInteractionMode(presentation: ActiveDashboardChartSourceProps['presentation']) {
  return presentation === 'mobile-viewer' ? ('touch-explore' as const) : ('standard' as const);
}

function sourceAxisRanges(settings?: DashboardChartDisplaySettings) {
  return {
    xRange: getManualAxisRange(settings?.axes?.x),
    yRange: getManualAxisRange(settings?.axes?.y),
    yRightRange: getManualAxisRange(settings?.axes?.y2),
  };
}

function SocHistorySource({
  definition,
  chartDefinition,
  ctx,
  height,
  timeFilter,
  smoothness,
  settings,
  presentation,
}: ActiveDashboardChartSourceProps) {
  const isDark = useDocumentTheme();
  const { data: soc = [], isLoading: socLoading } = useSocHistory(ctx.vehicleId, ctx.from, ctx.to);
  const { data: range = [], isLoading: rangeLoading } = useRangeHistory(
    ctx.vehicleId,
    ctx.from,
    ctx.to
  );
  return renderSocHistoryChart(
    definition,
    chartDefinition,
    height,
    socLoading || rangeLoading,
    soc.map((point) => ({ ts: point.ts, value: point.value })),
    range.map((point) => ({ ts: point.ts, value: point.value })),
    timeFilter,
    smoothness,
    sourceAxisRanges(settings).yRange,
    chartInteractionMode(presentation),
    isDark
  );
}

function ChargingSeriesSource({
  definition,
  chartDefinition,
  ctx,
  height,
  settings,
  presentation,
  sessions,
}: ActiveDashboardChartSourceProps & { sessions?: boolean }) {
  const isDark = useDocumentTheme();
  const { data, isLoading } = useChargingChartSeries(ctx.vehicleId, ctx.from, ctx.to);
  const color = editedSeriesColor(chartDefinition, 'accent', isDark);
  const range = sourceAxisRanges(settings).yRange ?? fixedDefinitionRange(chartDefinition.axes.y);
  const View = sessions ? ChargingSessionsChart : DailyEnergyChart;
  const selection = sessions ? { selectedDayLocal: ctx.chargeSessionDayLocal ?? null,
    ...(ctx.setChargeSessionDayLocal ? { onDayClick: ctx.setChargeSessionDayLocal } : {}) } : {};
  return <View definition={definition} daily={data?.daily ?? []} dailySessions={data?.daily_sessions ?? []}
    loading={isLoading} height={height} interactionMode={chartInteractionMode(presentation)}
    {...selection} {...(color ? { seriesColor: color } : {})} {...(range ? { yRange: range } : {})} />;
}

function ChargeSessionCurveSource({
  definition,
  chartDefinition,
  ctx,
  height,
  timeFilter,
  smoothness,
  settings,
  presentation,
}: ActiveDashboardChartSourceProps) {
  const { data = [], isLoading } = useChargeCurve(
    ctx.chargeSessionId ?? null,
    ctx.vehicleId,
    ctx.chargeSessionActive === true,
  );
  const { yRange, yRightRange } = sourceAxisRanges(settings);
  return (
    <ChargeSessionCurveChart
      definition={definition}
      chartDefinition={chartDefinition}
      data={data}
      loading={isLoading}
      height={height}
      timeFilter={timeFilter}
      smoothness={smoothness}
      startedAt={ctx.from || null}
      sessionEnergyKwh={ctx.chargeSessionEnergyKwh ?? null}
      interactionMode={chartInteractionMode(presentation)}
      {...(yRange ? { yRange } : {})}
      {...(yRightRange ? { yRightRange } : {})}
    />
  );
}

const ChargingCurveAnalysisSource = bindCanonicalChart(function useSource(ctx) { return useChargeCurveAnalysis(ctx.vehicleId, ctx.from, ctx.to); }, (data) => ({ data: data ?? [] }), ChargingCurveAnalysisChart, 'xy');

const EfficiencyTrendSource = bindCanonicalChart(function useSource(ctx) { return useEfficiencyTrend(ctx.vehicleId, ctx.from, ctx.to, ctx.tripTagFilter); }, (data) => ({ trend: data ?? [] }), EfficiencyTrendChart, 'y');

const EfficiencyTemperatureSource = bindCanonicalChart(function useSource(ctx) { return useEfficiencyVsTemp(ctx.vehicleId, ctx.from, ctx.to, ctx.tripTagFilter); }, (data) => ({ data: data ?? [] }), EfficiencyTemperatureChart, 'none');

const EfficiencyModeSource = bindCanonicalChart(function useSource(ctx) { return useEfficiencyByMode(ctx.vehicleId, ctx.from, ctx.to, ctx.tripTagFilter); }, (data) => ({ data: data ?? [] }), EfficiencyModeChart, 'none');

function EfficiencyTagsSource({ chartDefinition, ctx, height }: ActiveDashboardChartSourceProps) {
  const { data = [], isLoading } = useEfficiencyByTag(
    ctx.vehicleId,
    ctx.from,
    ctx.to,
    ctx.tripTagFilter
  );
  const tags = useTripTags(ctx.vehicleId);
  if (tags.isError) {
    return <EfficiencyTagsCatalogError height={height} onRetry={() => void tags.refetch()} />;
  }
  if (!tags.isLoading && (tags.data ?? []).length === 0) {
    return <EfficiencyTagsOnboarding height={height} canManage={Boolean(ctx.canManageTripTags)} />;
  }
  const hasTagFilter = Boolean(ctx.tripTagFilter?.tagIds.length || ctx.tripTagFilter?.untagged);
  return (
    <EfficiencyTagsChart
      data={data}
      loading={isLoading || tags.isLoading}
      height={height}
      hasTagFilter={hasTagFilter}
      chartDefinition={chartDefinition}
    />
  );
}

function EfficiencyTagsCatalogError({ height, onRetry }: { height: number; onRetry: () => void }) {
  return (
    <div
      role="alert"
      className="flex flex-col items-center justify-center gap-3 rounded-lg border border-status-danger/30 bg-status-danger/10 px-4 text-center text-sm text-status-danger"
      style={{ height }}
    >
      <p>Couldn’t load shared tags. Retry to compare tagged trips.</p>
      <button
        type="button"
        className="min-h-11 rounded-lg border border-status-danger/40 px-3 text-sm font-medium transition-colors hover:bg-status-danger/10"
        onClick={onRetry}
      >
        Retry
      </button>
    </div>
  );
}

function EfficiencyTagsOnboarding({ height, canManage }: { height: number; canManage: boolean }) {
  return (
    <div
      className="flex items-center justify-center rounded-lg border border-dashed border-border px-4 text-center text-sm text-fg-tertiary"
      style={{ height }}
    >
      {canManage
        ? 'No shared tags yet. Create one with the Tags filter above to compare setups.'
        : 'No shared tags yet. A vehicle manager can create tags from Trips.'}
    </div>
  );
}

function PhantomDrainSource({
  definition,
  chartDefinition,
  ctx,
  height,
  settings,
  presentation,
}: ActiveDashboardChartSourceProps) {
  const isDark = useDocumentTheme();
  const { data, isLoading } = usePhantomDrainPeriods(ctx.vehicleId, ctx.from, ctx.to, 500, 6);
  const seriesColor = editedSeriesColor(chartDefinition, 'accent', isDark);
  return (
    <PhantomDrainChart
      periods={data?.periods ?? []}
      loading={isLoading}
      height={height}
      emptyTitle={definition.emptyTitle}
      yUnit={chartDefinition.axes.y.unit ?? definition.yUnit}
      yRange={
        sourceAxisRanges(settings).yRange ??
        fixedDefinitionRange(chartDefinition.axes.y) ??
        definition.yRange
      }
      interactionMode={chartInteractionMode(presentation)}
      {...(seriesColor ? { seriesColor } : {})}
    />
  );
}

function BatteryDegradationSource({
  definition,
  chartDefinition,
  ctx,
  height,
  timeFilter,
  smoothness,
  settings,
  presentation,
}: ActiveDashboardChartSourceProps) {
  const isDark = useDocumentTheme();
  const { data = [], isLoading } = useDegradation(ctx.vehicleId, ctx.from, ctx.to);
  return renderSingleChart(
    definition,
    chartDefinition,
    height,
    isLoading,
    data.map((point) => ({ ts: point.ts, value: point.capacity_pct ?? null })),
    timeFilter,
    sourceAxisRanges(settings).yRange,
    chartInteractionMode(presentation),
    smoothness,
    isDark
  );
}

function mileagePoints(data: Awaited<ReturnType<typeof useBatteryMileage>>['data']) {
  return (data ?? []).map((point) => ({
    ts: point.ts,
    x: point.odometer_mi,
    y: point.usable_kwh,
    rangeMi: point.range_mi,
    projectedMaxRangeMi: point.projected_max_range_mi,
    degradationPct: point.degradation_pct,
  }));
}

const BatteryMileageSource = bindCanonicalChart(function useSource(ctx) { return useBatteryMileage(ctx.vehicleId, ctx.from, ctx.to); }, (data) => ({ points: mileagePoints(data) }), BatteryCapacityMileageChart, 'all');

const ProjectedRangeMileageSource = bindCanonicalChart(function useSource(ctx) { return useBatteryMileage(ctx.vehicleId, ctx.from, ctx.to); }, (data) => ({ points: mileagePoints(data) }), ProjectedRangeMileageChart, 'all');

function TirePressureTripsSource({
  definition,
  chartDefinition,
  ctx,
  height,
  settings,
  presentation,
}: ActiveDashboardChartSourceProps) {
  const isDark = useDocumentTheme();
  const { data, isLoading } = useTirePressureTimeline(
    ctx.vehicleId,
    ctx.from,
    ctx.to,
    ctx.tripTagFilter
  );
  const { data: vehicles } = useVehicles();
  const navigate = useNavigate();
  const activeVehicle = vehicles?.find((vehicle) => vehicle.id === ctx.vehicleId);
  const unitPreferences = getUnitPreferences();
  const pressureFactor = unitPreferences.pressure_unit === 'kpa' ? 6.89476 : 1;
  const pressureUnit = unitPreferences.pressure_unit === 'kpa' ? 'kPa' : 'psi';
  const tireFields = {
    front_left_psi: { field: 'tire_fl_psi' as const, color: CHART_COLORS.accent, token: 'accent' },
    front_right_psi: { field: 'tire_fr_psi' as const, color: CHART_COLORS.sky, token: 'sky' },
    rear_left_psi: { field: 'tire_rl_psi' as const, color: CHART_COLORS.emerald, token: 'emerald' },
    rear_right_psi: { field: 'tire_rr_psi' as const, color: CHART_COLORS.amber, token: 'amber' },
  };
  const seriesStyles = chartDefinition.series.flatMap((series) => {
    const field = tireFields[series.y.field as keyof typeof tireFields];
    if (!field) return [];
    const style = canonicalSeriesStyle(series, isDark, field.token, field.color);
    delete style.stackId;
    return [{ ...style, field: field.field, key: series.id, label: series.label, mode: chartSeriesMode(series) }];
  });
  const configuredYRange =
    sourceAxisRanges(settings).yRange ?? fixedDefinitionRange(chartDefinition.axes.y);

  return (
    <TirePressureTripsChart
      samples={data?.samples ?? []}
      trips={data?.trips ?? []}
      targetPressure={activeVehicle?.target_tire_pressure_psi ?? DEFAULT_TARGET_TIRE_PRESSURE_PSI}
      pressureFactor={pressureFactor}
      pressureUnit={pressureUnit}
      loading={isLoading}
      height={height}
      {...(definition.emptyTitle ? { emptyTitle: definition.emptyTitle } : {})}
      interactionMode={chartInteractionMode(presentation)}
      seriesStyles={seriesStyles}
      {...(configuredYRange ? { yRange: configuredYRange } : {})}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      {...(chartDefinition.axes.y.label ? { yAxisLabel: chartDefinition.axes.y.label } : {})}
      onTripClick={(tripId) => void navigate({ to: '/trips/$tripId', params: { tripId } })}
    />
  );
}

function renderSocHistoryChart(
  definition: DashboardChartDefinition,
  chartDefinition: ChartDefinitionV1,
  height: number,
  loading: boolean,
  soc: Array<{ ts: string; value: number | null }>,
  range: Array<{ ts: string; value: number | null }>,
  timeFilter: TimeFilterWindow = 'raw',
  smoothness: CurveSmoothness = DEFAULT_CURVE_SMOOTHNESS,
  manualYRange?: [number, number],
  interactionMode: 'standard' | 'touch-explore' = 'standard',
  isDark = false
) {
  const projection = projectTimestampValues(soc, range);
  const configured = configuredRichSeries(
    chartDefinition,
    {
      battery_level: {
        defaultColorToken: 'accent',
        series: {
          key: definition.id,
          label: definition.title,
          color: CHART_COLORS.accent,
          values: projection.values,
          mode: definition.mode ?? 'line',
        },
      },
    },
    isDark
  );

  return (
    <RichTimeSeriesChart
      points={projection.points}
      series={[
        ...configured,
        {
          key: `${definition.id}-active-range`,
          label: 'Active Range',
          values: projection.related,
          tooltipOnly: true,
          tooltipFormatter: formatMiles,
        },
      ]}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      yUnit={chartDefinition.axes.y.unit ?? definition.yUnit}
      yRange={manualYRange ?? fixedDefinitionRange(chartDefinition.axes.y) ?? definition.yRange}
      mode={definition.mode}
      timeFilter={timeFilter}
      smoothness={smoothness}
      xAxisLabel={chartDefinition.axes.x.label}
      yAxisLabel={chartDefinition.axes.y.label}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      connectGaps={chartDefinition.interaction.connectGaps}
      interactionMode={interactionMode}
    />
  );
}

function renderSingleChart(
  definition: DashboardChartDefinition,
  chartDefinition: ChartDefinitionV1,
  height: number,
  loading: boolean,
  data: Array<{ ts: string; value: number | null }>,
  timeFilter: TimeFilterWindow = 'raw',
  manualYRange?: [number, number],
  interactionMode: 'standard' | 'touch-explore' = 'standard',
  smoothness: CurveSmoothness = DEFAULT_CURVE_SMOOTHNESS,
  isDark = false
) {
  const projection = projectTimestampValues(data);
  const configured = configuredRichSeries(
    chartDefinition,
    {
      capacity_pct: {
        defaultColorToken: 'accent',
        series: {
          key: definition.id,
          label: definition.title,
          color: CHART_COLORS.accent,
          values: projection.values,
          mode: definition.mode ?? 'line',
        },
      },
    },
    isDark
  );
  return (
    <RichTimeSeriesChart
      points={projection.points}
      series={configured}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      yUnit={chartDefinition.axes.y.unit ?? definition.yUnit}
      yRange={manualYRange ?? fixedDefinitionRange(chartDefinition.axes.y) ?? definition.yRange}
      mode={definition.mode}
      timeFilter={timeFilter}
      smoothness={smoothness}
      xAxisLabel={chartDefinition.axes.x.label}
      yAxisLabel={chartDefinition.axes.y.label}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      connectGaps={chartDefinition.interaction.connectGaps}
      interactionMode={interactionMode}
    />
  );
}

function ChargingSessionsChart({
  definition,
  daily,
  dailySessions,
  loading,
  height,
  selectedDayLocal,
  onDayClick,
  interactionMode,
  seriesColor,
  yRange,
}: {
  definition: DashboardChartDefinition;
  daily: Array<{
    day_local: string;
    day_start: string;
    total_energy_kwh: number;
    session_count: number;
  }>;
  dailySessions: Array<{
    session_id: string;
    day_local: string;
    day_start: string;
    started_at: string;
    energy_added_kwh: number | null;
    cost_usd: number | null;
    charger_type: string | null;
    location_name: string | null;
  }>;
  loading: boolean;
  height: number;
  selectedDayLocal?: string | null;
  onDayClick?: (dayLocal: string | null) => void;
  interactionMode: 'standard' | 'touch-explore';
  seriesColor?: string;
  yRange?: [number, number];
}) {
  return (
    <DailyChargeSessionsChart
      daily={daily}
      dailySessions={dailySessions}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      interactionMode={interactionMode}
      {...(selectedDayLocal !== undefined ? { selectedDayLocal } : {})}
      {...(onDayClick ? { onDayClick } : {})}
      {...(seriesColor ? { seriesColor } : {})}
      {...(yRange ? { yRange } : {})}
    />
  );
}

function ChargeSessionCurveChart({
  definition,
  chartDefinition,
  data,
  loading,
  height,
  timeFilter,
  smoothness,
  startedAt,
  sessionEnergyKwh,
  yRange,
  yRightRange,
  interactionMode,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  data: ChargeCurvePoint[];
  loading: boolean;
  height: number;
  timeFilter: TimeFilterWindow;
  smoothness: CurveSmoothness;
  startedAt: string | null;
  sessionEnergyKwh: number | null;
  yRange?: [number, number];
  yRightRange?: [number, number];
  interactionMode: 'standard' | 'touch-explore';
}) {
  const isDark = useDocumentTheme();
  const { points, rateValues, energyValues, useTime } = React.useMemo(() => prepareSessionCurve(data, startedAt, sessionEnergyKwh), [data, startedAt, sessionEnergyKwh]);
  const xSplits = React.useMemo(() => sessionHourSplits(points, useTime), [points, useTime]);

  const configured = configuredRichSeries(
    chartDefinition,
    {
      energy_kwh: {
        defaultColorToken: 'emerald',
        series: {
          key: 'energy',
          label: 'Energy Added',
          color: CHART_COLORS.emerald,
          values: energyValues,
          mode: 'area',
          yScale: 'y2',
          filterable: false,
        },
      },
      power_kw: {
        defaultColorToken: 'accent',
        series: {
          key: 'rate',
          label: 'Charge Rate',
          color: CHART_COLORS.accent,
          values: rateValues,
          yScale: 'y',
        },
      },
    },
    isDark
  );

  return (
    <RichTimeSeriesChart
      points={points}
      series={configured}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      xTime={useTime}
      xUnit={useTime ? undefined : '%'}
      yUnit={chartDefinition.axes.y.unit ?? 'kW'}
      yRightUnit={chartDefinition.axes.y2?.unit ?? 'kWh'}
      yRange={yRange ?? fixedDefinitionRange(chartDefinition.axes.y)}
      yRightRange={yRightRange ?? fixedDefinitionRange(chartDefinition.axes.y2)}
      mode="line"
      xAxisLabel={chartDefinition.axes.x.label}
      yAxisLabel={chartDefinition.axes.y.label}
      yRightAxisLabel={chartDefinition.axes.y2?.label}
      xValueFormatter={useTime ? undefined : (value) => `${Math.round(value)}%`}
      xSplits={xSplits}
      timeFilter={timeFilter}
      smoothness={smoothness}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      connectGaps={chartDefinition.interaction.connectGaps}
      interactionMode={interactionMode}
    />
  );
}

function ChargingCurveAnalysisChart({
  definition,
  chartDefinition,
  data,
  loading,
  height,
  xRange,
  yRange,
  interactionMode,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  data: ChargeCurveAnalysisPoint[];
  loading: boolean;
  height: number;
  xRange?: [number, number];
  yRange?: [number, number];
  interactionMode: 'standard' | 'touch-explore';
}) {
  const isDark = useDocumentTheme();
  const [mode, setMode] = React.useState<ChargeCurveMode>('observed');
  const isMobile = useCompatibleWidgetMedia('(max-width: 639px)');
  const plot = React.useMemo(() => buildChargeCurvePlot(data, mode), [data, mode]);


  const nextMode = nextChargeCurveMode(mode);
  const nextModeLabel = chargeCurveModeLabel(nextMode);
  const primarySeries = chartDefinition.series[0];
  const observedColor = primarySeries
    ? canonicalSeriesStyle(primarySeries, isDark, 'accent', CHART_COLORS.accent).color
    : CHART_COLORS.accent;

  return (
    <div className="relative h-full min-h-0">
      <RichTimeSeriesChart
        points={plot.rows.map((row) => ({ ts: row.plotSoc }))}
        series={[
          ...buildChargeCurveScatterSeries(
            plot.rows,
            observedColor,
            primarySeries?.label ?? 'Verified DC sessions'
          ),
          ...(plot.hasEstimatedHistory
            ? [
                {
                  key: 'dc-estimated-history',
                  label: 'Estimated history',
                  color: CHART_COLORS.amber,
                  mode: 'scatter' as const,
                  values: plot.rows.map((row) => row.estimatedKw),
                  tooltipDetails: plot.rows.map((row) => row.estimatedDetail),
                  pointSize: 6,
                },
              ]
            : []),
          ...(mode === 'off'
            ? []
            : ([
                {
                  key: 'dc-summary',
                  label: mode === 'observed' ? 'Observed trend' : 'Best observed trend (P75)',
                  color: CHART_COLORS.orange,
                  mode: 'line' as const,
                  values: plot.rows.map((row) => row.summaryKw),
                  tooltipDetails: plot.rows.map((row) => row.summaryDetail),
                },
              ] as const)),
        ]}
        loading={loading}
        emptyTitle={definition.emptyTitle}
        height={height}
        xTime={false}
        xUnit="%"
        yUnit="kW"
        xRange={xRange ?? fixedDefinitionRange(chartDefinition.axes.x) ?? [0, 100]}
        yRange={yRange ?? fixedDefinitionRange(chartDefinition.axes.y)}
        mode="scatter"
        connectGaps
        xSplits={isMobile ? [0, 20, 40, 60, 80, 100] : [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]}
        xValueFormatter={(value) => `${Math.round(value)}%`}
        interactionMode={interactionMode}
        xAxisLabel={chartDefinition.axes.x.label}
        yAxisLabel={chartDefinition.axes.y.label}
        showLegend={chartDefinition.display.legend !== 'hide'}
        showGrid={chartDefinition.display.grid}
        showTooltip={chartDefinition.display.tooltip}
        showPoints={chartDefinition.display.showPoints ?? true}
      />
      <button
        type="button"
        aria-label={`Switch to ${nextModeLabel}`}
        title={`Switch to ${nextModeLabel}`}
        onClick={() => setMode(nextMode)}
        className="absolute right-3 top-3 z-10 rounded-md border border-border bg-bg-surface/95 px-2.5 py-1.5 text-xs font-medium text-fg shadow-sm transition-colors hover:bg-bg-elevated"
      >
        Trend: {chargeCurveModeLabel(mode)}
      </button>
    </div>
  );
}

type ChargeCurveMode = 'off' | 'observed' | 'best-observed';

type ChargeCurvePlotRow = {
  socPct: number;
  plotSoc: number;
  observedKw: number | null;
  observedDetail: string | null;
  estimatedKw: number | null;
  estimatedDetail: string | null;
  summaryKw: number | null;
  summaryDetail: string | null;
};

type CurvePoint = ChargeCurveAnalysisPoint & {
  soc_pct: number;
  charge_rate_kw: number;
  sample_source: 'telemetry' | 'telemetry_1min' | 'rivian_charge_curve_points';
  power_method: 'recorded' | 'soc_delta';
};

function normalizeSampleSource(value: unknown): CurvePoint['sample_source'] {
  const source = typeof value === 'string' ? value : '';
  if (source === 'rivian_charge_curve_points' || source === 'telemetry_1min') return source;
  return 'telemetry';
}

export function buildChargeCurvePlot(data: ChargeCurveAnalysisPoint[], mode: ChargeCurveMode) {
  const observed: CurvePoint[] = [], estimated: CurvePoint[] = [];
  for (const sample of data) {
    if (!Number.isFinite(sample.soc_pct) || !Number.isFinite(sample.charge_rate_kw) || sample.charge_rate_kw <= 0 || normalizeChargeCurveType(sample.charger_type) !== 'dc') continue;
    const point: CurvePoint = { ...sample, soc_pct: sample.soc_pct!, sample_source: normalizeSampleSource(sample.sample_source), power_method: sample.power_method === 'recorded' ? 'recorded' : 'soc_delta' };
    (point.sample_source === 'rivian_charge_curve_points' ? estimated : observed).push(point);
  }
  const row = (socPct: number): ChargeCurvePlotRow => ({ socPct, plotSoc: socPct, observedKw: null, observedDetail: null, estimatedKw: null, estimatedDetail: null, summaryKw: null, summaryDetail: null });
  const rows: ChargeCurvePlotRow[] = [];
  for (const group of [observed, estimated]) {
    for (const point of group) {
      const result = row(point.soc_pct);
      const historical = group === estimated;
      result[historical ? 'estimatedKw' : 'observedKw'] = point.charge_rate_kw;
      result[historical ? 'estimatedDetail' : 'observedDetail'] = formatExactSoc(point.soc_pct) + ' SoC; ' + (historical ? 'Recorded kW; estimated SoC (excluded from summaries)' : powerMethodLabel([point]));
      rows.push(result);
    }
  }
  for (const trend of buildChargeCurveRegression(observed, mode)) rows.push({ ...row(trend.socPct), summaryKw: trend.powerKw, summaryDetail: formatExactSoc(trend.socPct) + ' SoC; ' + trend.sampleCount + ' nearby samples; ' + (mode === 'best-observed' ? 'local upper-quartile regression' : 'local weighted regression') });
  return { rows: distributeChargeCurveSoc(rows), hasEstimatedHistory: estimated.length > 0 };
}

const CHARGE_CURVE_REGRESSION_RADIUS_SOC = 8;

export function buildChargeCurveRegression(points: CurvePoint[], mode: ChargeCurveMode) {
  if (points.length < 3) return [];
  const sorted = points.map((point, order) => ({ point, order })).sort((a, b) => a.point.soc_pct - b.point.soc_pct);
  const lower = Math.ceil(sorted[0]!.point.soc_pct), upper = Math.floor(sorted.at(-1)!.point.soc_pct);
  const trend: Array<{ socPct: number; powerKw: number; sampleCount: number }> = [];
  let begin = 0, end = 0;
  for (let socPct = lower; socPct <= upper; socPct++) {
    while (begin < sorted.length && sorted[begin]!.point.soc_pct <= socPct - CHARGE_CURVE_REGRESSION_RADIUS_SOC) begin++;
    while (end < sorted.length && sorted[end]!.point.soc_pct < socPct + CHARGE_CURVE_REGRESSION_RADIUS_SOC) end++;
    if (end - begin < 3) continue;
    let nearby = sorted.slice(begin, end).sort((a, b) => a.order - b.order).map(({ point }) => ({ x: point.soc_pct, y: point.charge_rate_kw, weight: (1 - (Math.abs(point.soc_pct - socPct) / CHARGE_CURVE_REGRESSION_RADIUS_SOC) ** 3) ** 3 }));
    if (mode === 'best-observed') { const threshold = weightedPercentile(nearby, 0.75); nearby = nearby.filter((sample) => sample.y >= threshold); }
    const powerKw = weightedLinearPrediction(nearby, socPct);
    if (powerKw != null) trend.push({ socPct, powerKw, sampleCount: nearby.length });
  }
  return trend;
}

function weightedLinearPrediction(
  points: Array<{ x: number; y: number; weight: number }>,
  atX: number
) {
  const moments = [0, 0, 0, 0, 0];
  for (const point of points) {
    const contributions = [point.weight, point.weight * point.x, point.weight * point.y, point.weight * point.x * point.x, point.weight * point.x * point.y];
    for (let index = 0; index < moments.length; index++) moments[index]! += contributions[index]!;
  }
  const [weight, x, y, xx, xy] = moments as [number, number, number, number, number];
  if (weight === 0) return null;
  const determinant = weight * xx - x ** 2;
  if (Math.abs(determinant) < Number.EPSILON) return Math.max(0, y / weight);
  const slope = (weight * xy - x * y) / determinant;
  return Math.max(0, (y - slope * x) / weight + slope * atX);
}

function weightedPercentile(points: Array<{ y: number; weight: number }>, quantile: number) {
  if (!points.length) return 0;
  const ordered = points.slice().sort((a, b) => a.y - b.y);
  let weight = 0;
  const cumulative = ordered.map((point) => (weight += point.weight));
  const target = weight * quantile;
  let low = 0, high = cumulative.length;
  while (low < high) { const middle = (low + high) >>> 1; if (cumulative[middle]! >= target) high = middle; else low = middle + 1; }
  return ordered[Math.min(low, ordered.length - 1)]!.y;
}

export function distributeChargeCurveSoc(rows: ChargeCurvePlotRow[]) {
  const groups = new Map<number, ChargeCurvePlotRow[]>();
  rows.forEach((row) => { const group = groups.get(row.socPct); if (group) group.push(row); else groups.set(row.socPct, [row]); });
  return [...groups].sort((a, b) => a[0] - b[0]).flatMap(([soc, group]) => {
    const spacing = Math.min(0.04, 0.2 / group.length), midpoint = (group.length - 1) / 2;
    return group.map((row, index) => ({ ...row, plotSoc: soc + (index - midpoint) * spacing }));
  });
}

function formatExactSoc(soc: number) {
  return Number.isInteger(soc) ? String(soc) : soc.toFixed(1).replace(/\.0$/, '');
}

function nextChargeCurveMode(mode: ChargeCurveMode): ChargeCurveMode {
  if (mode === 'observed') return 'best-observed';
  if (mode === 'best-observed') return 'off';
  return 'observed';
}

function chargeCurveModeLabel(mode: ChargeCurveMode) {
  if (mode === 'best-observed') return 'Best observed';
  if (mode === 'off') return 'Off';
  return 'Observed';
}

const CHARGE_CURVE_SCATTER_BUCKETS = 6;

function buildChargeCurveScatterSeries(
  rows: ChargeCurvePlotRow[],
  startColor: string = CHART_COLORS.accent,
  label = 'Verified DC sessions'
) {
  const groups = scatterBuckets(rows.map((row) => row.observedKw), CHARGE_CURVE_SCATTER_BUCKETS);
  const colors = scatterPalette(startColor, CHART_COLORS.emerald, CHARGE_CURVE_SCATTER_BUCKETS);
  return groups.map((indices, bucket) => {
    const values = Array<number | null>(rows.length).fill(null);
    const tooltipDetails = Array<string | null>(rows.length).fill(null);
    for (const index of indices) {
      values[index] = rows[index]!.observedKw;
      tooltipDetails[index] = rows[index]!.observedDetail;
    }
    return { key: 'dc-observed-samples-' + bucket, label, color: colors[bucket]!, mode: 'scatter' as const, showInLegend: false, pointSize: 6, values, tooltipDetails };
  });
}

function powerMethodLabel(points: CurvePoint[]) {
  const methods = new Set(points.map((point) => point.power_method));
  if (methods.size === 1 && methods.has('recorded')) return 'Recorded kW';
  if (methods.size === 1) return 'SoC/time estimate';
  return 'Recorded and SoC/time estimates';
}

function normalizeChargeCurveType(chargerType: ChargeCurveAnalysisPoint['charger_type']) {
  const normalized = chargerType as string | null;
  if (normalized === 'dc' || normalized === 'dcfc') return 'dc';
  if (normalized === 'ac' || normalized === 'ac_l2') return 'ac';
  return 'unknown';
}

function DailyEnergyChart({
  definition,
  daily,
  loading,
  height,
  yRange,
  interactionMode,
  seriesColor,
}: {
  definition: DashboardChartDefinition;
  daily: Array<{
    day_local: string;
    day_start: string;
    total_energy_kwh: number;
    session_count: number;
  }>;
  loading: boolean;
  height: number;
  yRange?: [number, number];
  interactionMode: 'standard' | 'touch-explore';
  seriesColor?: string;
}) {
  return (
    <DailyEnergyBarChart
      daily={daily}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      yRange={yRange}
      interactionMode={interactionMode}
      {...(seriesColor ? { seriesColor } : {})}
    />
  );
}

function EfficiencyTrendChart({
  definition,
  chartDefinition,
  trend,
  loading,
  height,
  timeFilter,
  smoothness,
  yRange,
  interactionMode,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  trend: Array<{
    ts: string;
    trip_efficiency_wh_mi: number | null;
    rolling_24h_wh_mi: number | null;
  }>;
  loading: boolean;
  height: number;
  timeFilter: TimeFilterWindow;
  smoothness: CurveSmoothness;
  yRange?: [number, number];
  interactionMode: 'standard' | 'touch-explore';
}) {
  const isDark = useDocumentTheme();
  const timeline = efficiencyTimeline(trend);
  const configured = configuredRichSeries(
    chartDefinition,
    {
      trip_efficiency_wh_mi: {
        defaultColorToken: 'accent',
        series: {
          key: 'trip',
          label: 'Trip efficiency',
          color: CHART_COLORS.accent,
          values: timeline.values.trip_efficiency_wh_mi,
        },
      },
      rolling_24h_wh_mi: {
        defaultColorToken: 'emerald',
        series: {
          key: 'rolling',
          label: '24-hour avg',
          color: CHART_COLORS.emerald,
          values: timeline.values.rolling_24h_wh_mi,
        },
      },
    },
    isDark
  );
  return (
    <RichTimeSeriesChart
      points={timeline.points}
      series={configured}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      yUnit={timeline.unit}
      yRange={yRange}
      mode={definition.mode}
      timeFilter={timeFilter}
      smoothness={smoothness}
      xAxisLabel={chartDefinition.axes.x.label}
      yAxisLabel={chartDefinition.axes.y.label}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      connectGaps={chartDefinition.interaction.connectGaps}
      interactionMode={interactionMode}
    />
  );
}

function EfficiencyTemperatureChart({
  definition,
  chartDefinition,
  data,
  loading,
  height,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  data: Array<{
    temp_c_low: number;
    temp_c_high: number;
    avg_efficiency_wh_mi: number | null;
    total_miles?: number | null;
    avg_speed_mph?: number | null;
  }>;
  loading: boolean;
  height: number;
}) {
  const isDark = useDocumentTheme();
  const preferences = getUnitPreferences();
  const distanceFactor = preferences.distance_unit === 'kilometers' ? 1.609344 : 1;
  const speedFactor = preferences.speed_unit === 'kmh' ? 1.609344 : 1;
  const seriesColor = editedSeriesColor(chartDefinition, 'accent', isDark);
  const points = efficiencyRows(data, (row) => row.avg_efficiency_wh_mi, (row) => ({
    label: formatTemp(row.temp_c_low), distance: typeof row.total_miles === 'number' ? row.total_miles * distanceFactor : null,
    speed: typeof row.avg_speed_mph === 'number' ? row.avg_speed_mph * speedFactor : null,
  }), (left, right) => right.temp_c_low - left.temp_c_low);

  return (
    <EfficiencyPillBarChart
      data={points}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      valueUnit={getEfficiencyUnit()}
      distanceUnit={preferences.distance_unit === 'kilometers' ? 'km' : 'mi'}
      speedUnit={preferences.speed_unit === 'kmh' ? 'km/h' : 'mph'}
      {...(seriesColor ? { seriesColor } : {})}
    />
  );
}

function EfficiencyModeChart({
  definition,
  chartDefinition,
  data,
  loading,
  height,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  data: Array<{ drive_mode: string; avg_efficiency: number | null; trip_count?: number | null }>;
  loading: boolean;
  height: number;
}) {
  const isDark = useDocumentTheme();
  const seriesColor = editedSeriesColor(chartDefinition, 'accent', isDark);
  const rows = efficiencyRows(data, (row) => row.avg_efficiency, (row) => ({ label: formatDriveModeLabel(row.drive_mode), count: typeof row.trip_count === 'number' ? row.trip_count : null }));

  return (
    <EfficiencyPillBarChart
      data={rows}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      valueUnit={getEfficiencyUnit()}
      {...(seriesColor ? { seriesColor } : {})}
    />
  );
}

function EfficiencyTagsChart({
  chartDefinition,
  data,
  loading,
  height,
  hasTagFilter,
}: {
  chartDefinition: ChartDefinitionV1;
  data: Array<{
    tag_id: string | null;
    tag_name: string;
    trip_count: number;
    total_miles: number;
    efficiency_miles: number;
    avg_efficiency_wh_mi: number | null;
    coverage: number;
  }>;
  loading: boolean;
  height: number;
  hasTagFilter: boolean;
}) {
  const isDark = useDocumentTheme();
  const preferences = getUnitPreferences();
  const distanceFactor = preferences.distance_unit === 'kilometers' ? 1.609344 : 1;
  const seriesColor = editedSeriesColor(chartDefinition, 'accent', isDark);
  const rows = efficiencyRows(data, (row) => row.avg_efficiency_wh_mi, (row) => ({ label: row.tag_name, count: row.trip_count, distance: row.total_miles * distanceFactor, coverage: row.coverage, tone: row.tag_id == null ? 'neutral' as const : 'accent' as const }));
  const emptyTitle = hasTagFilter
    ? 'No trips match these tag filters. Clear filters above or choose another range.'
    : data.length > 0 ? 'Trips in this range do not have efficiency readings yet.' : 'No trips in this period.';

  return (
    <EfficiencyPillBarChart
      data={rows}
      loading={loading}
      emptyTitle={emptyTitle}
      height={height}
      valueUnit={getEfficiencyUnit()}
      wideLabels
      distanceUnit={preferences.distance_unit === 'kilometers' ? 'km' : 'mi'}
      {...(seriesColor ? { seriesColor } : {})}
    />
  );
}

function formatDriveModeLabel(value: string) {
  return formatDriveMode(value);
}

type EstablishedRichSeries = { series: RichSeries; defaultColorToken?: string };

function editedSeriesColor(
  definition: ChartDefinitionV1,
  defaultColorToken: string,
  isDark: boolean
) {
  const spec = definition.series[0];
  if (!spec?.color) return undefined;
  const retained = spec.color.mode === 'token' && spec.color.token === defaultColorToken;
  return retained ? undefined : canonicalSeriesStyle(spec, isDark, '', '').color;
}

function chartDefinitionPlan(definition: ChartDefinitionV1, fields: Record<string, EstablishedRichSeries>, dark: boolean) {
  const plan: Array<{ spec: ChartDefinitionV1['series'][number]; established: EstablishedRichSeries; style: Partial<RichSeries> }> = [];
  for (const spec of definition.series) {
    const established = fields[spec.y.field];
    if (!established) continue;
    plan.push({ spec, established, style: canonicalSeriesStyle(spec, dark, established.defaultColorToken ?? '', established.series.color ?? getChartColor('accent')) });
  }
  return plan;
}

function finiteExtent(values: Array<number | null | undefined>) {
  let minimum = Infinity, maximum = -Infinity;
  for (const value of values) if (value != null && Number.isFinite(value)) { minimum = Math.min(minimum, value); maximum = Math.max(maximum, value); }
  return maximum === -Infinity ? undefined : [minimum, maximum] as [number, number];
}

function configuredRichSeries(
  definition: ChartDefinitionV1,
  establishedByField: Record<string, EstablishedRichSeries>,
  isDark: boolean
) {
  const plan = chartDefinitionPlan(definition, establishedByField, isDark);
  return plan.map(({ spec, established, style }) => ({ ...established.series, ...style, key: established.series.key || spec.id, yScale: spec.yAxis }));
}

function chartSeriesMode(
  series: ChartDefinitionV1['series'][number]
): NonNullable<RichSeries['mode']> {
  return chartMark(series);
}

function fixedDefinitionRange(
  axis: ChartDefinitionV1['axes']['y'] | ChartDefinitionV1['axes']['x'] | undefined
) {
  return chartDomain(axis);
}

function BatteryCapacityMileageChart({
  definition,
  chartDefinition,
  points,
  loading,
  height,
  timeFilter,
  smoothness,
  xRange,
  yRange,
  yRightRange,
  interactionMode,
  onResolvedAxisRanges,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  points: Array<{
    ts: string;
    x: number | null;
    y: number | null;
    rangeMi: number | null;
    projectedMaxRangeMi: number | null;
    degradationPct: number | null;
  }>;
  loading: boolean;
  height: number;
  timeFilter: TimeFilterWindow;
  smoothness: CurveSmoothness;
  xRange?: [number, number];
  yRange?: [number, number];
  yRightRange?: [number, number];
  interactionMode: 'standard' | 'touch-explore';
  onResolvedAxisRanges?: (ranges: DashboardChartResolvedAxisRanges) => void;
}) {
  const isDark = useDocumentTheme();
  const useTime = chartDefinition.x.kind === 'time';
  const rows = mileageProjection(points, chartDefinition, 'capacity');
  const series = configuredRichSeries(
    chartDefinition,
    {
      usable_kwh: {
        defaultColorToken: 'accent',
        series: {
          key: 'usable-capacity',
          label: 'Usable Capacity',
          color: CHART_COLORS.accent,
          mode: 'area',
          values: rows.map((point) => point.y),
          yScale: 'y',
        },
      },
      odometer_miles: {
        defaultColorToken: 'emerald',
        series: {
          key: 'odometer-mi',
          label: 'Mileage',
          color: CHART_COLORS.emerald,
          mode: 'line',
          values: rows.map((point) => point.x),
          yScale: 'y2',
          filterable: false,
        },
      },
      range_mi: {
        series: { key: 'range-mi', label: 'Range', values: rows.map((point) => point.rangeMi) },
      },
      projected_max_range_mi: {
        series: {
          key: 'projected-range-mi',
          label: 'Projected Range',
          values: rows.map((point) => point.projectedMaxRangeMi),
        },
      },
      degradation_pct: {
        series: {
          key: 'degradation-pct',
          label: 'Degradation',
          values: rows.map((point) => point.degradationPct),
        },
      },
    },
    isDark
  );
  const yValues = (series.find((item) => item.yScale === 'y')?.values ?? []).filter(
    (value): value is number => value != null && Number.isFinite(value)
  );
  const yPrecision = getBatteryCapacityMileagePrecision(yValues);
  const autoYRange = getBatteryCapacityMileageYRange(yValues);
  const primaryUsesCapacity = chartDefinition.series.some(
    (item) => item.yAxis === 'y' && item.y.field === 'usable_kwh'
  );

  return (
    <RichTimeSeriesChart
      points={rows.map((point) => ({ ts: useTime ? point.ts : point.domain }))}
      series={series}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      yUnit={chartDefinition.axes.y.unit ?? definition.yUnit}
      yRange={
        yRange ??
        fixedDefinitionRange(chartDefinition.axes.y) ??
        (primaryUsesCapacity ? autoYRange : undefined) ??
        definition.yRange
      }
      yRightUnit={chartDefinition.axes.y2?.unit ?? 'mi'}
      yRightRange={yRightRange ?? fixedDefinitionRange(chartDefinition.axes.y2)}
      mode={definition.mode}
      xTime={useTime}
      xUnit={chartDefinition.axes.x.unit}
      xRange={xRange ?? fixedDefinitionRange(chartDefinition.axes.x)}
      xAxisLabel={chartDefinition.axes.x.label}
      yAxisLabel={chartDefinition.axes.y.label}
      yRightAxisLabel={chartDefinition.axes.y2?.label}
      timeFilter={timeFilter}
      smoothness={smoothness}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      connectGaps={chartDefinition.interaction.connectGaps}
      yAxisValueFormatter={(value, unit) => formatChartNumber(value, unit, 0)}
      yRightAxisValueFormatter={(value, unit) => formatChartNumber(value, unit, 0)}
      yValueFormatter={(value, unit) => formatChartNumber(value, unit, yPrecision)}
      interactionMode={interactionMode}
      onResolvedAxisRanges={onResolvedAxisRanges}
    />
  );
}

export function getBatteryCapacityMileageYRange(values: Array<number | null | undefined>) {
  const extent = finiteExtent(values);
  return extent ? [0, extent[1] > 0 ? Math.ceil(extent[1] * 1.1) : 1] as [number, number] : undefined;
}

function ProjectedRangeMileageChart({
  definition,
  chartDefinition,
  points,
  loading,
  height,
  timeFilter,
  smoothness,
  xRange,
  yRange: manualYRange,
  yRightRange,
  interactionMode,
  onResolvedAxisRanges,
}: {
  definition: DashboardChartDefinition;
  chartDefinition: ChartDefinitionV1;
  points: Array<{
    ts: string;
    rangeMi: number | null;
    projectedMaxRangeMi: number | null;
    x: number | null;
  }>;
  loading: boolean;
  height: number;
  timeFilter: TimeFilterWindow;
  smoothness: CurveSmoothness;
  xRange?: [number, number];
  yRange?: [number, number];
  yRightRange?: [number, number];
  interactionMode: 'standard' | 'touch-explore';
  onResolvedAxisRanges?: (ranges: DashboardChartResolvedAxisRanges) => void;
}) {
  const isDark = useDocumentTheme();
  const useTime = chartDefinition.x.kind === 'time';
  const rows = mileageProjection(points, chartDefinition, 'range');
  const series = configuredRichSeries(
    chartDefinition,
    {
      projected_max_range_mi: {
        defaultColorToken: 'amber',
        series: {
          key: definition.id,
          label: 'Projected Max Range',
          values: rows.map((point) => point.projectedRangeMi),
          color: CHART_COLORS.amber,
          mode: 'area',
        },
      },
      odometer_miles: {
        defaultColorToken: 'emerald',
        series: {
          key: 'odometer-mi',
          label: 'Mileage',
          values: rows.map((point) => point.odometerMi),
          color: CHART_COLORS.emerald,
          mode: 'line',
          yScale: 'y2',
        },
      },
    },
    isDark
  );
  const yRange =
    manualYRange ??
    fixedDefinitionRange(chartDefinition.axes.y) ??
    getProjectedRangeMileageYRange(series.find((item) => item.yScale !== 'y2')?.values ?? []);

  return (
    <RichTimeSeriesChart
      points={rows.map((point) => ({ ts: useTime ? point.ts : point.domain }))}
      series={series}
      loading={loading}
      emptyTitle={definition.emptyTitle}
      height={height}
      yUnit={chartDefinition.axes.y.unit ?? definition.yUnit}
      yRange={yRange ?? fixedDefinitionRange(chartDefinition.axes.y)}
      yRightUnit={chartDefinition.axes.y2?.unit ?? 'mi'}
      yRightRange={yRightRange ?? fixedDefinitionRange(chartDefinition.axes.y2)}
      mode={definition.mode}
      xTime={useTime}
      xUnit={chartDefinition.axes.x.unit}
      xRange={xRange ?? fixedDefinitionRange(chartDefinition.axes.x)}
      xAxisLabel={chartDefinition.axes.x.label}
      yAxisLabel={chartDefinition.axes.y.label}
      yRightAxisLabel={chartDefinition.axes.y2?.label}
      timeFilter={timeFilter}
      smoothness={smoothness}
      showLegend={chartDefinition.display.legend !== 'hide'}
      showGrid={chartDefinition.display.grid}
      showTooltip={chartDefinition.display.tooltip}
      showPoints={chartDefinition.display.showPoints ?? false}
      connectGaps={chartDefinition.interaction.connectGaps}
      interactionMode={interactionMode}
      onResolvedAxisRanges={onResolvedAxisRanges}
    />
  );
}

export function getProjectedRangeMileageYRange(values: Array<number | null | undefined>) {
  const extent = finiteExtent(values);
  return extent ? [200, Math.max(225, Math.ceil(extent[1] / 25) * 25)] as [number, number] : undefined;
}

function getBatteryCapacityMileagePrecision(values: number[]) {
  if (!shouldUseBatteryCapacityMileageDecimals(values)) return 0;
  return Math.max(1, getAdaptiveDecimalPrecision(values));
}

function shouldUseBatteryCapacityMileageDecimals(values: number[]) {
  const precisionGroups = new Map<number, Set<string>>();
  for (const value of values) {
    if (!Number.isFinite(value)) continue;
    const whole = Math.round(value);
    const group = precisionGroups.get(whole) ?? new Set<string>();
    group.add(value.toFixed(4)); precisionGroups.set(whole, group);
  }
  const rounded = new Set<string>();
  precisionGroups.forEach((group) => group.forEach((value) => rounded.add(value)));
  return rounded.size > precisionGroups.size;
}

function getEfficiencyUnit() {
  return efficiencyProjection().unit;
}

function isDashboardChartPage(value: unknown): value is DashboardChartPage {
  return (
    value === 'overview' ||
    value === 'battery' ||
    value === 'charging' ||
    value === 'efficiency' ||
    value === 'trips'
  );
}

function legacySmoothingToTimeFilter(value: unknown): TimeFilterWindow {
  if (value === false || value === 0) return 'raw';
  return DEFAULT_CHART_TIME_FILTER;
}

export function normalizeChartSettingsMap(value: unknown): Record<string, DashboardChartDisplaySettings> {
  if (value === null || typeof value !== 'object') return {};
  const candidates = Object.entries(value).map(([id, raw]) => ({ id, canonical: normalizeChartId(id), settings: normalizeChartDisplaySettings(raw) }));
  const result: Record<string, DashboardChartDisplaySettings> = {};
  candidates.sort((left, right) => Number(left.id !== left.canonical) - Number(right.id !== right.canonical));
  for (const candidate of candidates) {
    if (!candidate.settings) continue;
    if (candidate.id === candidate.canonical || !(candidate.canonical in result)) result[candidate.canonical] = candidate.settings;
  }
  return result;
}

function normalizeChartDisplaySettings(value: unknown): DashboardChartDisplaySettings | null {
  if (value === null || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  const result: DashboardChartDisplaySettings = {};
  const axes = normalizeChartAxisSettingsMap(raw.axes);
  if (Object.keys(axes).length) result.axes = axes;
  const legacy = !('timeFilter' in raw) && typeof raw.smoothing === 'number';
  if (legacy) result.smoothing = raw.smoothing as number;
  if ('timeFilter' in raw) result.timeFilter = normalizeTimeFilter(raw.timeFilter, DEFAULT_CHART_TIME_FILTER);
  if ('smoothness' in raw) result.smoothness = normalizeCurveSmoothness(raw.smoothness);
  return Object.keys(result).length ? result : null;
}

function normalizeChartAxisSettingsMap(
  value: unknown
): Partial<Record<DashboardChartAxisId, DashboardChartAxisRangeSetting>> {
  if (value === null || typeof value !== 'object') return {};
  const axes: Partial<Record<DashboardChartAxisId, DashboardChartAxisRangeSetting>> = {};
  for (const key of Object.keys(value)) {
    const axis = key as DashboardChartAxisId;
    if (!AXIS_ORDER.includes(axis)) continue;
    const setting = normalizeChartAxisRangeSetting((value as Record<string, unknown>)[key]);
    if (setting != null) axes[axis] = setting;
  }
  return axes;
}

function normalizeChartAxisRangeSetting(value: unknown): DashboardChartAxisRangeSetting | null {
  if (value === null || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  const result: DashboardChartAxisRangeSetting = {};
  for (const bound of ['min', 'max'] as const) {
    if (typeof raw[bound] === 'number' && Number.isFinite(raw[bound])) result[bound] = raw[bound] as number;
  }
  if (raw.mode === 'auto' || raw.mode === 'manual') result.mode = raw.mode;
  return Object.keys(result).length ? result : null;
}

function resolveChartDisplaySettings(
  allSettings: Record<string, DashboardChartDisplaySettings>,
  chartId: string,
  legacyTimeFilter: TimeFilterWindow,
  legacySmoothness: CurveSmoothness
) {
  const saved = allSettings[chartId] ?? {};
  return resolveWidgetOptions({
    timeFilter: saved.smoothing == null ? legacyTimeFilter : legacySmoothingToTimeFilter(saved.smoothing),
    axes: {} as NonNullable<DashboardChartDisplaySettings['axes']>,
    smoothness: normalizeCurveSmoothness(saved.smoothing, legacySmoothness),
  }, saved);
}

function setChartSettingsEntry(
  current: Record<string, DashboardChartDisplaySettings>,
  chartId: string,
  chartSettings: DashboardChartDisplaySettings
) {
  const next = { ...current };
  const entry = normalizeChartDisplaySettings(chartSettings);
  delete next[chartId];
  if (entry) next[chartId] = entry;
  return next;
}

export function getManualAxisRange(setting: DashboardChartAxisRangeSetting | undefined) {
  const bounds = [setting?.min, setting?.max];
  const complete = bounds.every((bound) => typeof bound === 'number' && Number.isFinite(bound));
  return setting?.mode === 'manual' && complete && bounds[0]! < bounds[1]! ? bounds as [number, number] : undefined;
}

function sameResolvedAxisRanges(
  left: DashboardChartResolvedAxisRanges,
  right: DashboardChartResolvedAxisRanges
) {
  return (
    left.y?.[0] === right.y?.[0] &&
    left.y?.[1] === right.y?.[1] &&
    left.y2?.[0] === right.y2?.[0] &&
    left.y2?.[1] === right.y2?.[1]
  );
}

interface ChartSettingsPanelProps {
  open: boolean;
  triggerRef: React.RefObject<HTMLButtonElement | null>;
  chartTitle: string;
  capabilities: DashboardChartSettingsCapabilities;
  settings: DashboardChartDisplaySettings & {
    timeFilter: TimeFilterWindow;
    axes: Partial<Record<DashboardChartAxisId, DashboardChartAxisRangeSetting>>;
  };
  suggestedRanges: DashboardChartResolvedAxisRanges;
  persistent: boolean;
  onClose: () => void;
  onTimeFilterChange: (next: TimeFilterWindow) => void;
  onSmoothnessChange: (next: CurveSmoothness) => void;
  onAxisModeChange: (axisId: DashboardChartAxisId, mode: DashboardChartAxisMode) => void;
  onAxisValueChange: (
    axisId: DashboardChartAxisId,
    bound: 'min' | 'max',
    value: number | undefined
  ) => void;
}

function ChartSettingsPanel({
  open,
  triggerRef,
  chartTitle,
  capabilities,
  settings,
  suggestedRanges,
  persistent,
  onClose,
  onTimeFilterChange,
  onSmoothnessChange,
  onAxisModeChange,
  onAxisValueChange,
}: ChartSettingsPanelProps) {
  const panelRef = React.useRef<HTMLDivElement | null>(null);
  const timeFilterIndex = Math.max(
    0,
    TIME_FILTER_OPTIONS.findIndex((option) => option.value === settings.timeFilter)
  );
  const smoothnessIndex = Math.max(
    0,
    CURVE_SMOOTHNESS_OPTIONS.findIndex(
      (option) => option.value === (settings.smoothness ?? DEFAULT_CURVE_SMOOTHNESS)
    )
  );
  const axisEntries = AXIS_ORDER.flatMap((axisId) =>
    capabilities.axes[axisId] ? [[axisId, capabilities.axes[axisId]] as const] : []
  );
  const hasControls =
    capabilities.timeFilter || capabilities.smoothness === true || axisEntries.length > 0;

  useWidgetDialogDismiss(open, onClose, triggerRef, panelRef);

  if (!open || typeof document === 'undefined') return null;

  return createPortal(
    <>
      <div
        className="fixed inset-0 z-40 flex items-center justify-center bg-bg-page/70 p-3 backdrop-blur-sm"
        onMouseDown={onClose}
      >
        <div
          ref={panelRef}
          role="dialog"
          aria-modal="true"
          aria-label="Chart settings"
          className="max-h-[calc(100dvh-1.5rem)] w-full max-w-xl overflow-y-auto rounded-2xl border border-border bg-bg-surface shadow-xl"
          onMouseDown={(event) => event.stopPropagation()}
        >
          <div className="sticky top-0 z-10 flex items-start justify-between gap-3 border-b border-border bg-bg-surface px-4 py-3">
            <div className="min-w-0">
              <p className="text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
                Chart settings
              </p>
              <h3 className="truncate text-sm font-semibold text-fg">{chartTitle}</h3>
            </div>
            <button
              type="button"
              aria-label="Close chart settings"
              onClick={onClose}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border bg-bg-elevated text-fg-tertiary transition-colors hover:border-border-strong hover:text-fg"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="grid gap-3 p-4">
            {capabilities.timeFilter || capabilities.smoothness ? (
              <details
                open
                className="group overflow-hidden rounded-xl border border-border bg-bg-elevated/50"
              >
                <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-3 text-sm font-medium text-fg marker:hidden">
                  <span>
                    <span className="block text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
                      Display
                    </span>
                    <span>Filter &amp; curve</span>
                  </span>
                  <ChevronDown
                    className="h-4 w-4 shrink-0 text-fg-tertiary transition-transform group-open:rotate-180"
                    aria-hidden="true"
                  />
                </summary>
                <div className="grid gap-3 border-t border-border p-3">
                  {capabilities.timeFilter ? (
                    <section className="grid gap-2 rounded-lg border border-border bg-bg-surface/70 p-3">
                      <div>
                        <p className="text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
                          Display filter
                        </p>
                        <p className="text-sm text-fg">Time window</p>
                      </div>
                      <div>
                        <div className="mb-1.5 flex items-center justify-between text-xs text-fg-tertiary">
                          <span>All compatible curves keep their recorded timestamps.</span>
                          <span>{timeFilterLabel(settings.timeFilter)}</span>
                        </div>
                        <input
                          aria-label="Display filter"
                          type="range"
                          min={0}
                          max={TIME_FILTER_OPTIONS.length - 1}
                          step={1}
                          value={timeFilterIndex}
                          onChange={(event) =>
                            onTimeFilterChange(
                              TIME_FILTER_OPTIONS[Number(event.target.value)]!.value
                            )
                          }
                          className="rm-accent-range w-full"
                        />
                      </div>
                    </section>
                  ) : null}
                  {capabilities.smoothness ? (
                    <section className="grid gap-2 rounded-lg border border-border bg-bg-surface/70 p-3">
                      <div>
                        <p className="text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
                          Curve smoothness
                        </p>
                        <p className="text-sm text-fg">Path between recorded points</p>
                      </div>
                      <div>
                        <div className="mb-1.5 flex items-center justify-between text-xs text-fg-tertiary">
                          <span>Values and timestamps stay unchanged.</span>
                          <span>
                            {curveSmoothnessLabel(settings.smoothness ?? DEFAULT_CURVE_SMOOTHNESS)}
                          </span>
                        </div>
                        <input
                          type="range"
                          min={0}
                          max={CURVE_SMOOTHNESS_OPTIONS.length - 1}
                          step={1}
                          value={smoothnessIndex}
                          onChange={(event) =>
                            onSmoothnessChange(
                              CURVE_SMOOTHNESS_OPTIONS[Number(event.target.value)]!.value
                            )
                          }
                          className="rm-accent-range w-full"
                          aria-label="Curve smoothness"
                        />
                      </div>
                    </section>
                  ) : null}
                </div>
              </details>
            ) : null}
            {axisEntries.length > 0 ? (
              <details
                open
                className="group overflow-hidden rounded-xl border border-border bg-bg-elevated/50"
              >
                <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-3 text-sm font-medium text-fg marker:hidden">
                  <span>
                    <span className="block text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
                      Scale controls
                    </span>
                    <span>Axes</span>
                  </span>
                  <ChevronDown
                    className="h-4 w-4 shrink-0 text-fg-tertiary transition-transform group-open:rotate-180"
                    aria-hidden="true"
                  />
                </summary>
                <div className="grid gap-2 border-t border-border p-3">
                  <p className="text-xs text-fg-tertiary">
                    Auto or manual range for each supported axis.
                  </p>
                  {axisEntries.map(([axisId, capability]) => (
                    <ChartAxisRangeField
                      key={axisId}
                      axisId={axisId}
                      capability={capability}
                      setting={settings.axes[axisId]}
                      {...(axisId !== 'x' && suggestedRanges[axisId]
                        ? { suggestedRange: suggestedRanges[axisId] }
                        : {})}
                      onModeChange={onAxisModeChange}
                      onValueChange={onAxisValueChange}
                    />
                  ))}
                </div>
              </details>
            ) : null}
            {!hasControls ? (
              <div className="rounded-xl border border-border bg-bg-elevated/50 px-3 py-4 text-sm text-fg-secondary">
                This chart does not expose shared display controls yet.
              </div>
            ) : null}
            {!persistent && hasControls ? (
              <p className="text-[11px] text-fg-tertiary">
                Preview only while viewing.
              </p>
            ) : null}
          </div>
        </div>
      </div>
    </>,
    document.body
  );
}

function ChartAxisRangeField({
  axisId,
  capability,
  setting,
  suggestedRange,
  onModeChange,
  onValueChange,
}: {
  axisId: DashboardChartAxisId;
  capability: DashboardChartAxisCapability;
  setting: DashboardChartAxisRangeSetting | undefined;
  suggestedRange?: [number, number];
  onModeChange: (axisId: DashboardChartAxisId, mode: DashboardChartAxisMode) => void;
  onValueChange: (
    axisId: DashboardChartAxisId,
    bound: 'min' | 'max',
    value: number | undefined
  ) => void;
}) {
  const mode = setting?.mode === 'manual' ? 'manual' : 'auto';
  const hasValidRange = Boolean(getManualAxisRange(setting));
  const bounds = (['min', 'max'] as const).map((bound, index) => ({
    bound,
    label: bound === 'min' ? 'Min' : 'Max',
    ariaLabel: `${capability.label} ${bound === 'min' ? 'minimum' : 'maximum'}`,
    value: setting?.[bound] ?? suggestedRange?.[index] ?? '',
  }));

  return (
    <div className="rounded-lg border border-border bg-bg-surface/70 p-2.5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-fg">
          {capability.label}
          {capability.unit ? (
            <span className="ml-1 text-fg-tertiary">({capability.unit})</span>
          ) : null}
        </p>
        <div className="inline-flex rounded-lg border border-border bg-bg-elevated p-0.5">
          {(['auto', 'manual'] as const).map((nextMode) => (
            <button
              key={nextMode}
              type="button"
              onClick={() => onModeChange(axisId, nextMode)}
              className={cn(
                'rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
                mode === nextMode
                  ? 'bg-accent text-fg-on-accent'
                  : 'text-fg-secondary hover:text-fg'
              )}
            >
              {nextMode === 'auto' ? 'Auto' : 'Manual'}
            </button>
          ))}
        </div>
      </div>
      {mode === 'manual' ? (
        <div className="mt-2 grid grid-cols-2 gap-2">
          {bounds.map((field) => (
            <label key={field.bound} className="grid min-w-0 gap-1 text-xs font-medium text-fg-secondary">
              <span>{field.label}</span>
              <input
                aria-label={field.ariaLabel}
                type="number"
                inputMode="decimal"
                step={getAxisInputStep(capability.unit)}
                value={field.value}
                onChange={(event) => onValueChange(axisId, field.bound, parseAxisInputValue(event.target.value))}
                className="h-9 rounded-lg border border-border bg-bg-elevated px-3 text-sm text-fg outline-none focus:border-accent"
              />
            </label>
          ))}
          {!hasValidRange ? (
            <p className="col-span-2 text-[11px] text-fg-tertiary">
              Manual range applies after both values are valid and max is greater than min.
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function getAxisInputStep(unit: string | undefined) {
  return unit === '%' || unit === 'mi' ? 1 : 0.1;
}

function parseAxisInputValue(value: string) {
  if (!value.trim()) return undefined;
  const next = Number(value);
  return Number.isFinite(next) ? next : undefined;
}

const CANONICAL_SOURCES: Record<DashboardChartDefinition['source'], React.ComponentType<ActiveDashboardChartSourceProps>> = {
  soc_history: SocHistorySource,
  charging_sessions_energy: (props) => <ChargingSeriesSource {...props} sessions />,
  charging_weekly_energy: ChargingSeriesSource,
  charge_session_curve: ChargeSessionCurveSource,
  charging_curve_analysis: ChargingCurveAnalysisSource,
  efficiency_trend: EfficiencyTrendSource,
  efficiency_temperature: EfficiencyTemperatureSource,
  efficiency_mode: EfficiencyModeSource,
  efficiency_tags: EfficiencyTagsSource,
  phantom_drain: PhantomDrainSource,
  battery_degradation: BatteryDegradationSource,
  battery_capacity_mileage: BatteryMileageSource,
  projected_range_mileage: ProjectedRangeMileageSource,
  tire_pressure_trips: TirePressureTripsSource,
};

registerWidget({
  componentType: 'chart',
  definitionId: 'catalog',
  title: 'Chart',
  defaultSize: { w: 12, h: 8 },
  minSize: { w: 4, h: 6 },
  defaultOptions: {
    page: 'overview',
    chartId: 'projected-range-mileage',
    showPicker: true,
    timeFilter: DEFAULT_CHART_TIME_FILTER,
  },
  component: DashboardChartWidget,
});

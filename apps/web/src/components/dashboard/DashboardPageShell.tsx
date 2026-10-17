import { resolveShellDashboard } from './dashboardShellModel';
import React, { useEffect, useMemo, useState } from 'react';
import { useAuth, useResolvedVehicleSelection } from '@panda/hooks';
import { DateRangePicker, EfficiencyDisplayToggle, PageLayout, SelectPicker } from '@panda/ui/primitives';
import { getUnitPreferences } from '@panda/ui/lib/utils';
import {
  DashboardRenderer,
  getDefaultBySlug,
  useDashboardById,
  useDashboardBySlug,
} from '@panda/dashboards';
import type { WidgetCtx } from '@panda/dashboards';
import { AppLayout } from '../layout/AppLayout';
import { NoVehicleState } from '../layout/NoVehicleState';
import {
  DEFAULT_TIMEFRAME,
  getTimeframeRange,
  loadDashboardTimeframe,
  saveDashboardTimeframe,
  timeframeToQuery,
  type DashboardTimeframe,
} from '../../lib/dates';
import type { DashboardPageShellRenderState } from './dashboardShellTypes';
export type { DashboardPageShellRenderState } from './dashboardShellTypes';
import { RDashboardSurface } from '../../features/r-experience/RDashboardSurface';

export interface DashboardPageShellProps {
  navKey: string;
  slug: string;
  dashboardId?: string | undefined;
  title?: string | undefined;
  /** Legacy inputs are accepted but cannot activate layout editing. */
  isEditMode?: boolean;
  onEditModeChange?: (isEditMode: boolean) => void;
  renderTitleAction?: (state: DashboardPageShellRenderState) => React.ReactNode;
  /** Optional compact action rendered before the vehicle selector. */
  renderLeadingActions?: (state: DashboardPageShellRenderState) => React.ReactNode;
  renderActions?: (state: DashboardPageShellRenderState) => React.ReactNode;
  renderBeforeDashboard?: (state: DashboardPageShellRenderState) => React.ReactNode;
  renderDashboard?: (state: DashboardPageShellRenderState) => boolean;
  enableDashboardEditing?: boolean | undefined;
  showEfficiencyDisplayToggle?: boolean;
  /** Page-scoped values consumed by a subset of widgets. */
  widgetCtx?: Pick<WidgetCtx, 'tripTagFilter' | 'canManageTripTags'>;
}

export function DashboardPageShell(props: DashboardPageShellProps) {
  return <DashboardPageShellContent {...props} />;
}

function DashboardPageShellContent({
  navKey,
  slug,
  dashboardId,
  title,
  renderTitleAction,
  renderLeadingActions,
  renderActions,
  renderBeforeDashboard,
  renderDashboard,
  showEfficiencyDisplayToggle = false,
  widgetCtx,
}: DashboardPageShellProps) {
  const { setActiveVehicleId } = useAuth();
  const setSessionVehicleId = setActiveVehicleId ?? (() => {});
  const {
    authReady,
    effectiveVehicleId,
    vehicleSelectionReady,
    vehicles: availableVehicles,
  } = useResolvedVehicleSelection();
  const [timeframe, setTimeframe] = useState<DashboardTimeframe>(
    () => loadDashboardTimeframe() ?? DEFAULT_TIMEFRAME,
  );
  const [chargeSessionDayLocal, setChargeSessionDayLocal] = useState<string | null>(null);
  const [unitPreferences, setUnitPreferences] = useState(getUnitPreferences);
  const unitMode = unitPreferences.mode;
  const range = useMemo(() => getTimeframeRange(timeframe), [timeframe]);
  const { from, to } = useMemo(() => timeframeToQuery(timeframe), [timeframe]);

  const hasVehicleChoices = availableVehicles.length > 1;
  const bySlug = useDashboardBySlug(dashboardId ? null : slug);
  const byId = useDashboardById(dashboardId ?? null);
  const selectedResponse = dashboardId ? byId : bySlug;
  const dashboardIsLoading = selectedResponse.isLoading;
  const { saved: savedConfig, unavailable: exactDashboardUnavailable } = resolveShellDashboard({
    exact: Boolean(dashboardId),
    slug,
    response: selectedResponse.data,
    fallback: getDefaultBySlug(slug),
    loading: dashboardIsLoading,
    failed: selectedResponse.isError,
  });

  const activeConfig = savedConfig;
  const setChargeSessionDayFilter = setChargeSessionDayLocal;

  useEffect(() => {
    const subscription = new window.AbortController();
    const refresh = () => setUnitPreferences(getUnitPreferences());
    for (const event of ['rm-units-change', 'storage']) {
      window.addEventListener(event, refresh, { signal: subscription.signal });
    }
    return () => subscription.abort();
  }, []);

  useEffect(() => {
    setChargeSessionDayLocal(null);
  }, [effectiveVehicleId, timeframe]);

  const dashboardConfigId = activeConfig?.id;
  const ctx = useMemo<WidgetCtx>(
    () => ({
      vehicleId: effectiveVehicleId,
      dashboardSlug: slug,
      ...(dashboardConfigId === undefined ? {} : { dashboardConfigId }),
      timeframe,
      from,
      to,
      chargeSessionDayLocal,
      setChargeSessionDayLocal: setChargeSessionDayFilter,
      ...widgetCtx,
    }),
    [
      effectiveVehicleId,
      slug,
      dashboardConfigId,
      timeframe,
      from,
      to,
      chargeSessionDayLocal,
      setChargeSessionDayFilter,
      widgetCtx,
    ],
  );

  useEffect(() => {
    saveDashboardTimeframe(timeframe);
  }, [timeframe]);

  const shellState: DashboardPageShellRenderState = {
    activeConfig,
    savedConfig,
    isEditMode: false,
    isLoading: dashboardIsLoading,
    vehicleId: effectiveVehicleId,
    ctx,
    timeframe,
    range,
    chargeSessionDayLocal,
    setChargeSessionDayLocal: setChargeSessionDayFilter,
    setTimeframe,
  };
  const shouldRenderDashboard = renderDashboard?.(shellState) ?? true;
  const dateRangeAction = activeConfig?.controls?.dateRange ? (
    <DateRangePicker
      timeframe={timeframe}
      onChange={setTimeframe}
      triggerClassName="h-9"
    />
  ) : null;
  const vehicleAction = hasVehicleChoices ? (
    <SelectPicker
      className="min-w-[11rem]"
      value={effectiveVehicleId ?? ''}
      onChange={(vehicleId) => setSessionVehicleId(vehicleId || null)}
      aria-label="Select vehicle"
      options={availableVehicles.map(({ id, display_name, model }) => {
        const label = display_name || model;
        return { value: id, label, description: label === model ? undefined : model };
      })}
    />
  ) : null;
  const efficiencyDisplayAction = showEfficiencyDisplayToggle && unitMode !== 'custom' ? (
    <EfficiencyDisplayToggle />
  ) : null;
  const pageExtraActions = renderActions?.(shellState);
  const pageLeadingActions = renderLeadingActions?.(shellState);
  const titleAction = renderTitleAction?.(shellState);
  const pageActions = pageLeadingActions || vehicleAction || efficiencyDisplayAction || dateRangeAction || pageExtraActions ? (
    <div data-dashboard-actions className="flex flex-wrap items-center justify-end gap-2">
      {pageLeadingActions}
      {vehicleAction}
      {efficiencyDisplayAction}
      {dateRangeAction}
      {pageExtraActions}
    </div>
  ) : undefined;

  return (
    <AppLayout activeKey={navKey}>
      <RDashboardSurface state={shellState} actions={<>{titleAction}{pageActions}</>} ready={authReady && vehicleSelectionReady && Boolean(effectiveVehicleId)}>
      <PageLayout
        title={title ?? activeConfig?.name ?? slug}
        titleAction={titleAction}
        actions={pageActions}
      >
        {exactDashboardUnavailable ? (
          <div role="alert" className="rounded-xl border border-status-danger/30 bg-status-danger/10 p-4 text-sm text-status-danger">
            <p>This dashboard could not be opened. It may no longer exist or you may not have access to it.</p>
            <button
              type="button"
              onClick={() => window.history.back()}
              className="mt-3 rounded-lg border border-status-danger/40 px-3 py-1.5 text-xs font-medium transition-colors hover:bg-status-danger/10"
            >
              Back to Dashboards
            </button>
          </div>
        ) : !authReady || !vehicleSelectionReady ? (
          <div className="text-xs text-fg-tertiary p-4">Loading...</div>
        ) : !effectiveVehicleId ? (
          <NoVehicleState />
        ) : dashboardIsLoading && !activeConfig ? (
          <div className="text-xs text-fg-tertiary p-4">Loading...</div>
        ) : activeConfig ? (
          <>
            {renderBeforeDashboard?.(shellState)}
            {shouldRenderDashboard ? (
              <DashboardRenderer
                config={activeConfig}
                ctx={ctx}
                mode="view"
              />
            ) : null}
          </>
        ) : null}
      </PageLayout>
      </RDashboardSurface>
    </AppLayout>
  );
}

import { persistDashboardDraft } from './dashboardSave';
import { dashboardHeightEdit, resolveShellDashboard } from './dashboardShellModel';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { useAuth, useMe, useResolvedVehicleSelection } from '@panda/hooks';
import { DateRangePicker, EfficiencyDisplayToggle, PageLayout, SelectPicker } from '@panda/ui/primitives';
import { getUnitPreferences } from '@panda/ui/lib/utils';
import {
  DashboardRenderer,
  getDefaultBySlug,
  useCreateDashboard,
  useDashboardById,
  useDashboardBySlug,
  useUpdateAdminDashboard,
  useUpdateDashboard,
} from '@panda/dashboards';
import type { WidgetCtx } from '@panda/dashboards';
import { Edit2, Save, Trash2 } from 'lucide-react';
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
import { useDashboardEditDraft } from './useDashboardEditDraft';
import { useDashboardEditButtonPreference } from './useDashboardEditButtonPreference';
import type { DashboardPageShellRenderState } from './dashboardShellTypes';
export type { DashboardPageShellRenderState } from './dashboardShellTypes';
import { RDashboardSurface } from '../../features/r-experience/RDashboardSurface';

export interface DashboardPageShellProps {
  navKey: string;
  slug: string;
  dashboardId?: string | undefined;
  title?: string | undefined;
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

export interface DashboardEditMutations {
  updateDashboard: ReturnType<typeof useUpdateDashboard>;
  updateAdminDashboard: ReturnType<typeof useUpdateAdminDashboard>;
  createDashboard: ReturnType<typeof useCreateDashboard>;
  qc: ReturnType<typeof useQueryClient>;
  isAdmin: boolean;
}

export function canManageSystemDashboards(role: string | null | undefined) {
  return role === 'admin' || role === 'super_user';
}

export function createDefaultDashboardEditActions({ updateDashboard, updateAdminDashboard, createDashboard, qc, isAdmin }: DashboardEditMutations) {
  return function renderDefaultDashboardEditActions({ isEditMode, isDirty, localConfig, savedConfig, exitEdit, setSaveError }: DashboardPageShellRenderState) {
    if (!isEditMode) return undefined;
    const isPending = updateDashboard.isPending || updateAdminDashboard.isPending || createDashboard.isPending;

    async function handleSave() {
      setSaveError(null);
      if (!localConfig || !savedConfig) { exitEdit(); return; }
      try {
        await persistDashboardDraft(localConfig, savedConfig, { updateDashboard, updateAdminDashboard, createDashboard, qc, isAdmin });
        exitEdit();
      } catch (error) {
        setSaveError(dashboardSaveErrorMessage(error));
      }
    }

    return (
      <>
        <span className="text-[11px] font-medium text-fg-tertiary">{isDirty ? 'Unsaved' : 'Dashboard'}</span>
        <button
          type="button"
          onClick={handleSave}
          disabled={isPending}
          title="Save changes"
          aria-label="Save dashboard changes"
          className="inline-flex h-8 w-8 items-center justify-center rounded-lg bg-accent text-fg-on-accent transition-colors hover:bg-accent/90 disabled:opacity-40"
        >
          <Save className="h-4 w-4" />
        </button>
        <button
          type="button"
          onClick={exitEdit}
          title="Discard changes"
          aria-label="Discard dashboard changes"
          className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border text-fg-tertiary transition-colors hover:bg-bg-elevated hover:text-fg"
        >
          <Trash2 className="h-4 w-4" />
        </button>
      </>
    );
  };
}

function dashboardSaveErrorMessage(error: unknown) {
  if (error instanceof Error && error.message) {
    return `Dashboard save failed: ${error.message}`;
  }
  return 'Dashboard save failed. Your unsaved edits are still open.';
}

export function DashboardPageShell(props: DashboardPageShellProps) {
  return <DashboardPageShellContent {...props} />;
}

function DashboardPageShellContent({
  navKey,
  slug,
  dashboardId,
  title,
  isEditMode: controlledEditMode,
  onEditModeChange,
  renderTitleAction,
  renderLeadingActions,
  renderActions,
  renderBeforeDashboard,
  renderDashboard,
  enableDashboardEditing = true,
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
  const updateDashboard = useUpdateDashboard();
  const updateAdminDashboard = useUpdateAdminDashboard();
  const createDashboard = useCreateDashboard();
  const qc = useQueryClient();
  const me = useMe();
  const [showEditButton] = useDashboardEditButtonPreference(me.data?.user_id);
  const isAdmin = canManageSystemDashboards(me.data?.role);
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
    editing: Boolean(controlledEditMode),
    authReady,
    slug,
    response: selectedResponse.data,
    fallback: getDefaultBySlug(slug),
    loading: dashboardIsLoading,
    failed: selectedResponse.isError,
  });

  const {
    activeConfig,
    localConfig,
    setLocalConfig,
    isEditMode: currentEditMode,
    isDirty,
    saveError,
    setSaveError,
    enterEdit,
    exitEdit,
  } = useDashboardEditDraft({
    savedConfig,
    slug,
    controlledEditMode,
    onEditModeChange,
  });

  const setChargeSessionDayFilter = setChargeSessionDayLocal;
  const updateWidgetLayout = useCallback((widgetId: string, nextHeight: number) => {
    if (Number.isFinite(nextHeight)) setLocalConfig(dashboardHeightEdit(widgetId, nextHeight, savedConfig));
  }, [setLocalConfig, savedConfig]);

  useEffect(() => {
    const handleUnits = () => {
      setUnitPreferences(getUnitPreferences());
    };
    window.addEventListener('rm-units-change', handleUnits as EventListener);
    window.addEventListener('storage', handleUnits);
    return () => {
      window.removeEventListener('rm-units-change', handleUnits as EventListener);
      window.removeEventListener('storage', handleUnits);
    };
  }, []);

  useEffect(() => {
    setChargeSessionDayLocal(null);
  }, [effectiveVehicleId, timeframe]);

  const dashboardConfigId = activeConfig?.id ?? savedConfig?.id;
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
      updateWidgetLayout,
      ...widgetCtx,
    }),
    [
      effectiveVehicleId,
      slug,
      dashboardConfigId,
      timeframe,
      activeConfig?.id,
      savedConfig?.id,
      from,
      to,
      chargeSessionDayLocal,
      setChargeSessionDayFilter,
      updateWidgetLayout,
      widgetCtx,
    ],
  );

  useEffect(() => {
    saveDashboardTimeframe(timeframe);
  }, [timeframe]);

  const shellState: DashboardPageShellRenderState = {
    activeConfig,
    savedConfig,
    localConfig,
    setLocalConfig,
    isEditMode: currentEditMode,
    isDirty,
    isLoading: dashboardIsLoading,
    saveError,
    setSaveError,
    vehicleId: effectiveVehicleId,
    ctx,
    timeframe,
    range,
    chargeSessionDayLocal,
    setChargeSessionDayLocal: setChargeSessionDayFilter,
    setTimeframe,
    enterEdit,
    exitEdit,
  };
  const shouldRenderDashboard = renderDashboard?.(shellState) ?? true;
  const canEditDashboard = enableDashboardEditing
    && shouldRenderDashboard
    && Boolean(activeConfig)
    && Boolean(effectiveVehicleId);

  const dateRangeAction = activeConfig?.controls?.dateRange && !currentEditMode ? (
    <DateRangePicker
      timeframe={timeframe}
      onChange={setTimeframe}
      triggerClassName="h-9"
    />
  ) : null;
  const vehicleAction = hasVehicleChoices && !currentEditMode ? (
    <SelectPicker
      className="min-w-[11rem]"
      value={effectiveVehicleId ?? ''}
      onChange={(vehicleId) => setSessionVehicleId(vehicleId || null)}
      aria-label="Select vehicle"
      options={availableVehicles.map((vehicle) => ({
        value: vehicle.id,
        label: vehicle.display_name || vehicle.model,
        description: vehicle.display_name && vehicle.model !== vehicle.display_name ? vehicle.model : undefined,
      }))}
    />
  ) : null;
  const efficiencyDisplayAction = showEfficiencyDisplayToggle && !currentEditMode && unitMode !== 'custom' ? (
    <EfficiencyDisplayToggle />
  ) : null;
  const dashboardEditActions = currentEditMode && canEditDashboard
    ? createDefaultDashboardEditActions({
      updateDashboard,
      updateAdminDashboard,
      createDashboard,
      qc,
      isAdmin,
    })(shellState)
    : undefined;
  const pageExtraActions = currentEditMode ? undefined : renderActions?.(shellState);
  const pageLeadingActions = currentEditMode ? undefined : renderLeadingActions?.(shellState);
  const defaultTitleAction = !currentEditMode && canEditDashboard && showEditButton ? (
    <button
      type="button"
      onClick={enterEdit}
      className="inline-flex h-8 w-8 items-center justify-center rounded-md text-fg-tertiary/80 transition-colors hover:text-fg hover:bg-bg-elevated"
      title="Edit dashboard"
      aria-label="Edit dashboard"
    >
      <Edit2 className="h-4 w-4" />
    </button>
  ) : null;
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
      <RDashboardSurface state={shellState} actions={<>{renderTitleAction?.(shellState) ?? defaultTitleAction}{pageActions}</>} ready={authReady && vehicleSelectionReady && Boolean(effectiveVehicleId)}>
      <PageLayout
        title={title ?? activeConfig?.name ?? slug}
        titleAction={renderTitleAction?.(shellState) ?? defaultTitleAction}
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
            {currentEditMode && saveError ? (
              <div
                role="alert"
                className="mb-3 rounded-lg border border-status-danger/30 bg-status-danger/10 px-3 py-2 text-xs text-status-danger"
              >
                {saveError}
              </div>
            ) : null}
            {shouldRenderDashboard ? (
              <DashboardRenderer
                config={activeConfig}
                ctx={ctx}
                mode={currentEditMode ? 'edit' : 'view'}
                onConfigChange={setLocalConfig}
                editActions={dashboardEditActions}
              />
            ) : null}
          </>
        ) : null}
      </PageLayout>
      </RDashboardSurface>
    </AppLayout>
  );
}

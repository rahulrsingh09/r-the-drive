import React from 'react';
import { useAuth, useCurrentVehicleStatus } from '@panda/hooks';
import type { DashboardConfig } from './schema';
import type { WidgetCtx } from './registry';
import { DashboardGrid } from './DashboardGrid';
import { DashboardDataProvider, collectDashboardDataRequirements } from './dashboardData';
import { resolveDashboardViewWidgets } from './dashboardModel';
import {
  dashboardVisibilityStateFromStatus,
  hasDashboardVisibilityRules,
} from './dashboardVisibility';

export interface DashboardRendererProps {
  config: DashboardConfig;
  ctx: WidgetCtx;
  /** Legacy editor inputs are accepted but cannot enable layout changes. */
  mode?: 'view' | 'edit';
  onConfigChange?: (next: DashboardConfig) => void;
  editActions?: React.ReactNode;
}

export function DashboardRenderer({ config, ctx }: DashboardRendererProps) {
  const viewCtx = React.useMemo(() => {
    if (!('visibilityState' in ctx)) return ctx;
    const liveContext = { ...ctx };
    delete liveContext.visibilityState;
    return liveContext;
  }, [ctx]);
  const widgets = Array.isArray(config.widgets) ? config.widgets : [];
  const hasVisibilityRules = hasDashboardVisibilityRules(widgets);
  const { defaultVehicleId } = useAuth();
  const vehicleId = ctx.vehicleId ?? defaultVehicleId;
  const { data: status } = useCurrentVehicleStatus(hasVisibilityRules ? vehicleId : null);
  const liveWidgets = React.useMemo(
    () => hasVisibilityRules
      ? resolveDashboardViewWidgets(widgets, dashboardVisibilityStateFromStatus(status))
      : widgets,
    [widgets, hasVisibilityRules, status],
  );
  const requirements = React.useMemo(
    () => collectDashboardDataRequirements(liveWidgets),
    [liveWidgets],
  );

  return (
    <DashboardDataProvider ctx={viewCtx} requirements={requirements}>
      <DashboardGrid widgets={liveWidgets} ctx={viewCtx} />
    </DashboardDataProvider>
  );
}

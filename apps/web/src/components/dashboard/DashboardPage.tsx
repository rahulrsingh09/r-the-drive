import React from 'react';
import { CurrentVehicleStatePanel } from '@panda/dashboards';
import { DashboardPageShell } from './DashboardPageShell';

export { canManageSystemDashboards } from './dashboardPermissions';
export type { DashboardPageShellRenderState } from './DashboardPageShell';
// Compatibility export for route-local imports. The dashboard widget owns the
// production implementation so both surfaces render the same vehicle state.
export { CurrentVehicleStatePanel };

export interface DashboardPageProps {
  /** Sidebar nav key (e.g. "dashboard", "battery"). */
  navKey: string;
  /** Dashboard slug to resolve. */
  slug: string;
  /** Override the page title shown in PageLayout. */
  title?: string | undefined;
  /** Show the page-level efficiency unit toggle next to the date range picker. */
  showEfficiencyDisplayToggle?: boolean | undefined;
}

export function DashboardPage({ navKey, slug, title, showEfficiencyDisplayToggle = false }: DashboardPageProps) {
  return (
    <DashboardPageShell
      navKey={navKey}
      slug={slug}
      title={title}
      showEfficiencyDisplayToggle={showEfficiencyDisplayToggle}
    />
  );
}

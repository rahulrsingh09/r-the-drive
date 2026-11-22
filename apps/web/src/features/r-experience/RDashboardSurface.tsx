import React from 'react';
import { useLocation } from '@tanstack/react-router';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { usesBundledLayout } from './defaultLayout';
import { RBatteryNavigation } from './RBatteryNavigation';

/** "/" renders the Now page directly (routes/index.tsx), so every dashboard shell uses this surface. */
export function RDashboardSurface({ state, children }: {
  state: DashboardPageShellRenderState; actions?: React.ReactNode; ready?: boolean; children: React.ReactNode;
}) {
  const { pathname } = useLocation();
  return (
    <div className="r-dashboard-surface" data-dashboard-slug={state.ctx.dashboardSlug}
      data-r-default-layout={usesBundledLayout(state.activeConfig) || undefined}>
      <RBatteryNavigation pathname={pathname} />
      {children}
    </div>
  );
}

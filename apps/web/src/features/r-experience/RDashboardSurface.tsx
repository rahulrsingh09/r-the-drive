import React from 'react';
import { useLocation } from '@tanstack/react-router';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { ROverview } from './ROverview';
import { usesBundledLayout } from './defaultLayout';
import { RBatteryNavigation } from './RBatteryNavigation';

export function RDashboardSurface({ state, actions, ready, children }: {
  state: DashboardPageShellRenderState; actions: React.ReactNode; ready: boolean; children: React.ReactNode;
}) {
  const { pathname } = useLocation();
  if (ready && pathname === '/') return <ROverview state={state} actions={actions} />;
  return <div className="r-dashboard-surface" data-dashboard-slug={state.ctx.dashboardSlug}
    data-r-default-layout={usesBundledLayout(state.activeConfig) || undefined}>
    <RBatteryNavigation pathname={pathname} />
    {children}
  </div>;
}

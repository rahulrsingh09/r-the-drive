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
  return pathname === '/' && ready && !state.isEditMode
    ? <ROverview state={state} actions={actions} />
    : <div className="r-dashboard-surface" data-dashboard-slug={state.ctx.dashboardSlug}
        data-r-default-layout={(!state.isEditMode && usesBundledLayout(state.activeConfig)) || undefined}>
        <RBatteryNavigation pathname={pathname} />
        {children}
      </div>;
}

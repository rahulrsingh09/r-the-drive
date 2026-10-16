import React from 'react';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { RHomeWorkspace } from './RHomeWorkspace';
import { RRecentActivity } from './RRecentActivity';
import './r-overview.css';

export function ROverview({ state, actions }: { state: DashboardPageShellRenderState; actions: React.ReactNode }) {
  return <RHomeWorkspace state={state} actions={actions} activity={<RRecentActivity ctx={state.ctx} />} />;
}

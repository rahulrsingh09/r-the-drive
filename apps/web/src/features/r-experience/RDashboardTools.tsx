import React from 'react';
import { Link } from '@tanstack/react-router';
import { downloadDashboardYaml } from '@panda/dashboards';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { PanelsTopLeft } from 'lucide-react';

export function RDashboardTools({ state }: { state: DashboardPageShellRenderState }) {
  const config = state.activeConfig;
  const exportActive = config ? () => downloadDashboardYaml(config) : undefined;
  return <details className="r-dashboard-tools">
    <summary aria-label="Dashboard tools" title="Dashboard tools"><PanelsTopLeft aria-hidden="true" /><span>Dashboard tools</span></summary>
    <div>
      <button type="button" onClick={exportActive}>Export dashboard</button>
      <Link to="/settings" search={{ section: 'dashboards' }}>Dashboard library</Link>
    </div>
  </details>;
}

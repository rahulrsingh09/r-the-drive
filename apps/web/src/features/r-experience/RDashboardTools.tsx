import React from 'react';
import { Link, useNavigate } from '@tanstack/react-router';
import { downloadDashboardYaml, useCloneDashboard } from '@panda/dashboards';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { RDashboardImport } from './RDashboardImport';
import { PanelsTopLeft } from 'lucide-react';
import { attemptDashboard, dashboardDestination } from './dashboardModel';

export function RDashboardTools({ state }: { state: DashboardPageShellRenderState }) {
  const clone = useCloneDashboard();
  const navigate = useNavigate();
  const [error, setError] = React.useState<string | null>(null);
  const duplicate = async () => {
    if (!state.activeConfig) return;
    setError(null);
    const result = await attemptDashboard(() =>
      clone.mutateAsync(state.activeConfig!.id).then(config => navigate(dashboardDestination(config))));
    if (!result.ok) {
      const { reason } = result;
      setError(reason instanceof Error ? reason.message : 'Dashboard could not be duplicated.');
    }
  };
  const exportActive = () => {
    const config = state.activeConfig;
    if (config) downloadDashboardYaml(config);
  };
  return <details className="r-dashboard-tools">
    <summary aria-label="Dashboard tools" title="Dashboard tools"><PanelsTopLeft aria-hidden="true" /><span>Dashboard tools</span></summary>
    <div>
      <button type="button" disabled={clone.isPending} onClick={duplicate}>Duplicate dashboard</button>
      <button type="button" onClick={exportActive}>Export dashboard</button>
      <RDashboardImport />
      <Link to="/settings" search={{ section: 'dashboards' }}>Dashboard library</Link>
    </div>
    {error && <p role="alert">{error}</p>}
  </details>;
}

import React from 'react';
import { Link, useNavigate } from '@tanstack/react-router';
import { downloadDashboardYaml, useCloneDashboard } from '@panda/dashboards';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { RDashboardImport } from './RDashboardImport';
import { PanelsTopLeft } from 'lucide-react';

export function RDashboardTools({ state }: { state: DashboardPageShellRenderState }) {
  const clone = useCloneDashboard();
  const navigate = useNavigate();
  const [error, setError] = React.useState<string | null>(null);
  const duplicate = async () => {
    if (!state.activeConfig) return;
    setError(null);
    try {
      const config = await clone.mutateAsync(state.activeConfig.id);
      await navigate({ to: '/d/$slug', params: { slug: config.slug }, search: { edit: 1, dashboardId: config.id } });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Dashboard could not be duplicated.');
    }
  };
  return <details className="r-dashboard-tools">
    <summary aria-label="Dashboard tools" title="Dashboard tools"><PanelsTopLeft aria-hidden="true" /><span>Dashboard tools</span></summary>
    <div>
      <button type="button" disabled={clone.isPending} onClick={() => void duplicate()}>Duplicate dashboard</button>
      <button type="button" onClick={() => { if (state.activeConfig) downloadDashboardYaml(state.activeConfig); }}>Export dashboard</button>
      <RDashboardImport />
      <Link to="/settings" search={{ section: 'dashboards' }}>Dashboard library</Link>
    </div>
    {error && <p role="alert">{error}</p>}
  </details>;
}

import React from 'react';
import { useParams, useSearch } from '@tanstack/react-router';
import { downloadDashboardYaml } from '@panda/dashboards';
import { Download } from 'lucide-react';
import { DashboardPageShell } from './DashboardPageShell';

export function UserDashboardPage() {
  const { slug } = useParams({ from: '/d/$slug' });
  const { dashboardId } = useSearch({ from: '/d/$slug' });
  return (
    <DashboardPageShell
      navKey="dashboard"
      slug={slug}
      dashboardId={dashboardId}
      showEfficiencyDisplayToggle
      renderActions={({ activeConfig }) => activeConfig ? (
        <button
          type="button"
          onClick={() => downloadDashboardYaml(activeConfig)}
          title="Export as YAML"
          className="p-1.5 rounded-lg border border-border hover:bg-bg-elevated transition-colors text-fg-tertiary hover:text-fg"
        >
          <Download className="h-3.5 w-3.5" />
        </button>
      ) : null}
    />
  );
}

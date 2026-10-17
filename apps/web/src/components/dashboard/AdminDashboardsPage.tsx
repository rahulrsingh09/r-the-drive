import React from 'react';
import { useNavigate } from '@tanstack/react-router';
import { PageLayout } from '@panda/ui/primitives';
import { useDashboards, type DashboardConfig } from '@panda/dashboards';
import { AppLayout } from '../layout/AppLayout';
import { ExternalLink } from 'lucide-react';

function DashRow({ d, onAction }: { d: DashboardConfig; onAction: (dashboard: DashboardConfig) => void }) {
    return (
      <div className="flex items-center gap-3 py-3 px-4 border-b border-border last:border-0">
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-fg truncate">{d.name}</p>
          <p className="text-xs text-fg-tertiary">{d.slug} · {d.widgets.length} widgets</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => onAction(d)}
            className="p-1.5 rounded-lg border border-border hover:bg-bg-elevated transition-colors text-fg-tertiary hover:text-fg"
            title="View"
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    );
  }


export function AdminDashboardsPage() {
  const navigate = useNavigate();
  const { data: dashboards, isLoading } = useDashboards();
  const sections: [DashboardConfig[], DashboardConfig[]] = [[], []];
  for (const dashboard of dashboards ?? []) sections[dashboard.isDefault ? 0 : 1].push(dashboard);
  const [defaults, userDashboards] = sections;
  const onAction = (dashboard: DashboardConfig) => navigate({
    to: '/d/$slug',
    params: { slug: dashboard.slug },
    search: { dashboardId: dashboard.id },
  });


  return (
    <AppLayout activeKey="settings">
      <PageLayout title="Admin: Dashboards">
        {isLoading ? (
          <div className="text-xs text-fg-tertiary p-4">Loading…</div>
        ) : (
          <div className="space-y-6">
            <section>
              <h2 className="text-xs font-medium text-fg-tertiary uppercase tracking-wider mb-2">
                System Defaults
              </h2>
              <div className="rounded-xl border border-border bg-bg divide-y divide-border overflow-hidden">
                {defaults.length === 0 ? (
                  <p className="text-xs text-fg-tertiary p-4">No system defaults found.</p>
                ) : (
                  defaults.map((d) => <DashRow key={d.id} d={d} onAction={onAction} />)
                )}
              </div>
            </section>

            <section>
              <h2 className="text-xs font-medium text-fg-tertiary uppercase tracking-wider mb-2">
                User Dashboards
              </h2>
              <div className="rounded-xl border border-border bg-bg overflow-hidden">
                {userDashboards.length === 0 ? (
                  <p className="text-xs text-fg-tertiary p-4">No user dashboards yet.</p>
                ) : (
                  userDashboards.map((d) => <DashRow key={d.id} d={d} onAction={onAction} />)
                )}
              </div>
            </section>
          </div>
        )}
      </PageLayout>
    </AppLayout>
  );
}

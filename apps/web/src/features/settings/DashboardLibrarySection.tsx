import type { DashboardConfig } from '@panda/dashboards';
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from '@panda/ui/primitives';
import { Download, ExternalLink } from 'lucide-react';

export function DashboardLibrarySection({
  dashboards, isLoading, onOpen, onExport,
}: {
  dashboards: DashboardConfig[];
  isLoading: boolean;
  onOpen: (dashboard: DashboardConfig) => void;
  onExport: (dashboard: DashboardConfig) => void;
}) {
  const sections = [
    { title: 'System Defaults', rows: [] as DashboardConfig[] },
    { title: 'My Dashboards', rows: [] as DashboardConfig[] },
  ] as const;
  const personalSlugs = new Set<string>();
  for (const dashboard of dashboards) {
    sections[dashboard.isDefault ? 0 : 1].rows.push(dashboard);
    if (!dashboard.isDefault) personalSlugs.add(dashboard.slug);
  }
  return (
    <div className="flex flex-col gap-5">
      <Card>
        <CardHeader><CardTitle>Dashboards</CardTitle><Badge variant="info">{dashboards.length} saved</Badge></CardHeader>
        <CardContent className="grid gap-4">
          <details className="group rounded-xl border border-border bg-bg-elevated/35">
            <summary className="cursor-pointer list-none px-3 py-3 text-sm font-medium text-fg marker:hidden">How dashboard defaults and personal copies work</summary>
            <div className="grid gap-2 border-t border-border px-3 py-3 text-sm text-fg-secondary">
              <p><strong className="text-fg">System defaults</strong> are shared within this R:The Drive installation.</p>
              <p><strong className="text-fg">My dashboards</strong> are private to your account. A saved personal copy with the same page name is active for you and takes precedence without changing the system default.</p>
              <p>Saved layouts remain available to open and export.</p>
            </div>
          </details>
          {isLoading ? <div className="rounded-xl border border-border bg-bg-elevated/35 p-4 text-sm text-fg-tertiary">Loading dashboards...</div> : sections.map(({ title, rows }) => (
            <section key={title} className="grid gap-2">
              <div className="flex items-center justify-between">
                <h3 className="text-xs font-medium uppercase tracking-wider text-fg-tertiary">{title}</h3>
                <span className="text-xs text-fg-tertiary">{rows.length}</span>
              </div>
              <div className="overflow-hidden rounded-xl border border-border">
                {rows.length === 0 ? <div className="p-4 text-sm text-fg-tertiary">No dashboards here yet.</div> : rows.map((dashboard) => (
                  <div key={dashboard.id} className="grid gap-3 border-b border-border bg-bg last:border-0 p-3 md:grid-cols-[minmax(0,1fr)_auto]">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="truncate text-sm font-medium text-fg">{dashboard.name}</p>
                        <Badge variant={dashboard.isDefault ? 'info' : 'default'} size="sm">{dashboard.isDefault ? 'Default' : 'User'}</Badge>
                        {dashboard.isLocked ? <Badge variant="warning" size="sm">Locked</Badge> : null}
                        {dashboard.ownerId != null || !personalSlugs.has(dashboard.slug) ? <Badge variant="success" size="sm">Active for you</Badge> : null}
                      </div>
                      <p className="mt-1 text-xs text-fg-tertiary">{dashboard.slug} &middot; {dashboard.widgets.length} widgets</p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2 [&>button]:min-h-11 sm:[&>button]:min-h-8">
                      <Button variant="secondary" size="sm" iconLeft={<ExternalLink className="h-3.5 w-3.5" />} onClick={() => onOpen(dashboard)}>{dashboard.isDefault ? 'Open default' : 'Open'}</Button>
                      <Button variant="secondary" size="sm" iconLeft={<Download className="h-3.5 w-3.5" />} onClick={() => onExport(dashboard)}>Export</Button>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

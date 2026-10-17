import type { DashboardConfig } from '@panda/dashboards';

export function resolveShellDashboard({
  exact, editing, authReady, slug, response, fallback, loading, failed,
}: {
  exact: boolean; editing: boolean; authReady: boolean; slug: string;
  response: DashboardConfig | undefined; fallback: DashboardConfig | undefined;
  loading: boolean; failed: boolean;
}) {
  const matching = response?.slug === slug ? response : undefined;
  const mayUseDefault = !exact && !(editing && (!authReady || loading || failed));
  return {
    saved: matching ?? (mayUseDefault ? fallback : undefined),
    unavailable: exact && !loading && (failed || matching === undefined),
  };
}

export function dashboardHeightEdit(widgetId: string, height: number, saved: DashboardConfig | undefined) {
  const requested = Number.isFinite(height) ? Math.max(1, Math.min(20, Math.round(height))) : null;
  return (draft: DashboardConfig | null): DashboardConfig | null => {
    const source = draft ?? saved;
    const index = source?.widgets.findIndex(widget => widget.id === widgetId) ?? -1;
    const widget = source?.widgets[index];
    if (requested === null || !source || !widget || widget.layout.h === requested) return draft;
    const resized = { ...widget, layout: { ...widget.layout, h: requested } };
    return { ...source, widgets: [...source.widgets.slice(0, index), resized, ...source.widgets.slice(index + 1)] };
  };
}

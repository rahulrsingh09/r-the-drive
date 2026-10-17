import type { DashboardConfig } from '@panda/dashboards';

type Widget = DashboardConfig['widgets'][number];

export function homeSections(config: DashboardConfig) {
  let vehicle: Widget | undefined;
  for (const widget of config.widgets) {
    if (widget.componentType === 'custom' && widget.definitionId === 'overview.vehicle') {
      vehicle = widget;
      break;
    }
  }
  const totals: Widget[] = [];
  const charts: Widget[] = [];
  for (const widget of config.widgets) {
    if (widget.id === vehicle?.id) continue;
    (widget.componentType === 'sensor' ? totals : charts).push(widget);
  }
  return { vehicle, totals: rebaseSection(config, totals), charts: rebaseSection(config, charts) };
}

function rebaseSection(config: DashboardConfig, widgets: Widget[]): DashboardConfig {
  const origin = widgets.reduce((lowest, widget) => Math.min(lowest, widget.layout.y), Infinity);
  const positions = widgets.map(widget => ({
    ...widget, layout: { ...widget.layout, y: widget.layout.y - origin },
  }));
  return { ...config, widgets: positions };
}

export function dashboardDestination(config: Pick<DashboardConfig, 'id' | 'slug'>) {
  return { to: '/d/$slug' as const, params: { slug: config.slug }, search: { edit: 1 as const, dashboardId: config.id } };
}

export type DashboardOutcome<T> = { ok: true; value: T } | { ok: false; reason: unknown };

export function attemptDashboard<T>(operation: () => T | Promise<T>): Promise<DashboardOutcome<T>> {
  return Promise.resolve().then(operation).then(
    value => ({ ok: true as const, value }),
    (reason: unknown) => ({ ok: false as const, reason }),
  );
}

export type ImportState = { draft: DashboardConfig | null; error: string | null };
export type ImportEvent =
  | { type: 'begin' }
  | { type: 'dismiss' }
  | { type: 'loaded'; draft: DashboardConfig }
  | { type: 'failed'; message: string };

export function importState(state: ImportState, event: ImportEvent): ImportState {
  switch (event.type) {
    case 'begin': return { ...state, error: null };
    case 'dismiss': return { ...state, draft: null };
    case 'failed': return { ...state, error: event.message };
    case 'loaded': {
      const { draft } = event;
      return { ...state, draft: { ...draft, slug: `${draft.slug}-import-${draft.id.slice(0, 8)}` } };
    }
  }
}

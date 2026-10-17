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

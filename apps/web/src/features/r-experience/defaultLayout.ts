import { getDefaultBySlug, type DashboardConfig } from '@panda/dashboards';

export function usesBundledLayout(config: DashboardConfig | undefined): boolean {
  const bundled = config?.isDefault ? getDefaultBySlug(config.slug) : undefined;
  if (!config || !bundled || bundled.widgets.length !== config.widgets.length) return false;
  const signatures = new Map<string, ReturnType<typeof layoutSignature>>();
  for (const widget of bundled.widgets) {
    if (!signatures.has(widget.id)) signatures.set(widget.id, layoutSignature(widget));
  }
  for (const widget of config.widgets) {
    const expected = signatures.get(widget.id);
    if (!expected) return false;
    const actual = layoutSignature(widget);
    if (actual.some((part, index) => part !== expected[index])) return false;
  }
  return true;
}

function layoutSignature(widget: DashboardConfig['widgets'][number]) {
  const { componentType, definitionId, layout: { x, y, w, h } } = widget;
  return [componentType, definitionId, x, y, w, h] as const;
}

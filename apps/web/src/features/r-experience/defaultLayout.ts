import { getDefaultBySlug, type DashboardConfig } from '@panda/dashboards';

export function usesBundledLayout(config: DashboardConfig | undefined): boolean {
  if (!config?.isDefault) return false;
  const bundled = getDefaultBySlug(config.slug);
  if (!bundled || config.widgets.length !== bundled.widgets.length) return false;
  return config.widgets.every(widget => {
    const original = bundled.widgets.find(item => item.id === widget.id);
    return original && original.componentType === widget.componentType
      && original.definitionId === widget.definitionId
      && (['x', 'y', 'w', 'h'] as const).every(key => original.layout[key] === widget.layout[key]);
  });
}

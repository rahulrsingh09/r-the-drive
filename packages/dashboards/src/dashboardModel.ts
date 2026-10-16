import { v4 as uuidv4 } from 'uuid';
import type { DashboardConfig, WidgetInstance, WidgetLayout } from './schema';
import type { WidgetDef } from './registry';
import { sanitizeDashboardConfig, sanitizeWidgetInstance, sanitizeWidgetLayout } from './layout';
import type { DashboardVisibilityState } from './dashboardVisibility';
import { isWidgetVisible } from './dashboardVisibility';

export const DASHBOARD_GRID_COLUMNS = 12;
export const DASHBOARD_ROW_HEIGHT = 40;
export type DashboardOwnership = 'system-default' | 'user-owned';

export interface DashboardLayoutPatch {
  i: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export function dashboardKey(config: DashboardConfig | null | undefined, fallbackSlug: string) {
  return config ? `${config.id}:${config.slug}` : `pending:${fallbackSlug}`;
}

export function widgetKeyForInstance(widget: WidgetInstance) {
  return `${widget.id}:${widget.componentType}:${widget.definitionId}`;
}

export function getDashboardOwnership(config: DashboardConfig): DashboardOwnership {
  return config.ownerId ? 'user-owned' : 'system-default';
}

export function isSystemDefaultDashboard(config: DashboardConfig) {
  return config.isDefault && getDashboardOwnership(config) === 'system-default';
}

export function isUserOwnedDashboard(config: DashboardConfig) {
  return getDashboardOwnership(config) === 'user-owned';
}

export function findOwnedDashboardBySlug(
  dashboards: DashboardConfig[] | undefined,
  slug: string,
): DashboardConfig | undefined {
  return dashboards?.find((dashboard) => dashboard.slug === slug && dashboard.ownerId != null);
}

export function materializeUserDashboardDraft(
  draft: DashboardConfig,
  ownedCopy?: DashboardConfig | null,
): DashboardConfig {
  return sanitizeDashboardConfig({
    ...draft,
    id: ownedCopy?.id ?? (draft.ownerId ? draft.id : uuidv4()),
    ownerId: ownedCopy?.ownerId ?? draft.ownerId ?? null,
    isDefault: false,
    isLocked: false,
  });
}

export function materializeSystemDashboardDraft(
  draft: DashboardConfig,
  systemDefault: DashboardConfig,
): DashboardConfig {
  return sanitizeDashboardConfig({
    ...draft,
    id: systemDefault.id,
    ownerId: systemDefault.ownerId,
    isDefault: true,
    isLocked: systemDefault.isLocked,
  });
}

export function createWidgetInstance(
  def: WidgetDef,
  layout: Pick<WidgetLayout, 'x' | 'y'> & Partial<Pick<WidgetLayout, 'w' | 'h'>>,
): WidgetInstance {
  return sanitizeWidgetInstance({
    id: uuidv4(),
    componentType: def.componentType,
    definitionId: def.definitionId,
    title: def.title,
    layout: {
      x: layout.x,
      y: layout.y,
      w: layout.w ?? def.defaultSize.w,
      h: layout.h ?? def.defaultSize.h,
    },
    options: def.defaultOptions,
  });
}

export function applyWidgetLayout(
  widgets: WidgetInstance[],
  layout: readonly DashboardLayoutPatch[],
): WidgetInstance[] {
  // Grid callbacks may omit hidden widgets and may contain repeated IDs.
  // Only the last patch for an ID applies; the saved widget order never changes.
  const patches = new Map(layout.map(patch => [patch.i, patch]));
  let result = widgets;
  for (const [index, widget] of widgets.entries()) {
    const patch = patches.get(widget.id);
    if (!patch) continue;
    const updated = sanitizeWidgetInstance({
      ...widget,
      layout: sanitizeWidgetLayout({ x: patch.x, y: patch.y, w: patch.w, h: patch.h }),
    });
    if (layoutsEqual(updated.layout, widget.layout)) continue;
    if (result === widgets) result = widgets.slice();
    result[index] = updated;
  }
  return result;
}

const UNPLUGGED_MIX_COLUMNS: Readonly<Record<string, number>> = {
  'charging:home_share': 0,
  'sensor:charging_home_share': 0,
  'charging:dc_share': 6,
  'sensor:charging_dc_share': 6,
};

export function resolveDashboardViewWidgets(
  widgets: readonly WidgetInstance[],
  state: DashboardVisibilityState,
): WidgetInstance[] {
  const result: WidgetInstance[] = [];
  const unplugged = state['vehicle-connection'] === 'unplugged';
  for (const widget of widgets) {
    if (!isWidgetVisible(widget, state)) continue;
    const column = unplugged
      ? UNPLUGGED_MIX_COLUMNS[`${widget.componentType}:${widget.definitionId}`]
      : undefined;
    if (column === undefined || (widget.layout.x === column && widget.layout.w === 6)) {
      result.push(widget);
    } else {
      result.push({ ...widget, layout: { ...widget.layout, x: column, w: 6 } });
    }
  }
  return result;
}

function layoutsEqual(a: WidgetLayout, b: WidgetLayout) {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

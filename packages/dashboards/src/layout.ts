import { DashboardConfigSchema } from './schema';
import type { DashboardConfig, WidgetInstance, WidgetLayout } from './schema';
import { getWidgetEditorMeta, getWidgetForInstance } from './registry';

const COLS = 12;
type Size = Pick<WidgetLayout, 'w' | 'h'>;
type LayoutOptions = { minSize?: Size; maxSize?: Size; fixedSize?: boolean };
type Bounds = { min: number; max: number };

export function sanitizeDashboardConfig(input: DashboardConfig): DashboardConfig {
  // Saved configurations must pass the schema before registry constraints apply.
  const parsed = DashboardConfigSchema.parse(input);
  return { ...parsed, widgets: parsed.widgets.map(sanitizeWidgetInstance) };
}

export function sanitizeWidgetInstance(widget: WidgetInstance): WidgetInstance {
  const definition = getWidgetForInstance(widget);
  const editor = getWidgetEditorMeta(definition);
  const defaultSize = definition?.defaultSize ?? { w: 3, h: 2 };
  const layout = editor.fixedSize
    ? { ...widget.layout, w: defaultSize.w, h: defaultSize.h }
    : widget.layout;
  const options: LayoutOptions = {
    minSize: definition?.minSize ?? { w: 1, h: 1 },
    fixedSize: editor.fixedSize,
  };
  if (editor.maxSize) options.maxSize = editor.maxSize;
  return { ...widget, layout: sanitizeWidgetLayout(layout, options) };
}

export function sanitizeWidgetLayout(
  layout: WidgetLayout,
  options: LayoutOptions = {},
): WidgetLayout {
  const size = {
    w: project(layout.w, dimensionBounds('w', layout, options)),
    h: project(layout.h, dimensionBounds('h', layout, options)),
  };
  const position = {
    x: project(layout.x, { min: 0, max: Math.max(0, COLS - size.w) }),
    y: Math.max(0, Math.round(Number.isFinite(layout.y) ? layout.y : 0)),
  };
  return { ...position, ...size };
}

function dimensionBounds(axis: keyof Size, layout: WidgetLayout, options: LayoutOptions): Bounds {
  const ceiling = axis === 'w' ? COLS : Number.MAX_SAFE_INTEGER;
  const min = options.fixedSize ? layout[axis] : options.minSize?.[axis] ?? 1;
  const max = options.fixedSize ? layout[axis] : options.maxSize?.[axis] ?? ceiling;
  return { min, max: axis === 'w' ? Math.min(max, COLS) : max };
}

function project(value: number, { min, max }: Bounds): number {
  const integer = Math.round(Number.isFinite(value) ? value : min);
  return Math.min(max, Math.max(min, integer));
}

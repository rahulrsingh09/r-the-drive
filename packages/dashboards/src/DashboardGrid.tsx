import React from 'react';
import { DASHBOARD_GRID_COLUMNS, DASHBOARD_ROW_HEIGHT } from './dashboardModel';
import { WidgetChrome } from './WidgetChrome';
import type { WidgetCtx } from './registry';
import type { WidgetInstance, WidgetLayout } from './schema';

/** Selector for a sensor tile that has no value and hides in dashboard view. */
const HIDDEN_SENSOR_SELECTOR = '[data-sensor-empty="hidden"]';
/** Selector for the compact row that a chart with no data shows (56–64 px). */
const EMPTY_CHART_SELECTOR = '[data-chart-empty-row]';
/** Matches the `gap-4` of the view grid. */
const DASHBOARD_GRID_GAP = 16;
/** Desktop rows for an empty chart row: 2 rows (96 px) hold 64 px. */
const EMPTY_CHART_MIN_ROWS = 2;

function useIsMobile() {
  const [viewport] = React.useState(() => {
    let mobile = typeof window !== 'undefined' && window.innerWidth < 640;
    return {
      read: () => mobile,
      subscribe(notify: () => void) {
        const query = window.matchMedia('(max-width: 639px)');
        const update = ({ matches }: { matches: boolean }) => {
          mobile = matches;
          notify();
        };
        query.addEventListener('change', update);
        update(query);
        return () => query.removeEventListener('change', update);
      },
    };
  });
  return React.useSyncExternalStore(viewport.subscribe, viewport.read, viewport.read);
}

export interface DashboardGridProps {
  widgets: readonly WidgetInstance[];
  ctx: WidgetCtx;
}

const DEFERRED_CUSTOM_WIDGETS = new Set([
  'trips.map',
  'trips.table',
  'charging.sessions.table',
]);

function shouldDeferWidget(widget: WidgetInstance) {
  return widget.componentType === 'chart' ||
    (widget.componentType === 'custom' && DEFERRED_CUSTOM_WIDGETS.has(widget.definitionId));
}

const pendingWidgets = new Map<Element, () => void>();
let widgetObserver: IntersectionObserver | undefined;

function releaseWidget(element: Element) {
  pendingWidgets.delete(element);
  widgetObserver?.unobserve(element);
  if (!pendingWidgets.size) {
    widgetObserver?.disconnect();
    widgetObserver = undefined;
  }
}

function observeWidget(element: Element, reveal: () => void) {
  if (!widgetObserver) widgetObserver = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const notify = pendingWidgets.get(entry.target);
      releaseWidget(entry.target);
      notify?.();
    }
  }, { rootMargin: '320px 0px' });
  pendingWidgets.set(element, reveal);
  widgetObserver.observe(element);
  return () => releaseWidget(element);
}

function DeferredWidget({
  enabled,
  children,
}: {
  enabled: boolean;
  children: React.ReactNode;
}) {
  const [visible, reveal] = React.useReducer(() => true, !enabled);
  const boundaryRef = React.useRef<HTMLDivElement | null>(null);

  React.useEffect(() => {
    if (visible || !enabled) return;
    const element = boundaryRef.current;
    if (!element || typeof IntersectionObserver === 'undefined') {
      reveal();
      return;
    }
    return observeWidget(element, reveal);
  }, [visible, enabled]);

  return (
    <div ref={boundaryRef} className="h-full min-h-[inherit]">
      {visible ? children : (
        <div className="h-full min-h-[10rem] animate-pulse rounded-xl border border-border bg-bg-surface" aria-label="Widget loading" />
      )}
    </div>
  );
}

/** Desktop grid rows that hold `height` px of content. */
export function rowsForHeight(height: number) {
  return Math.max(1, Math.ceil((height + DASHBOARD_GRID_GAP) / (DASHBOARD_ROW_HEIGHT + DASHBOARD_GRID_GAP)));
}

/**
 * Builds the view layout without gaps.
 *
 * 1. A hidden widget leaves the layout. A tile row is a set of widgets with
 *    the same `y` and `h` that sit side by side with no space between them.
 *    When a tile row loses a widget, the widgets that stay share the full
 *    width of the row. The share keeps the ratio of their saved widths.
 * 2. A widget in `heights` (an empty chart row) gets that smaller height.
 * 3. Widgets then move up until they touch the widget above or row 0. This is
 *    the same vertical compaction that the grid editor applies.
 *
 * With no hidden widget and no height change, the function returns the saved
 * layout unchanged.
 */
export function collapseHiddenWidgets(
  widgets: readonly WidgetInstance[],
  hiddenIds: ReadonlySet<string>,
  heights: ReadonlyMap<string, number> = new Map(),
): Map<string, WidgetLayout> {
  const layouts = new Map<string, WidgetLayout>();
  const shrinks = (widget: WidgetInstance) => {
    const h = heights.get(widget.id);
    return h !== undefined && h < widget.layout.h;
  };
  if (!widgets.some((widget) => hiddenIds.has(widget.id) || shrinks(widget))) {
    for (const widget of widgets) layouts.set(widget.id, widget.layout);
    return layouts;
  }

  const rows = new Map<string, WidgetInstance[]>();
  for (const widget of widgets) {
    const key = `${widget.layout.y}:${widget.layout.h}`;
    rows.set(key, [...(rows.get(key) ?? []), widget]);
  }

  for (const row of rows.values()) {
    const sorted = [...row].sort((a, b) => a.layout.x - b.layout.x);
    const visible = sorted.filter((widget) => !hiddenIds.has(widget.id));
    for (const widget of visible) layouts.set(widget.id, { ...widget.layout });
    const contiguous = sorted.every((widget, index) => {
      const previous = sorted[index - 1];
      return !previous || previous.layout.x + previous.layout.w === widget.layout.x;
    });
    if (!contiguous || visible.length === 0 || visible.length === sorted.length) continue;

    const start = sorted[0]!.layout.x;
    const span = sorted.reduce((total, widget) => total + widget.layout.w, 0);
    const visibleSpan = visible.reduce((total, widget) => total + widget.layout.w, 0);
    let x = start;
    visible.forEach((widget, index) => {
      const remaining = start + span - x;
      const slotsAfter = visible.length - index - 1;
      const w = index === visible.length - 1
        ? remaining
        : Math.max(1, Math.min(remaining - slotsAfter, Math.round((widget.layout.w * span) / visibleSpan)));
      layouts.set(widget.id, { ...widget.layout, x, w });
      x += w;
    });
  }

  for (const widget of widgets) {
    if (!hiddenIds.has(widget.id) && shrinks(widget)) layouts.get(widget.id)!.h = heights.get(widget.id)!;
  }

  const placed: WidgetLayout[] = [];
  const collides = (a: WidgetLayout, b: WidgetLayout) =>
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
  const ordered = widgets
    .filter((widget) => !hiddenIds.has(widget.id))
    .sort((a, b) => a.layout.y - b.layout.y || a.layout.x - b.layout.x);
  for (const widget of ordered) {
    const layout = layouts.get(widget.id)!;
    while (layout.y > 0 && !placed.some((other) => collides({ ...layout, y: layout.y - 1 }, other))) {
      layout.y -= 1;
    }
    while (placed.some((other) => collides(layout, other))) layout.y += 1;
    placed.push(layout);
  }
  return layouts;
}

/**
 * Calls `onChange` with the first element in the cell that matches
 * `selector`, or null. A widget decides this after its data loads, so the
 * cell watches its own subtree.
 */
function useSubtreeMatch(
  ref: React.RefObject<HTMLDivElement | null>,
  selector: string,
  onChange: (match: Element | null) => void,
) {
  const onChangeRef = React.useRef(onChange);
  onChangeRef.current = onChange;

  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    let last: Element | null | undefined;
    const check = () => {
      const match = element.querySelector(selector);
      if (match !== last) {
        last = match;
        onChangeRef.current(match);
      }
    };
    check();
    if (typeof MutationObserver === 'undefined') return;
    const observer = new MutationObserver(check);
    observer.observe(element, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['data-sensor-empty', 'data-chart-empty-row'],
    });
    return () => {
      observer.disconnect();
      onChangeRef.current(null);
    };
  }, [ref, selector]);
}

function DashboardGridCell({
  widget,
  ctx,
  isMobile,
  layout,
  hidden,
  emptyChart,
  onHiddenChange,
  onEmptyChartChange,
}: {
  widget: WidgetInstance;
  ctx: WidgetCtx;
  isMobile: boolean;
  layout: WidgetLayout;
  hidden: boolean;
  emptyChart: boolean;
  onHiddenChange: (id: string, hidden: boolean) => void;
  onEmptyChartChange: (id: string, rows: number | null) => void;
}) {
  const ref = React.useRef<HTMLDivElement | null>(null);
  const isChart = widget.componentType === 'chart';
  const reportMatch = React.useCallback((match: Element | null) => {
    if (isChart) {
      const height = match ? match.getBoundingClientRect().height : 0;
      onEmptyChartChange(widget.id, match ? Math.max(EMPTY_CHART_MIN_ROWS, rowsForHeight(height)) : null);
    } else {
      onHiddenChange(widget.id, match !== null);
    }
  }, [isChart, onEmptyChartChange, onHiddenChange, widget.id]);
  useSubtreeMatch(ref, isChart ? EMPTY_CHART_SELECTOR : HIDDEN_SENSOR_SELECTOR, reportMatch);

  const mobileHeight = Math.min(widget.layout.h * DASHBOARD_ROW_HEIGHT, 480);
  // An empty chart row takes its own height (auto) instead of the chart height.
  const mobileStyle: React.CSSProperties = isChart
    ? emptyChart ? {} : { height: `${mobileHeight}px` }
    : { minHeight: `${mobileHeight}px` };

  return (
    <div
      ref={ref}
      data-widget-id={widget.id}
      data-widget-type={widget.componentType}
      data-widget-definition={widget.definitionId}
      {...(hidden ? { 'data-widget-collapsed': 'true' } : {})}
      {...(emptyChart ? { 'data-widget-empty-chart': 'true' } : {})}
      hidden={hidden}
      // The CSS rule hides the cell before the layout state updates.
      className={isChart ? undefined : 'has-[[data-sensor-empty=hidden]]:hidden'}
      style={isMobile ? mobileStyle : {
        gridColumn: `${layout.x + 1} / span ${layout.w}`,
        gridRow: `${layout.y + 1} / span ${layout.h}`,
      }}
    >
      <DeferredWidget enabled={shouldDeferWidget(widget)}>
        <WidgetChrome instance={widget} ctx={ctx} mode="view" />
      </DeferredWidget>
    </div>
  );
}

/**
 * View-mode dashboard grid.
 * - A sensor tile with no value renders `[data-sensor-empty="hidden"]`. Its
 *   cell collapses and the other widgets close the gap.
 * - A chart with no data renders `[data-chart-empty-row]` (56–64 px). Its
 *   cell gets auto height on mobile and 2 rows on desktop.
 * See `collapseHiddenWidgets`. The editor (`GridEditor`) keeps the saved
 * cell size.
 */
export function DashboardGrid({ widgets, ctx }: DashboardGridProps) {
  const isMobile = useIsMobile();
  const [hiddenIds, setHiddenIds] = React.useState<ReadonlySet<string>>(() => new Set());
  const handleHiddenChange = React.useCallback((id: string, hidden: boolean) => {
    setHiddenIds((current) => {
      if (current.has(id) === hidden) return current;
      const next = new Set(current);
      if (hidden) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);
  const [emptyChartRows, setEmptyChartRows] = React.useState<ReadonlyMap<string, number>>(() => new Map());
  const handleEmptyChartChange = React.useCallback((id: string, rows: number | null) => {
    setEmptyChartRows((current) => {
      if ((current.get(id) ?? null) === rows) return current;
      const next = new Map(current);
      if (rows === null) next.delete(id);
      else next.set(id, rows);
      return next;
    });
  }, []);
  const layouts = React.useMemo(
    () => collapseHiddenWidgets(widgets, hiddenIds, emptyChartRows),
    [widgets, hiddenIds, emptyChartRows],
  );

  return (
    <div
      className="grid gap-4"
      data-dashboard-grid="view"
      style={isMobile ? {
        gridTemplateColumns: 'minmax(0, 1fr)',
        gridAutoRows: 'auto',
      } : {
        gridTemplateColumns: `repeat(${DASHBOARD_GRID_COLUMNS}, minmax(0, 1fr))`,
        gridAutoRows: `${DASHBOARD_ROW_HEIGHT}px`,
      }}
    >
      {widgets.map((widget) => (
        <DashboardGridCell
          key={widget.id}
          widget={widget}
          ctx={ctx}
          isMobile={isMobile}
          layout={layouts.get(widget.id) ?? widget.layout}
          hidden={hiddenIds.has(widget.id)}
          emptyChart={emptyChartRows.has(widget.id)}
          onHiddenChange={handleHiddenChange}
          onEmptyChartChange={handleEmptyChartChange}
        />
      ))}
    </div>
  );
}

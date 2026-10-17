import React from 'react';
import { DASHBOARD_GRID_COLUMNS, DASHBOARD_ROW_HEIGHT } from './dashboardModel';
import { WidgetChrome } from './WidgetChrome';
import type { WidgetCtx } from './registry';
import type { WidgetInstance } from './schema';

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

function widgetGridStyle(widget: WidgetInstance, mobile: boolean): React.CSSProperties {
  if (!mobile) return {
    gridColumn: `${widget.layout.x + 1} / span ${widget.layout.w}`,
    gridRow: `${widget.layout.y + 1} / span ${widget.layout.h}`,
  };
  const height = `${Math.min(widget.layout.h * DASHBOARD_ROW_HEIGHT, 480)}px`;
  return widget.componentType === 'chart' ? { height } : { minHeight: height };
}

export function DashboardGrid({ widgets, ctx }: DashboardGridProps) {
  const isMobile = useIsMobile();
  const frames = widgets.map(widget => ({ widget, style: widgetGridStyle(widget, isMobile), deferred: shouldDeferWidget(widget) }));

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
      {frames.map(({ widget, style, deferred }) => {
        return (
          <div
            key={widget.id}
            data-widget-id={widget.id}
            data-widget-type={widget.componentType}
            data-widget-definition={widget.definitionId}
            style={style}
          >
            <DeferredWidget enabled={deferred}>
              <WidgetChrome instance={widget} ctx={ctx} mode="view" />
            </DeferredWidget>
          </div>
        );
      })}
    </div>
  );
}

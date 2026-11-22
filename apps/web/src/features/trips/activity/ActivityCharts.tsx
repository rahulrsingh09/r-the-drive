import React from 'react';
import { createPortal } from 'react-dom';
import { Maximize2, X } from 'lucide-react';
import { projectRoute, type RouteCoordinate } from './activityModel';

function useWidth<T extends HTMLElement>(fallback = 320) {
  const ref = React.useRef<T>(null);
  const [width, setWidth] = React.useState(fallback);
  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return undefined;
    const measure = () => {
      const next = Math.floor(element.clientWidth);
      if (next > 0) setWidth(Math.max(200, next));
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

export interface LinePoint { x: number; y: number; tip: string }

export interface LineChartProps {
  label: string;
  points: LinePoint[];
  yDomain: [number, number];
  ticks: number[];
  tickFormat?: (value: number) => string;
  /** CSS color, normally a token variable. */
  color: string;
  height?: number;
  area?: boolean;
  xDomain?: [number, number];
  xLabels?: Array<[number, string, 'start' | 'middle' | 'end']>;
  /** Labels at the first and last points. */
  endLabels?: [string, string];
  reference?: { value: number; label: string } | null;
  /** Controlled cursor (for example the trip replay), in x units. */
  cursorX?: number | null;
  testId?: string;
}

/** Line chart in the R chart style: fixed y domain, labels inside the plot, a hover or replay cursor. */
export function LineChart({
  label, points, yDomain, ticks, tickFormat = String, color, height = 150, area = false, xDomain, xLabels = [], endLabels,
  reference = null, cursorX = null, testId,
}: LineChartProps) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = React.useState<number | null>(null);
  const pt = 14;
  const pb = 22;
  const xs = points.map((point) => point.x);
  const [x0, x1] = xDomain ?? [Math.min(...xs), Math.max(...xs)];
  const [y0, y1] = yDomain;
  const X = (value: number) => ((value - x0) / ((x1 - x0) || 1)) * width;
  const Y = (value: number) => pt + (1 - (value - y0) / ((y1 - y0) || 1)) * (height - pt - pb);
  const plotted = points.map((point) => ({ x: X(point.x), y: Y(Math.max(y0, Math.min(y1, point.y))), tip: point.tip, raw: point.x }));
  const path = plotted.map((point, index) => `${index ? 'L' : 'M'}${point.x.toFixed(1)},${point.y.toFixed(1)}`).join('');
  const first = plotted[0];
  const last = plotted.at(-1);

  const nearest = (rawX: number) => {
    let best = 0;
    for (let index = 0; index < plotted.length; index += 1) if (Math.abs(plotted[index]!.raw - rawX) < Math.abs(plotted[best]!.raw - rawX)) best = index;
    return best;
  };
  const lastAtOrBefore = (rawX: number) => {
    let index = 0;
    while (index < plotted.length - 1 && plotted[index + 1]!.raw <= rawX) index += 1;
    return index;
  };
  const active = hover ?? (cursorX != null && plotted.length ? lastAtOrBefore(cursorX) : null);
  const activePoint = active != null ? plotted[active] : null;
  const onMove = (event: React.PointerEvent<SVGRectElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    if (!box.width || !plotted.length) return;
    const rawX = x0 + ((event.clientX - box.left) / box.width) * (x1 - x0);
    setHover(nearest(rawX));
  };

  return (
    <div ref={ref} className="r-act-chart" data-testid={testId}>
      {plotted.length < 2 ? null : (
        <svg viewBox={`0 0 ${width} ${height}`} height={height} role="img" aria-label={label}>
          <g className="r-act-grid">{ticks.map((tick) => <line key={tick} x1={0} x2={width} y1={Y(tick)} y2={Y(tick)} />)}</g>
          {reference ? (
            <g className="r-act-ref">
              <line x1={0} x2={width} y1={Y(reference.value)} y2={Y(reference.value)} />
              <text x={width / 2} y={Y(reference.value) - 5} textAnchor="middle">{reference.label}</text>
            </g>
          ) : null}
          {area && first && last ? <path d={`${path}L${last.x},${Y(y0)}L${first.x},${Y(y0)}Z`} fill={color} opacity={0.12} /> : null}
          <path d={path} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" className="r-act-draw" pathLength={1} />
          {endLabels && first && last ? (
            <g className="r-act-ends">
              <circle cx={first.x} cy={first.y} r={4} fill={color} />
              <text x={first.x} y={first.y - 9} textAnchor="start">{endLabels[0]}</text>
              <circle cx={last.x} cy={last.y} r={4} fill={color} />
              <text x={last.x} y={last.y - 9} textAnchor="end">{endLabels[1]}</text>
            </g>
          ) : null}
          <g className="r-act-axis">
            {ticks.filter((tick) => !(first && Math.abs(first.y - Y(tick)) < 16 && first.x < 70)).map((tick) => (
              <text key={tick} x={2} y={Y(tick) - 4}>{tickFormat(tick)}</text>
            ))}
            {xLabels.map(([value, text, anchor]) => <text key={`${value}-${text}`} x={X(value)} y={height - 4} textAnchor={anchor}>{text}</text>)}
          </g>
          {activePoint ? (
            <g className="r-act-cursor" data-chart-cursor>
              <line x1={activePoint.x} x2={activePoint.x} y1={pt} y2={height - pb} />
              <circle cx={activePoint.x} cy={activePoint.y} r={5} fill={color} />
            </g>
          ) : null}
          <rect x={0} y={0} width={width} height={height} fill="transparent" onPointerMove={onMove} onPointerLeave={() => setHover(null)} />
        </svg>
      )}
      {hover != null && activePoint ? (
        <div className="r-act-tip" role="status" style={{ left: Math.min(Math.max(activePoint.x, 60), width - 60), top: activePoint.y }}>
          {activePoint.tip}
        </div>
      ) : null}
    </div>
  );
}

export interface StripBar { segments: Array<{ value: number; color: string }>; tip: string }

/** Small per-day bar strip for the summary card. No axis. */
export function BarStrip({ label, bars, height = 44 }: { label: string; bars: StripBar[]; height?: number }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = React.useState<number | null>(null);
  const n = Math.max(1, bars.length);
  const gap = Math.max(1, Math.min(10, (width / n) * 0.3));
  const barWidth = Math.max(1, (width - gap * (n - 1)) / n);
  const max = Math.max(1e-6, ...bars.map((bar) => bar.segments.reduce((sum, segment) => sum + segment.value, 0)));
  const base = height - 1;
  const H = (value: number) => (value / max) * (height - 3);
  return (
    <div ref={ref} className="r-act-chart r-act-strip">
      <svg viewBox={`0 0 ${width} ${height}`} height={height} role="img" aria-label={label}>
        <line x1={0} x2={width} y1={base} y2={base} className="r-act-baseline" />
        {bars.map((bar, index) => {
          const x = index * (barWidth + gap);
          let y = base;
          return (
            <g key={index} onPointerEnter={() => setHover(index)} onPointerLeave={() => setHover(null)}>
              {bar.segments.map((segment, part) => {
                const h = H(segment.value);
                if (h <= 0) return null;
                y -= h;
                return <rect key={part} x={x} y={y} width={barWidth} height={h} rx={Math.min(2, barWidth / 2)} fill={segment.color} />;
              })}
              <rect x={x} y={0} width={barWidth + gap} height={height} fill="transparent" />
            </g>
          );
        })}
      </svg>
      {hover != null && bars[hover] ? (
        <div className="r-act-tip" role="status" style={{ left: Math.min(Math.max(hover * (barWidth + gap), 60), width - 60), top: 0 }}>{bars[hover]!.tip}</div>
      ) : null}
    </div>
  );
}

/** Small route drawing for a trip row: grey line, start dot, accent end dot. */
export function RouteGlyph({ route, size = 44 }: { route: RouteCoordinate[] | null; size?: number }) {
  const points = projectRoute(route, size, size, 8);
  if (!points) {
    return (
      <svg viewBox="0 0 24 24" width={22} height={22} aria-hidden="true" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
        <path d="M3 15.5 5 10a3 3 0 0 1 2.8-2h8.4A3 3 0 0 1 19 10l2 5.5V18a1 1 0 0 1-1 1h-1.5a1 1 0 0 1-1-1v-1H6.5v1a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" />
      </svg>
    );
  }
  const d = points.map((point, index) => `${index ? 'L' : 'M'}${point[0].toFixed(1)},${point[1].toFixed(1)}`).join('');
  const end = points.at(-1)!;
  return (
    <svg viewBox={`0 0 ${size} ${size}`} aria-hidden="true" className="r-act-glyph">
      <path d={d} fill="none" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" className="r-act-glyph-line" />
      <circle cx={points[0]![0]} cy={points[0]![1]} r={2.5} className="r-act-glyph-start" />
      <circle cx={end[0]} cy={end[1]} r={3.5} className="r-act-glyph-end" />
    </svg>
  );
}

/**
 * A detail chart with the "Expand" control of the other charts: it opens the chart full screen,
 * taller. `render(big)` draws the chart; `big` is true in the full-screen view.
 */
export function ExpandableChart({ title, render }: { title: string; render: (big: boolean) => React.ReactNode }) {
  const [open, setOpen] = React.useState(false);
  const close = React.useCallback(() => setOpen(false), []);
  return (
    <>
      {render(false)}
      <div className="r-act-chart-tools">
        <button type="button" className="r-act-xlink" onClick={() => setOpen(true)} aria-label={`Expand ${title}`} data-chart-expand>
          <Maximize2 aria-hidden="true" />Expand
        </button>
      </div>
      {open && <ChartSheet title={title} render={render} onClose={close} />}
    </>
  );
}

function ChartSheet({ title, render, onClose }: { title: string; render: (big: boolean) => React.ReactNode; onClose: () => void }) {
  const closeRef = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    closeRef.current?.focus();
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      document.body.style.overflow = overflow;
      opener?.focus?.();
    };
  }, [onClose]);
  return createPortal(
    <div className="r-act r-act-sheet" role="dialog" aria-modal="true" aria-label={title} onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="r-act-sheet-in">
        <div className="r-act-sheet-h"><b>{title}</b>
          <button ref={closeRef} type="button" className="r-act-iconbtn" onClick={onClose} aria-label="Close"><X aria-hidden="true" /></button>
        </div>
        {render(true)}
      </div>
    </div>,
    document.body,
  );
}

import React from 'react';
import { createPortal } from 'react-dom';
import { Link } from '@tanstack/react-router';
import { ChevronRight, Maximize2, X } from 'lucide-react';

/* ------------------------------------------------------------------ tooltip */

/**
 * A tooltip is a first line in bold and more lines under it.
 * Marks carry it in `data-tip` as text with "\n" between lines; the page root shows it on hover or tap.
 */
export const tip = (title: string, ...lines: string[]) => [title, ...lines].join('\n');

interface TipState { x: number; y: number; text: string }
const TipContext = React.createContext<{ show: (x: number, y: number, text: string) => void; hide: () => void }>({
  show: () => undefined, hide: () => undefined,
});
export const useTip = () => React.useContext(TipContext);

/** Holds the one tooltip of the page. Wrap the page body in it. */
export function TipLayer({ children, className }: { children: React.ReactNode; className?: string }) {
  const [state, setState] = React.useState<TipState | null>(null);
  const value = React.useMemo(() => ({
    show: (x: number, y: number, text: string) => setState({ x, y, text }),
    hide: () => setState(null),
  }), []);
  const onPointer = (event: React.PointerEvent) => {
    const target = event.target as Element | null;
    const mark = target?.closest?.('[data-tip]') as HTMLElement | SVGElement | null;
    if (mark?.dataset.tip) {
      const rect = mark.getBoundingClientRect();
      value.show(rect.left + rect.width / 2, rect.top, mark.dataset.tip);
      return;
    }
    if (!target?.closest?.('[data-line]')) value.hide();
  };
  const [title, ...lines] = state?.text.split('\n') ?? [];
  return (
    <TipContext.Provider value={value}>
      <div className={className} onPointerMove={onPointer} onPointerDown={onPointer} onPointerLeave={value.hide}>
        {children}
      </div>
      {state && createPortal(
        <div className="ri-tip ri-tip-on" role="status" style={{ left: state.x, top: state.y }}>
          <b>{title}</b>{lines.map((line, index) => <React.Fragment key={index}><br />{line}</React.Fragment>)}
        </div>,
        document.body,
      )}
    </TipContext.Provider>
  );
}

/* ------------------------------------------------------------------ motion and size */

export function prefersReducedMotion() {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return true;
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** Counts up to the value on first view; reduced motion shows the value at once. */
export function CountUp({ value, decimals = 0, prefix = '' }: { value: number; decimals?: number; prefix?: string }) {
  const format = (v: number) => prefix + v.toLocaleString('en-US', { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
  const [shown, setShown] = React.useState(() => (prefersReducedMotion() ? value : 0));
  React.useEffect(() => {
    if (prefersReducedMotion() || typeof requestAnimationFrame !== 'function') { setShown(value); return; }
    const t0 = performance.now(); let frame = 0;
    const tick = (t: number) => {
      const p = Math.min(1, (t - t0) / 900);
      setShown(value * (1 - (1 - p) ** 3));
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [value]);
  return <span className="ri-num">{format(shown)}</span>;
}

function useWidth<T extends HTMLElement>() {
  const [element, setElement] = React.useState<T | null>(null);
  const [width, setWidth] = React.useState(0);
  React.useLayoutEffect(() => {
    if (!element) return;
    setWidth(element.clientWidth);
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(entries => setWidth(Math.round(entries[0]?.contentRect.width ?? 0)));
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return [setElement, Math.max(240, Math.floor(width))] as const;
}

/* ------------------------------------------------------------------ chart frame */

/** Draws an SVG chart at the width of its box. `big` is true in the full-screen view. */
export type ChartRender = (width: number, big: boolean) => React.ReactNode;

function ChartBox({ render, big = false }: { render: ChartRender; big?: boolean }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  return <div ref={ref} className="ri-chart">{render(width, big)}</div>;
}

/** A chart with "Show as table" (when it has a table) and "Expand", which opens it full screen with its table. */
export function Chart({ title, render, table, tools = true }: {
  title: string;
  render: ChartRender;
  table?: (() => React.ReactNode) | undefined;
  tools?: boolean;
}) {
  const [showTable, setShowTable] = React.useState(false);
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <ChartBox render={render} />
      {tools && (
        <div className="ri-chart-tools">
          {table ? <button type="button" className="ri-tablelink" aria-expanded={showTable} onClick={() => setShowTable(v => !v)}>
            {showTable ? 'Hide table' : 'Show as table'}
          </button> : <span />}
          <button type="button" className="ri-tablelink ri-xlink" onClick={() => setOpen(true)} aria-label={`Expand ${title}`}>
            <Maximize2 aria-hidden="true" />Expand
          </button>
        </div>
      )}
      {table && showTable && table()}
      {open && <ChartSheet title={title} render={render} table={table} onClose={() => setOpen(false)} />}
    </>
  );
}

function ChartSheet({ title, render, table, onClose }: { title: string; render: ChartRender; table?: (() => React.ReactNode) | undefined; onClose: () => void }) {
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
    <div className="ri ri-sheet" role="dialog" aria-modal="true" aria-label={title} onClick={event => { if (event.target === event.currentTarget) onClose(); }}>
      <TipLayer className="ri-sheet-in">
        <div className="ri-sheet-h"><b>{title}</b>
          <button ref={closeRef} type="button" className="ri-iconbtn" onClick={onClose} aria-label="Close"><X aria-hidden="true" /></button>
        </div>
        <ChartBox render={render} big />
        {table && <div className="ri-sheet-t">{table()}</div>}
      </TipLayer>
    </div>,
    document.body,
  );
}

/* ------------------------------------------------------------------ cards, legend, table */

export function Card({ title, sub, wide, index = 0, link, children, label }: {
  title: string;
  sub?: string;
  wide?: boolean;
  index?: number;
  link?: { to: string; label: string; search?: Record<string, string> | undefined };
  children: React.ReactNode;
  label?: string;
}) {
  return (
    <section className={`ri-card ri-pad ri-rise${wide ? ' ri-wide' : ''}`} style={{ '--i': index } as React.CSSProperties} aria-label={label ?? title}>
      <div className="ri-ctitle"><h2>{title}</h2><span className="ri-muted">{sub}</span></div>
      {children}
      {link && <MoreLink to={link.to} search={link.search}>{link.label}</MoreLink>}
    </section>
  );
}

export function MoreLink({ to, search, children }: { to: string; search?: Record<string, string> | undefined; children: React.ReactNode }) {
  if (search) {
    const href = `${to}?${new URLSearchParams(search)}`;
    return <AppLink href={href} className="ri-morelink">{children} <ChevronRight aria-hidden="true" /></AppLink>;
  }
  return <Link to={to as '/'} className="ri-morelink">{children} <ChevronRight aria-hidden="true" /></Link>;
}

/** A link to any in-app URL (including a query string) that navigates without a full reload. */
export function AppLink({ href, className, children, ...rest }: React.AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  return (
    <a href={href} className={className} {...rest} onClick={event => {
      rest.onClick?.(event);
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      event.preventDefault();
      window.history.pushState({}, '', href);
      window.dispatchEvent(new PopStateEvent('popstate'));
    }}>{children}</a>
  );
}

export function Legend({ items }: { items: ReadonlyArray<{ label: string; color: string; dashed?: boolean; line?: boolean }> }) {
  return (
    <div className="ri-legend">
      {items.map(item => (
        <span key={item.label}>
          <i aria-hidden="true" className={item.line ? 'ri-legend-line' : undefined}
            style={item.line ? { borderTopStyle: item.dashed ? 'dashed' : 'solid', borderTopColor: item.color } : { background: item.color }} />
          {item.label}
        </span>
      ))}
    </div>
  );
}

export function DataTable({ head, rows, caption }: { head: readonly string[]; rows: ReadonlyArray<ReadonlyArray<React.ReactNode>>; caption?: string }) {
  return (
    <table className="ri-data">
      {caption && <caption className="ri-sr">{caption}</caption>}
      <thead><tr>{head.map(h => <th key={h} scope="col">{h}</th>)}</tr></thead>
      <tbody>{rows.map((row, r) => <tr key={r}>{row.map((cell, c) => <td key={c}>{cell}</td>)}</tr>)}</tbody>
    </table>
  );
}

/* ------------------------------------------------------------------ charts */

const scaleH = (h: number, big: boolean) => (big ? h * 2.2 : h);

/** Estimated width of one character of an 11 px axis label (Inter stays under 0.62 em per character). */
const AXIS_CHAR_PX = 6.8;
/** Space kept between two axis labels. */
const AXIS_LABEL_GAP = 14;

export interface AxisLabel { x: number; text: string; anchor: 'start' | 'middle' | 'end' }

/**
 * The axis labels that fit side by side in `width` px: every k-th label, counted back from the newest,
 * with the smallest k that keeps AXIS_LABEL_GAP px between neighbours. Labels that cross an edge are left out.
 */
export function fitAxisLabels<T extends AxisLabel>(labels: readonly T[], width: number): T[] {
  const span = (label: AxisLabel) => {
    const size = label.text.length * AXIS_CHAR_PX;
    const left = label.anchor === 'start' ? label.x : label.anchor === 'end' ? label.x - size : label.x - size / 2;
    return [left, left + size] as const;
  };
  const inside = labels.filter(label => { const [left, right] = span(label); return left >= -0.5 && right <= width + 0.5; });
  for (let k = 1; k <= inside.length; k++) {
    const kept = inside.filter((_, i) => (inside.length - 1 - i) % k === 0);
    if (kept.every((label, i) => i === 0 || span(label)[0] - span(kept[i - 1]!)[1] >= AXIS_LABEL_GAP)) return kept;
  }
  return inside.slice(-1);
}
const pathOf = (points: ReadonlyArray<{ x: number; y: number }>) =>
  points.map((p, i) => `${i ? 'L' : 'M'}${p.x.toFixed(1)},${p.y.toFixed(1)}`).join('');
const lengthOf = (points: ReadonlyArray<{ x: number; y: number }>) =>
  Math.ceil(points.reduce((s, p, i) => (i ? s + Math.hypot(p.x - points[i - 1]!.x, p.y - points[i - 1]!.y) : s), 0));

export interface LinePoint { x: number; y: number; tip?: string }

/** A line with labels inside the plot, so the axis takes no width. */
export function LineChart({ width: w, big, label, points, xDomain, yDomain, ticks = [], tickFormat, color, area, dots, endLabels = [], xLabels = [], height = 150 }: {
  width: number; big: boolean; label: string; points: readonly LinePoint[];
  xDomain?: [number, number] | undefined; yDomain: [number, number]; ticks?: readonly number[]; tickFormat?: ((v: number) => string) | undefined;
  color: string; area?: boolean; dots?: boolean; height?: number;
  endLabels?: ReadonlyArray<[number, string, 'start' | 'middle' | 'end']>;
  xLabels?: ReadonlyArray<[number, string, 'start' | 'middle' | 'end']>;
}) {
  const { show, hide } = useTip();
  const [hover, setHover] = React.useState<number | null>(null);
  const h = scaleH(height, big); const pt = 14; const pb = 22;
  if (!points.length) return <p className="ri-empty">No data yet.</p>;
  const xs = points.map(p => p.x);
  const [x0, x1] = xDomain ?? [Math.min(...xs), Math.max(...xs)];
  const [y0, y1] = yDomain;
  const X = (v: number) => (v - x0) / ((x1 - x0) || 1) * w;
  const Y = (v: number) => pt + (1 - (v - y0) / ((y1 - y0) || 1)) * (h - pt - pb);
  const P = points.map(p => ({ x: X(p.x), y: Y(p.y), tip: p.tip }));
  const d = pathOf(P);
  const at = (i: number) => P[i < 0 ? P.length + i : i]!;
  const ends = endLabels.map(([i]) => at(i));
  const move = (event: React.PointerEvent<SVGRectElement>) => {
    const svg = event.currentTarget.ownerSVGElement!; const rect = svg.getBoundingClientRect();
    const x = (event.clientX - rect.left) * (w / (rect.width || w));
    let best = 0; P.forEach((p, i) => { if (Math.abs(p.x - x) < Math.abs(P[best]!.x - x)) best = i; });
    setHover(best);
    const p = P[best]!;
    if (p.tip) show(rect.left + p.x * rect.width / w, rect.top + p.y * rect.height / h, p.tip); else hide();
  };
  const hp = hover != null ? P[hover] : null;
  return (
    <svg viewBox={`0 0 ${w} ${h}`} height={h} role="img" aria-label={label}>
      <g className="ri-grid">{ticks.map(t => <line key={t} x1="0" x2={w} y1={Y(t)} y2={Y(t)} />)}</g>
      {area && P.length > 1 && <path d={`${d}L${P[P.length - 1]!.x},${Y(y0)}L${P[0]!.x},${Y(y0)}Z`} fill={color} opacity=".12" className="ri-fade" />}
      <path d={d} fill="none" stroke={color} strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" className="ri-draw" style={{ '--len': lengthOf(P) } as React.CSSProperties} />
      {dots && P.map((p, i) => <circle key={i} cx={p.x} cy={p.y} r="4" fill={color} stroke="var(--ri-s1)" strokeWidth="2" />)}
      {endLabels.map(([i, text, anchor], k) => { const p = at(i); return (
        <g key={k}><circle cx={p.x} cy={p.y} r="4" fill={color} stroke="var(--ri-s1)" strokeWidth="2" />
          <text className="ri-lab1" x={p.x} y={p.y - 9} textAnchor={anchor}>{text}</text></g>); })}
      <g className="ri-axis">
        {ticks.filter(t => !ends.some(p => Math.abs(p.y - Y(t)) < 16 && p.x < 70)).map(t => <text key={t} x="2" y={Y(t) - 4}>{tickFormat ? tickFormat(t) : t}</text>)}
        {fitAxisLabels(xLabels.map(([v, text, anchor]) => ({ x: X(v), text, anchor })), w)
          .map((item, k) => <text key={k} x={item.x} y={h - 4} textAnchor={item.anchor}>{item.text}</text>)}
      </g>
      {hp && <><line x1={hp.x} x2={hp.x} y1={pt} y2={h - pb} stroke="var(--ri-t3)" strokeWidth="1" />
        <circle cx={hp.x} cy={hp.y} r="5" fill={color} stroke="var(--ri-s1)" strokeWidth="2" /></>}
      <rect data-line="" x="0" y="0" width={w} height={h} fill="transparent" onPointerMove={move} onPointerDown={move}
        onPointerLeave={() => { setHover(null); hide(); }} />
    </svg>
  );
}

export interface BarSegment { v: number; color: string; tip?: string }
export interface Bar { label?: string; top?: string; zero?: boolean; segs: readonly BarSegment[] }

/** Stacked bars with direct value labels and no y axis. */
export function BarChart({ width: w, big, label, bars, max, height = 150, pt = 20, pb = 22 }: {
  width: number; big: boolean; label: string; bars: readonly Bar[]; max?: number; height?: number; pt?: number; pb?: number;
}) {
  const h = scaleH(height, big); const n = bars.length;
  if (!n) return <p className="ri-empty">No data yet.</p>;
  const gap = Math.max(2, Math.min(10, w / n * 0.3)); const bw = (w - gap * (n - 1)) / n;
  const top = max ?? Math.max(...bars.map(b => b.segs.reduce((s, x) => s + x.v, 0)), 1);
  const H = (v: number) => v / (top || 1) * (h - pt - pb);
  const axis = fitAxisLabels(bars.flatMap((bar, i) => {
    if (!bar.label) return [];
    const x = i * (bw + gap); const edge = n > 12 && (i === 0 || i === n - 1);
    const anchor: AxisLabel['anchor'] = edge ? (i === 0 ? 'start' : 'end') : 'middle';
    return [{ x: edge ? (i === 0 ? x : x + bw) : x + bw / 2, text: bar.label, anchor }];
  }), w);
  return (
    <svg viewBox={`0 0 ${w} ${h}`} height={h} role="img" aria-label={label}>
      <line x1="0" x2={w} y1={h - pb} y2={h - pb} stroke="var(--ri-line2)" />
      {bars.map((bar, i) => {
        const x = i * (bw + gap); let y = h - pb; const total = bar.segs.reduce((s, x) => s + x.v, 0);
        return (
          <g key={i}>
            {bar.segs.map((seg, j) => {
              const hh = H(seg.v); if (hh <= 0) return null;
              const last = bar.segs.slice(j + 1).every(z => !z.v); const r = last ? Math.min(4, bw / 2, hh) : 0;
              const y2 = y - hh;
              const d = `M${x},${y}V${y2 + r}${r ? `Q${x},${y2} ${x + r},${y2}H${x + bw - r}Q${x + bw},${y2} ${x + bw},${y2 + r}` : `H${x + bw}`}V${y}Z`;
              y = y2 - (!last && hh > 2 ? 2 : 0);
              return <path key={j} className="ri-grow" style={{ '--i': i } as React.CSSProperties} d={d} fill={seg.color} data-tip={seg.tip} />;
            })}
            {bar.top && (total > 0 || bar.zero) && <text className="ri-lab" x={x + bw / 2} y={h - pb - H(total) - 6} textAnchor="middle">{bar.top}</text>}
          </g>
        );
      })}
      {axis.map((item, k) => <text key={k} className="ri-ax" x={item.x} y={h - 5} textAnchor={item.anchor}>{item.text}</text>)}
    </svg>
  );
}

export interface ScatterPoint { x: number; y: number; r?: number; tip: string }

export function Scatter({ width: w, big, label, points, xDomain, yDomain, xTicks, yTicks, xUnit = '', color, height = 180 }: {
  width: number; big: boolean; label: string; points: readonly ScatterPoint[]; xDomain: [number, number]; yDomain: [number, number];
  xTicks: readonly number[]; yTicks: readonly number[]; xUnit?: string; color: string; height?: number;
}) {
  const h = scaleH(height, big); const pt = 14; const pb = 22; const [x0, x1] = xDomain; const [y0, y1] = yDomain;
  const X = (v: number) => 8 + (v - x0) / ((x1 - x0) || 1) * (w - 16);
  const Y = (v: number) => pt + (1 - (v - y0) / ((y1 - y0) || 1)) * (h - pt - pb);
  return (
    <svg viewBox={`0 0 ${w} ${h}`} height={h} role="img" aria-label={label}>
      <g className="ri-grid">{yTicks.map(t => <line key={t} x1="0" x2={w} y1={Y(t)} y2={Y(t)} />)}</g>
      <g className="ri-axis">
        {yTicks.map(t => <text key={t} x="2" y={Y(t) - 4}>{t}</text>)}
        {xTicks.map((t, i) => <text key={t} x={X(t)} y={h - 4} textAnchor={i === 0 ? 'start' : i === xTicks.length - 1 ? 'end' : 'middle'}>{`${t}${xUnit}`}</text>)}
      </g>
      {points.map((p, i) => <circle key={i} className="ri-fade" style={{ animationDelay: `${i * 6}ms` }} cx={X(p.x)} cy={Y(p.y)} r={p.r ?? 5}
        fill={color} fillOpacity=".75" stroke="var(--ri-s1)" strokeWidth="2" data-tip={p.tip} />)}
    </svg>
  );
}

export interface Series { name: string; dash: string; points: readonly LinePoint[] }

export function MultiLine({ width: w, big, label, series, xDomain, yDomain, xTicks, yTicks, xUnit = '', yUnit = '', color, height = 190 }: {
  width: number; big: boolean; label: string; series: readonly Series[]; xDomain: [number, number]; yDomain: [number, number];
  xTicks: readonly number[]; yTicks: readonly number[]; xUnit?: string; yUnit?: string; color: string; height?: number;
}) {
  const h = scaleH(height, big); const pt = 14; const pb = 22; const [x0, x1] = xDomain; const [y0, y1] = yDomain;
  const X = (v: number) => 4 + (v - x0) / ((x1 - x0) || 1) * (w - 8);
  const Y = (v: number) => pt + (1 - (v - y0) / ((y1 - y0) || 1)) * (h - pt - pb);
  return (
    <svg viewBox={`0 0 ${w} ${h}`} height={h} role="img" aria-label={label}>
      <g className="ri-grid">{yTicks.map(t => <line key={t} x1="0" x2={w} y1={Y(t)} y2={Y(t)} />)}</g>
      <g className="ri-axis">
        {yTicks.map(t => <text key={t} x="2" y={Y(t) - 4}>{`${t}${yUnit}`}</text>)}
        {xTicks.map((t, i) => <text key={t} x={X(t)} y={h - 4} textAnchor={i === 0 ? 'start' : i === xTicks.length - 1 ? 'end' : 'middle'}>{`${t}${xUnit}`}</text>)}
      </g>
      {series.map((s, k) => {
        const P = s.points.filter(p => p.x >= x0 && p.x <= x1);
        if (P.length < 2) return null;
        return (
          <g key={k}>
            <path d={pathOf(P.map(p => ({ x: X(p.x), y: Y(p.y) })))} fill="none" stroke={color} strokeWidth="2" strokeDasharray={s.dash || undefined} strokeLinejoin="round" className="ri-fade" />
            {P.map((p, i) => <circle key={i} cx={X(p.x)} cy={Y(p.y)} r="7" fill="transparent" data-tip={p.tip} />)}
          </g>
        );
      })}
    </svg>
  );
}

export interface RangeBar { lo: number; hi: number; color: string; tip: string; index: number }

/** Vertical range bars on a 0–100 % axis with a dashed limit line. */
export function RangeBars({ width: w, big, label, bars, count, limit, startLabel, endLabel, height = 165 }: {
  width: number; big: boolean; label: string; bars: readonly RangeBar[]; count: number; limit: number | null;
  startLabel: string; endLabel: string; height?: number;
}) {
  const h = scaleH(height, big); const pt = 10; const pb = 22; const left = 40;
  const n = Math.max(1, count);
  const X = (i: number) => left + (i + 0.5) / n * (w - left - 4);
  const Y = (v: number) => pt + (1 - v / 100) * (h - pt - pb);
  const bw = Math.max(3, Math.min(10, (w - left - 4) / n - 3));
  return (
    <svg viewBox={`0 0 ${w} ${h}`} height={h} role="img" aria-label={label}>
      <g className="ri-grid">{[0, 50, 100].map(t => <line key={t} x1="0" x2={w} y1={Y(t)} y2={Y(t)} />)}</g>
      {limit != null && <line x1="0" x2={w} y1={Y(limit)} y2={Y(limit)} stroke="var(--ri-t2)" strokeDasharray="3 4" strokeWidth="1.5" />}
      <g className="ri-axis">
        {[0, 50, 100].map(t => <text key={t} x="2" y={Y(t) - 4}>{`${t}%`}</text>)}
        <text x={left} y={h - 4}>{startLabel}</text><text x={w - 4} y={h - 4} textAnchor="end">{endLabel}</text>
      </g>
      {bars.map(bar => (
        <rect key={bar.index} className="ri-grow" style={{ '--i': bar.index } as React.CSSProperties} x={X(bar.index) - bw / 2} y={Y(bar.hi)} width={bw}
          height={Math.max(3, Y(bar.lo) - Y(bar.hi))} rx={bw / 2} fill={bar.color} data-tip={bar.tip} />
      ))}
    </svg>
  );
}

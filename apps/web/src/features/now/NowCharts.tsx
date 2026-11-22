import React from 'react';

/** Width of an element in CSS px; follows resizes. Starts at `fallback` (tests and the first paint). */
export function useWidth<T extends HTMLElement>(fallback = 300) {
  const ref = React.useRef<T>(null);
  const [width, setWidth] = React.useState(fallback);
  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => { const next = Math.floor(element.clientWidth); if (next > 0) setWidth(next); };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

export interface Bar { key: string; value: number; color: string; label?: string; title: string }

/** Bars with no y axis. Each bar has a native tooltip; the chart has one accessible name. */
export function NowBarChart({ bars, height, label, max, padBottom = 0, className }: {
  bars: readonly Bar[]; height: number; label: string; max?: number; padBottom?: number; className?: string;
}) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const count = Math.max(1, bars.length);
  const gap = Math.max(2, Math.min(10, (width / count) * 0.3));
  const barWidth = Math.max(1, (width - gap * (count - 1)) / count);
  const top = Math.max(max ?? 0, ...bars.map(bar => bar.value), 1e-9);
  const plot = height - padBottom - 2;
  return <div ref={ref} className={`now-chart ${className ?? ''}`}>
    <svg viewBox={`0 0 ${width} ${height}`} height={height} width="100%" role="img" aria-label={label}>
      <line className="now-chart-base" x1={0} x2={width} y1={height - padBottom} y2={height - padBottom} />
      {bars.map((bar, index) => {
        const x = index * (barWidth + gap);
        const h = bar.value > 0 ? Math.max(2, (bar.value / top) * plot) : 0;
        const y = height - padBottom - h;
        const r = Math.min(4, barWidth / 2, h);
        return <g key={bar.key}>
          {h > 0 && <path className="now-grow" style={{ '--i': index } as React.CSSProperties} fill={bar.color}
            d={`M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + barWidth - r}Q${x + barWidth},${y} ${x + barWidth},${y + r}V${y + h}Z`}>
            <title>{bar.title}</title>
          </path>}
          {h === 0 && <rect x={x} y={0} width={barWidth} height={height - padBottom} fill="transparent"><title>{bar.title}</title></rect>}
          {bar.label && <text className="now-chart-axis" x={x + barWidth / 2} y={height - 4} textAnchor="middle">{bar.label}</text>}
        </g>;
      })}
    </svg>
  </div>;
}

export interface LinePoint { x: number; y: number; title?: string }

/** A line in a fixed y domain. Grid labels sit inside the plot, so the axis takes no width. */
export function NowLineChart({ points, height, label, yDomain, xDomain, color, dots, area, ticks = [], tickFormat, marker, padTop = 10, padBottom = 4 }: {
  points: readonly LinePoint[]; height: number; label: string; yDomain: readonly [number, number];
  xDomain?: readonly [number, number]; color: string; dots?: boolean; area?: boolean; ticks?: readonly number[];
  tickFormat?: (value: number) => string; marker?: LinePoint | null; padTop?: number; padBottom?: number;
}) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const xs = points.map(point => point.x);
  const [x0, x1] = xDomain ?? [Math.min(...xs, 0), Math.max(...xs, 1)];
  const [y0, y1] = yDomain;
  const inset = dots || marker ? 6 : 0;
  const X = (value: number) => inset + ((value - x0) / ((x1 - x0) || 1)) * (width - inset * 2);
  const Y = (value: number) => padTop + (1 - (value - y0) / ((y1 - y0) || 1)) * (height - padTop - padBottom);
  const path = points.map((point, index) => `${index ? 'L' : 'M'}${X(point.x).toFixed(1)},${Y(point.y).toFixed(1)}`).join('');
  return <div ref={ref} className="now-chart">
    <svg viewBox={`0 0 ${width} ${height}`} height={height} width="100%" role="img" aria-label={label}>
      <g className="now-chart-grid">{ticks.map(tick => <line key={tick} x1={0} x2={width} y1={Y(tick)} y2={Y(tick)} />)}</g>
      {ticks.map(tick => <text key={tick} className="now-chart-axis" x={2} y={Y(tick) - 4}>{tickFormat ? tickFormat(tick) : tick}</text>)}
      {area && points.length > 1 && <path d={`${path}L${X(points.at(-1)!.x)},${Y(y0)}L${X(points[0]!.x)},${Y(y0)}Z`} fill={color} opacity={0.12} />}
      {points.length > 1 && <path d={path} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" className="now-draw" pathLength={1} />}
      {dots && points.map((point, index) => <circle key={index} cx={X(point.x)} cy={Y(point.y)} r={4} fill={color} className="now-chart-dot">
        {point.title && <title>{point.title}</title>}
      </circle>)}
      {marker && <circle cx={X(marker.x)} cy={Y(marker.y)} r={6} fill={color} className="now-chart-dot now-pulse" />}
    </svg>
  </div>;
}

/** A y domain with some room above and below the values. */
export function paddedDomain(values: readonly number[]): [number, number] {
  if (!values.length) return [0, 1];
  const low = Math.min(...values);
  const high = Math.max(...values);
  const pad = (high - low) * 0.35 || Math.abs(high) * 0.1 || 1;
  return [low - pad, high + pad];
}

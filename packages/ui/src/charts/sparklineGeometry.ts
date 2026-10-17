import { clampedControlPoints, type CurvePoint, type CurveSmoothness } from './curveSmoothness';
import { maximumContinuousTimeGapMilliseconds, type TimeFilterWindow } from './timeFilter';

type Sample = { x: string; value: number | null };
type LineCommand = { end: CurvePoint; controls?: [CurvePoint, CurvePoint] };
interface CurveRun { first: CurvePoint; last: CurvePoint; commands: LineCommand[] }
export type SparklineGeometry =
  | { kind: 'bars'; bars: Array<{ x: number; y: number; width: number; height: number; alpha: number }> }
  | { kind: 'curves'; runs: CurveRun[] };

export function prepareSparklineGeometry(data: Sample[], width: number, height: number, type: 'line' | 'bar', timeFilter: TimeFilterWindow, smoothness: CurveSmoothness, domain?: { min: number; max: number }): SparklineGeometry {
  let minimum = Infinity;
  let maximum = -Infinity;
  const times: number[] = [];
  let validTimes = true;
  for (const point of data) {
    if (point.value != null) { minimum = Math.min(minimum, point.value); maximum = Math.max(maximum, point.value); }
    const time = Date.parse(point.x);
    times.push(time);
    validTimes &&= Number.isFinite(time);
  }
  if (type === 'bar') {
    const peak = Math.max(0, maximum);
    const slot = Math.max(1, width / data.length);
    return { kind: 'bars', bars: data.map((point, index) => {
      const ratio = peak > 0 ? (point.value ?? 0) / peak : 0;
      const barHeight = ratio > 0 ? Math.max(2, ratio * (height - 6)) : 1;
      return { x: index * slot + slot * 0.18, y: height - 2 - barHeight, width: Math.max(1, slot * 0.64), height: barHeight, alpha: ratio > 0 ? 0.72 : 0.26 };
    }) };
  }
  const useDomain = domain != null && Number.isFinite(domain.min) && Number.isFinite(domain.max) && domain.max > domain.min;
  const bottom = useDomain ? domain.min : minimum;
  const span = (useDomain ? domain.max : maximum) - bottom;
  const useTime = validTimes && times.length > 1 && times[0] !== times[times.length - 1];
  const timeSpan = useTime ? times[times.length - 1]! - times[0]! : 1;
  const maximumGap = maximumContinuousTimeGapMilliseconds(timeFilter);
  const groups: CurvePoint[][] = [];
  let group: CurvePoint[] | undefined;
  let previous: number | undefined;
  data.forEach((sample, index) => {
    if (sample.value == null) { group = undefined; previous = undefined; return; }
    const timestamp = times[index]!;
    if (useTime && previous != null && timestamp - previous > maximumGap) group = undefined;
    if (!group) { group = []; groups.push(group); }
    group.push({
      x: useTime ? ((timestamp - times[0]!) / timeSpan) * width : (index / Math.max(1, data.length - 1)) * width,
      y: span === 0 ? height * 0.62 : height - 4 - ((sample.value - bottom) / span) * Math.max(1, height - 8),
    });
    previous = useTime ? timestamp : undefined;
  });
  return { kind: 'curves', runs: groups.map(points => ({
    first: points[0]!, last: points[points.length - 1]!,
    commands: points.slice(1).map((end, index) => smoothness === 'straight'
      ? { end } : { end, controls: clampedControlPoints(points, index, smoothness) }),
  })) };
}

function replayRun(context: CanvasRenderingContext2D, run: CurveRun) {
  for (const command of run.commands) {
    if (command.controls && typeof context.bezierCurveTo === 'function') {
      const [a, b] = command.controls;
      context.bezierCurveTo(a.x, a.y, b.x, b.y, command.end.x, command.end.y);
    } else context.lineTo(command.end.x, command.end.y);
  }
}

export function paintSparkline(context: CanvasRenderingContext2D, geometry: SparklineGeometry, color: string, height: number, area: boolean) {
  if (geometry.kind === 'bars') {
    context.fillStyle = color;
    for (const bar of geometry.bars) {
      context.globalAlpha = bar.alpha;
      context.fillRect(bar.x, bar.y, bar.width, bar.height);
    }
    context.globalAlpha = 1;
    return;
  }
  context.strokeStyle = color;
  context.lineWidth = 1.6;
  context.lineJoin = 'round';
  context.lineCap = 'round';
  if (area) {
    context.beginPath();
    for (const run of geometry.runs) {
      context.moveTo(run.first.x, height);
      context.lineTo(run.first.x, run.first.y);
      replayRun(context, run);
      context.lineTo(run.last.x, height);
      context.closePath();
    }
    context.globalAlpha = 0.16;
    context.fillStyle = color;
    context.fill();
    context.globalAlpha = 1;
  }
  context.beginPath();
  for (const run of geometry.runs) { context.moveTo(run.first.x, run.first.y); replayRun(context, run); }
  context.stroke();
  for (const run of geometry.runs) {
    if (run.commands.length) continue;
    context.beginPath();
    context.arc(run.first.x, run.first.y, 1.6, 0, Math.PI * 2);
    context.fillStyle = color;
    context.fill();
  }
}

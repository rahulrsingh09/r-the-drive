import { CHART_FONT } from './ChartProvider';

export interface ChartLabelMeasure {
  text: string;
  width: number;
}

export interface ValueLabelCandidate {
  index: number;
  value: number;
  x: number;
  y: number;
  width: number;
  height?: number;
}

const FALLBACK_GLYPH_WIDTH = 0.62;

/** Measures with the chart font when canvas is available, and stays deterministic in SSR/jsdom. */
export function measureChartText(text: string, font = CHART_FONT): number {
  if (typeof document !== 'undefined') {
    try {
      const canvas = document.createElement('canvas');
      const context = canvas.getContext('2d');
      if (context) {
        context.font = `${font.fontWeight} ${font.fontSize}px ${font.fontFamily}`;
        const measured = context.measureText(text).width;
        if (Number.isFinite(measured) && measured > 0) return measured;
      }
    } catch {
      // Canvas is optional (not available in SSR and some test environments).
    }
  }
  return Math.max(1, text.length * font.fontSize * FALLBACK_GLYPH_WIDTH);
}

export function measureChartLabels(labels: string[], font = CHART_FONT): ChartLabelMeasure[] {
  return labels.map((text) => ({ text, width: measureChartText(text, font) }));
}

export interface AxisLabelLayoutOptions {
  minSpacing?: number;
  font?: typeof CHART_FONT;
}

/** Returns the densest readable set of x-axis labels, retaining endpoints when they fit. */
export function selectAdaptiveAxisLabelIndices(
  labels: string[],
  xPositions: number[],
  options: AxisLabelLayoutOptions = {},
): number[] {
  const count = Math.min(labels.length, xPositions.length);
  if (!count) return [];
  const gap = options.minSpacing ?? 12;
  const boxes = labels.slice(0, count).map((label, index) => ({ center: xPositions[index] ?? 0, radius: measureChartText(label, options.font) / 2 }));
  const compatible = (a: number, b: number) => Math.abs(boxes[a]!.center - boxes[b]!.center) >= boxes[a]!.radius + boxes[b]!.radius + gap;
  const chosen = [0];
  for (let index = 1; index < count - 1; index += 1) {
    if (compatible(chosen[chosen.length - 1]!, index)) chosen.push(index);
  }
  const endpoint = count - 1;
  while (chosen.length > 1 && !compatible(chosen[chosen.length - 1]!, endpoint)) chosen.pop();
  if (endpoint && compatible(chosen[chosen.length - 1]!, endpoint)) chosen.push(endpoint);
  return chosen;
}


export const selectAxisLabelIndices = selectAdaptiveAxisLabelIndices;

/** Selects value labels by descending value while preventing padded bounding-box collisions. */
export function selectValueLabelIndices(
  candidates: ValueLabelCandidate[],
  padding = 6,
): number[] {
  const boxes = candidates.map((candidate, order) => ({
    index: candidate.index, order, value: candidate.value,
    left: candidate.x - candidate.width / 2 - padding,
    right: candidate.x + candidate.width / 2 + padding,
    top: candidate.y - (candidate.height ?? CHART_FONT.fontSize) - padding,
    bottom: candidate.y + padding,
  }));
  boxes.sort((a, b) => b.value - a.value || a.order - b.order);
  const visible: typeof boxes = [];
  const indices: number[] = [];
  for (const box of boxes) {
    let collision = false;
    for (const occupied of visible) {
      if (box.left < occupied.right && box.right > occupied.left && box.top < occupied.bottom && box.bottom > occupied.top) { collision = true; break; }
    }
    if (!collision) { visible.push(box); indices.push(box.index); }
  }
  return indices.sort((a, b) => a - b);
}


export const selectNonOverlappingValueLabels = selectValueLabelIndices;

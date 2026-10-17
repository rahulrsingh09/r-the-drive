export type CurveSmoothness = 'straight' | 'gentle' | 'smooth';

export const CURVE_SMOOTHNESS_OPTIONS: ReadonlyArray<{ value: CurveSmoothness; label: string }> = [
  { value: 'straight', label: 'Straight' },
  { value: 'gentle', label: 'Gentle' },
  { value: 'smooth', label: 'Smooth' },
];

export const DEFAULT_CURVE_SMOOTHNESS: CurveSmoothness = 'gentle';

const CURVES = new Map<unknown, CurveSmoothness>([
  ['straight', 'straight'], ['gentle', 'gentle'], ['smooth', 'smooth'],
  [false, 'straight'], [0, 'straight'], [true, 'gentle'],
]);
const CURVE_LABELS = new Map(CURVE_SMOOTHNESS_OPTIONS.map(option => [option.value, option.label]));

export function normalizeCurveSmoothness(value: unknown, fallback: CurveSmoothness = DEFAULT_CURVE_SMOOTHNESS): CurveSmoothness {
  const known = CURVES.get(value);
  if (known) return known;
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? 'gentle' : fallback;
}

export function curveSmoothnessLabel(value: CurveSmoothness): string {
  return CURVE_LABELS.get(value) ?? 'Gentle';
}


export interface CurvePoint {
  x: number;
  y: number;
}

export function splitCurveSegments(points: Array<CurvePoint | null>): CurvePoint[][] {
  const boundaries = [-1];
  for (let index = 0; index < points.length; index += 1) if (!points[index]) boundaries.push(index);
  boundaries.push(points.length);
  const result: CurvePoint[][] = [];
  for (let index = 1; index < boundaries.length; index += 1) {
    const from = boundaries[index - 1]! + 1;
    const to = boundaries[index]!;
    if (from < to) result.push(points.slice(from, to) as CurvePoint[]);
  }
  return result;
}


export function curveTension(smoothness: CurveSmoothness): number {
  return smoothness === 'smooth' ? 0.34 : 0.18;
}

export function clampedControlPoints(
  points: CurvePoint[],
  index: number,
  smoothness: CurveSmoothness,
): [CurvePoint, CurvePoint] {
  const first = points[index]!;
  const second = points[index + 1]!;
  const before = points[Math.max(0, index - 1)]!;
  const after = points[Math.min(points.length - 1, index + 2)]!;
  const tension = curveTension(smoothness);
  const minY = Math.min(first.y, second.y);
  const maxY = Math.max(first.y, second.y);
  return [
    {
      x: first.x + (second.x - before.x) * tension,
      y: Math.min(maxY, Math.max(minY, first.y + (second.y - before.y) * tension)),
    },
    {
      x: second.x - (after.x - first.x) * tension,
      y: Math.min(maxY, Math.max(minY, second.y - (after.y - first.y) * tension)),
    },
  ];
}

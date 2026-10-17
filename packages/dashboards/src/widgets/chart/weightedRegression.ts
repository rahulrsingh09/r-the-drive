type WeightedPoint = { x: number; y: number; weight: number };
type LineFit =
  | { kind: 'empty' }
  | { kind: 'constant'; value: number }
  | { kind: 'line'; intercept: number; slope: number };

function fitWeightedLine(points: WeightedPoint[]): LineFit {
  // Retained weighted-moment primitive: source order and left-associated
  // products are required for exact floating-point compatibility.
  let weight = 0;
  let x = 0;
  let y = 0;
  let xx = 0;
  let xy = 0;
  for (const point of points) {
    weight += point.weight;
    x += point.weight * point.x;
    y += point.weight * point.y;
    xx += point.weight * point.x * point.x;
    xy += point.weight * point.x * point.y;
  }
  if (weight === 0) return { kind: 'empty' };
  const determinant = weight * xx - x ** 2;
  if (Math.abs(determinant) < Number.EPSILON) return { kind: 'constant', value: y / weight };
  const slope = (weight * xy - x * y) / determinant;
  return { kind: 'line', intercept: (y - slope * x) / weight, slope };
}

export function predictWeightedLine(points: WeightedPoint[], atX: number) {
  const fit = fitWeightedLine(points);
  switch (fit.kind) {
    case 'empty': return null;
    case 'constant': return Math.max(0, fit.value);
    case 'line': return Math.max(0, fit.intercept + fit.slope * atX);
  }
}

type WeightedPoint = { x: number; y: number; weight: number };
type LineFit =
  | { kind: 'empty' }
  | { kind: 'constant'; value: number }
  | { kind: 'line'; intercept: number; slope: number };

function fitWeightedLine(points: WeightedPoint[]): LineFit {
  const columns: [number[], number[], number[], number[], number[]] = [[], [], [], [], []];
  for (const { x, y, weight } of points) {
    const weightedX = weight * x;
    columns[0].push(weight);
    columns[1].push(weightedX);
    columns[2].push(weight * y);
    columns[3].push(weightedX * x);
    columns[4].push(weightedX * y);
  }
  // Each moment retains source order and left-associated weighted products.
  const totals = columns.map((column) => column.reduce((sum, value) => sum + value, 0));
  const [weight, x, y, xx, xy] = totals as [number, number, number, number, number];
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

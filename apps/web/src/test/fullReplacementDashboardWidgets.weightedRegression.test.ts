import { describe, expect, it } from 'vitest';
import { predictWeightedLine } from '../../../../packages/dashboards/src/widgets/chart/weightedRegression';

describe('weighted charging regression', () => {
  it('has no prediction for empty or zero-total-weight samples', () => {
    expect(predictWeightedLine([], 10)).toBeNull();
    expect(predictWeightedLine([{ x: 1, y: 10, weight: 0 }], 10)).toBeNull();
    expect(predictWeightedLine([{ x: 1, y: 10, weight: 1 }, { x: 2, y: 20, weight: -1 }], 10)).toBeNull();
  });

  it('uses the weighted mean for coincident x even at a nonfinite evaluation point', () => {
    const points = [{ x: 3, y: 10, weight: 1 }, { x: 3, y: 30, weight: 3 }];
    expect(predictWeightedLine(points, NaN)).toBe(25);
    expect(predictWeightedLine(points, Infinity)).toBe(25);
  });

  it('solves a sloped line and clamps extrapolated negative power', () => {
    const points = [{ x: 1, y: 10, weight: 1 }, { x: 2, y: 20, weight: 2 }, { x: 3, y: 30, weight: 1 }];
    expect(predictWeightedLine(points, 4)).toBe(40);
    expect(predictWeightedLine(points, -4)).toBe(0);
  });

  it('retains the strict EPSILON determinant threshold', () => {
    const sample = (weight: number) => [{ x: 0, y: 0, weight: 1 }, { x: 1, y: 1, weight }];
    expect(predictWeightedLine(sample(Number.EPSILON / 2), 1)).toBe(Number.EPSILON / 2);
    expect(predictWeightedLine(sample(Number.EPSILON), 1)).toBe(1);
  });

  it('preserves source accumulation order without sorting or mutating samples', () => {
    const points = Object.freeze([1e16, 1, -1e16].map((y) => Object.freeze({ x: 0, y, weight: 1 })));
    expect(predictWeightedLine([...points], 0)).toBe(0);
    expect(predictWeightedLine([points[0]!, points[2]!, points[1]!], 0)).toBe(1 / 3);
  });

  it('propagates nonfinite moments and returns positive zero for a negative constant', () => {
    expect(predictWeightedLine([{ x: NaN, y: 1, weight: 1 }], 0)).toBeNaN();
    expect(predictWeightedLine([{ x: 1, y: -1, weight: 1 }], 0)).toBe(0);
    expect(predictWeightedLine([{ x: 1, y: -0, weight: 1 }], 0)).toBe(0);
  });
});

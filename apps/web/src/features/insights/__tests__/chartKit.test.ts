import { describe, expect, it } from 'vitest';
import { fitAxisLabels } from '../chartKit';

const weekly = (count: number, width: number) => Array.from({ length: count }, (_, i) => ({
  x: (i / (count - 1)) * width, text: `Oct ${10 + i}`, anchor: i === 0 ? 'start' as const : i === count - 1 ? 'end' as const : 'middle' as const,
}));

describe('fitAxisLabels', () => {
  it('keeps every label when they fit', () => {
    expect(fitAxisLabels(weekly(4, 600), 600)).toHaveLength(4);
  });

  it('thins crowded labels, keeps the newest and leaves space between them', () => {
    const labels = weekly(13, 327);
    const kept = fitAxisLabels(labels, 327);
    expect(kept.length).toBeGreaterThanOrEqual(2);
    expect(kept.length).toBeLessThan(13);
    expect(kept.at(-1)).toBe(labels.at(-1));
    const right = (label: typeof kept[number]) => label.anchor === 'end' ? label.x : label.x + label.text.length * 6.8 / 2;
    const left = (label: typeof kept[number]) => label.anchor === 'start' ? label.x : label.x - label.text.length * 6.8 / 2;
    for (let i = 1; i < kept.length; i++) expect(left(kept[i]!) - right(kept[i - 1]!)).toBeGreaterThanOrEqual(14);
  });

  it('leaves out a label that crosses the chart edge', () => {
    expect(fitAxisLabels([{ x: 4, text: 'Oct 10', anchor: 'middle' }, { x: 300, text: 'Nov 2', anchor: 'end' }], 300))
      .toEqual([{ x: 300, text: 'Nov 2', anchor: 'end' }]);
  });
});

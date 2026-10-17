import { describe, expect, it } from 'vitest';
import { BUNDLED_CHART_DEFINITIONS } from '../../../../packages/dashboards/src/charts/defaults';
import { MAX_CHART_EXPORT_BYTES, parseChartJson } from '../../../../packages/dashboards/src/charts/portable';

describe('portable chart byte limit', () => {
  it.each(['a', 'é', '中', '😀', '\ud800'])('counts UTF-8 bytes at the exact limit for %j', (character) => {
    const json = JSON.stringify({
      kind: 'panda-chart', version: 1,
      chart: { slug: 'synthetic', name: 'SENTINEL', enabled: true, definition: BUNDLED_CHART_DEFINITIONS[0] },
    }).replace('SENTINEL', character);
    const exact = json + ' '.repeat(MAX_CHART_EXPORT_BYTES - new TextEncoder().encode(json).byteLength);
    expect(parseChartJson(exact).chart.name).toBe(character);
    expect(() => parseChartJson(exact + ' ')).toThrow('Chart export exceeds the 256 KiB portable file limit');
  });

  it('rejects an oversized prefix before attempting JSON parsing', () => {
    expect(() => parseChartJson('😀'.repeat(MAX_CHART_EXPORT_BYTES / 4 + 1))).toThrow(
      'Chart export exceeds the 256 KiB portable file limit',
    );
  });
});

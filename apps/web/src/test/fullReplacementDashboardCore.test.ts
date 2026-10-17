import { describe, expect, it } from 'vitest';
import type { ChartRecord, ChartDefinitionV1 } from '@panda/types';
import { BUNDLED_CHART_DEFINITIONS, ChartDefinitionV1Schema, buildChartManagerEntries, resolveAssignedCharts, validateBundledRendererDefinition } from '@panda/dashboards';
import { defaultChartPermissions, withChartPlacement } from '../../../../packages/dashboards/src/charts/model';
import { removeCurveAndUnusedSources } from '../../../../packages/dashboards/src/charts/curveCatalog';
import { seriesToDailyDeltas, seriesToDailyTotals } from '../../../../packages/dashboards/src/editor/dailyDelta';

const config = () => ChartDefinitionV1Schema.parse(BUNDLED_CHART_DEFINITIONS[0]) as ChartDefinitionV1;
const record = (overrides: Partial<ChartRecord> = {}): ChartRecord => ({ id: 'system', ownerId: null, slug: 'test', name: 'Test', isDefault: true, isLocked: false, isEnabled: true, config: config(), ...overrides });

describe('dashboard core replacement characterization', () => {
  it.each([false, true])('preserves system and personal permission combinations for admin=%s', (admin) => {
    for (const locked of [false, true]) {
      expect(defaultChartPermissions(record({ isLocked: locked }), admin)).toEqual({ read: true, edit: admin && !locked, duplicate: true, reset: false, restore: admin, delete: false, lock: admin });
      expect(defaultChartPermissions(record({ ownerId: 'self', isLocked: locked }), admin)).toEqual({ read: true, edit: !locked, duplicate: true, reset: false, restore: false, delete: true, lock: false });
    }
  });
  it('keeps last duplicates, excludes other owners, and honors a disabled override', () => {
    const system = record();
    const override = record({ id: 'mine', ownerId: 'self', isEnabled: false });
    const entries = buildChartManagerEntries([system, record({ id: 'other', ownerId: 'other' }), override, { ...system, id: 'last' }], 'self');
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({ effective: override, systemBase: { id: 'last' }, origin: 'override', permissions: { reset: true } });
    expect(resolveAssignedCharts(entries, 'overview', ['test'])).toEqual([]);
    expect(buildChartManagerEntries([system], null)[0]?.personalOverride).toBe(system);
  });
  it('replaces duplicate placements without mutating the record', () => {
    const source = record(); source.config.placements = [{ dashboardSlug: 'z' }, { dashboardSlug: 'a' }, { dashboardSlug: 'z' }];
    const result = withChartPlacement(source, 'z', true);
    expect(result.config.placements).toEqual([{ dashboardSlug: 'a' }, { dashboardSlug: 'z' }]);
    expect(source.config.placements).toHaveLength(3);
  });
  it('uses lexical timestamps to select daily counters and floors resets', () => {
    const data = [{ ts: '2026-07-03T10:00:00Z', value: 4 }, { ts: '2026-07-01T08:00:00Z', value: 10 }, { ts: '2026-07-01T09:00:00Z', value: 12 }, { ts: 'invalid', value: 100 }, { ts: '2026-07-04T00:00:00Z', value: NaN }];
    const original = structuredClone(data);
    expect(seriesToDailyDeltas(data, 4)).toEqual([{ ts: '2026-06-30T00:00:00.000Z', value: 0 }, { ts: '2026-07-01T09:00:00Z', value: 0 }, { ts: '2026-07-02T00:00:00.000Z', value: 0 }, { ts: '2026-07-03T10:00:00Z', value: 0 }]);
    expect(seriesToDailyTotals(data, 0)).toEqual([{ ts: '2026-07-01T00:00:00.000Z', value: 22 }, { ts: '2026-07-03T00:00:00.000Z', value: 4 }]);
    expect(data).toEqual(original);
  });
  it('reports nested prohibited keys in traversal order', () => {
    const definition = config(); definition.sources[0]!.params = { nested: [{ remote_url: { script: 'blocked' } }], css: 'blocked' };
    const result = ChartDefinitionV1Schema.safeParse(definition);
    expect(result.success).toBe(false);
    if (!result.success) expect(result.error.issues.map(issue => issue.path.join('.'))).toEqual(['config.sources.0.params.nested.0.remote_url', 'config.sources.0.params.nested.0.remote_url.script', 'config.sources.0.params.css']);
  });
  it('rejects cyclic parameter graphs without rejecting shared acyclic objects', () => {
    const definition = config();
    const shared = { sample: 1 };
    definition.sources[0]!.params = { a: shared, b: shared };
    expect(ChartDefinitionV1Schema.safeParse(definition).success).toBe(true);
    const cyclic: Record<string, unknown> = {}; cyclic.self = cyclic;
    definition.sources[0]!.params = cyclic;
    expect(() => ChartDefinitionV1Schema.parse(definition)).toThrow(RangeError);
  });
  it('retains every bundled production renderer contract', () => {
    for (const bundled of BUNDLED_CHART_DEFINITIONS) {
      const definition = ChartDefinitionV1Schema.parse(bundled) as ChartDefinitionV1;
      expect(validateBundledRendererDefinition(bundled.slug, definition)).toEqual([]);
      definition.rendererId = 'custom-renderer';
      expect(validateBundledRendererDefinition(bundled.slug, definition)).toContainEqual({ path: 'config.rendererId', message: 'Bundled renderer selection cannot be changed.' });
    }
  });
  it('rebases the shared X binding after removing its last implicit curve', () => {
    const definition = config();
    definition.sources = [{ ...definition.sources[0]!, id: 'old' }, { ...definition.sources[0]!, id: 'next' }];
    definition.x.field = { sourceBindingId: 'old', field: 'timestamp' };
    definition.series = [{ ...definition.series[0]!, y: { sourceBindingId: 'old', field: 'value' } }, { ...definition.series[0]!, id: 'next', x: { sourceBindingId: 'next', field: 'distance' }, y: { sourceBindingId: 'next', field: 'value' } }];
    const result = removeCurveAndUnusedSources(definition, 0);
    expect(result.x.field).toEqual({ sourceBindingId: 'next', field: 'distance' });
    expect(result.sources.map(source => source.id)).toEqual(['next']);
    expect(removeCurveAndUnusedSources(result, 0).sources).toEqual([]);
    expect(definition.sources).toHaveLength(2);
  });
});

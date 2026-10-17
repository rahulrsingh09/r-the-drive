import type {
  ChartColorDefinition,
  ChartDefinitionV1,
  ChartRecord,
} from '@panda/types';
import { getBundledChartDefinition } from './defaults';
import { ChartDefinitionV1Schema } from './schema';

interface LegacyMileageBaseline {
  primaryField: string;
  definition: ChartDefinitionV1;
}

const LEGACY_MILEAGE_BASELINES: Record<string, LegacyMileageBaseline> = {
  'battery-capacity-mileage': {
    primaryField: 'usable_kwh',
    definition: legacyMileageDefinition({
      primaryField: 'usable_kwh',
      primaryLabel: 'Usable Capacity',
      primaryUnit: 'kWh',
      primaryColor: { mode: 'token', token: 'accent' },
      showPoints: true,
    }),
  },
  'projected-range-mileage': {
    primaryField: 'projected_max_range_mi',
    definition: legacyMileageDefinition({
      primaryField: 'projected_max_range_mi',
      primaryLabel: 'Projected Range',
      primaryUnit: 'mi',
      primaryColor: { mode: 'token', token: 'emerald' },
    }),
  },
};

/**
 * Repairs the two exact incomplete mileage-chart definitions shipped by the
 * first persisted-chart baseline. Placement, timeframe, and primary color are
 * deliberately ignored by the fingerprint because those values could differ
 * without correcting the broken renderer geometry. Every other serialized
 * field must still match the obsolete baseline.
 */
export function normalizeLegacyBundledChartRecord(record: ChartRecord): ChartRecord {
  const baseline = LEGACY_MILEAGE_BASELINES[record.slug];
  if (!baseline || !matchesLegacyMileageBaseline(record.config, baseline.definition)) return record;
  const bundled = getBundledChartDefinition(record.slug);
  if (!bundled) return record;
  const config = ChartDefinitionV1Schema.parse(bundled) as unknown as ChartDefinitionV1;
  config.placements = structuredClone(record.config.placements);
  const primary = record.config.series[0];
  if (primary) {
    for (let index = 0; index < config.series.length; index++) {
      const series = config.series[index]!;
      if (series.y.field !== baseline.primaryField) continue;
      config.series[index] = { ...series, color: structuredClone(primary.color) };
    }
  }
  return { ...record, config };
}

function matchesLegacyMileageBaseline(definition: ChartDefinitionV1, baseline: ChartDefinitionV1) {
  const candidate = structuredClone(definition);
  candidate.placements = structuredClone(baseline.placements);
  candidate.timeframe = structuredClone(baseline.timeframe);
  if (candidate.series[0] && baseline.series[0]) candidate.series[0].color = structuredClone(baseline.series[0].color);
  const pending: Array<[unknown, unknown]> = [[candidate, baseline]];
  while (pending.length) {
    const [left, right] = pending.pop()!;
    if (Object.is(left, right)) continue;
    if (Array.isArray(left) || Array.isArray(right)) {
      if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
      for (let index = 0; index < left.length; index++) if (index in left) pending.push([left[index], right[index]]);
      continue;
    }
    if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
    const l = left as Record<string, unknown>;
    const r = right as Record<string, unknown>;
    const keys = Object.keys(l);
    if (keys.length !== Object.keys(r).length) return false;
    for (const key of keys) {
      if (!Object.hasOwn(r, key)) return false;
      pending.push([l[key], r[key]]);
    }
  }
  return true;
}

function legacyMileageDefinition({
  primaryField,
  primaryLabel,
  primaryUnit,
  primaryColor,
  showPoints,
}: {
  primaryField: string;
  primaryLabel: string;
  primaryUnit: string;
  primaryColor: ChartColorDefinition;
  showPoints?: boolean;
}): ChartDefinitionV1 {
  return {
    schemaVersion: 1,
    placements: [{ dashboardSlug: 'overview' }, { dashboardSlug: 'battery' }],
    timeframe: { mode: 'dashboard' },
    sources: [
      {
        id: 'main',
        sourceId: 'battery.mileage',
        params: {},
        filters: [],
        inherit: { vehicle: true, timeframe: true },
      },
    ],
    x: { field: { sourceBindingId: 'main', field: 'odometer_miles' }, kind: 'number' },
    series: [
      {
        id: primaryField,
        label: primaryLabel,
        y: { sourceBindingId: 'main', field: primaryField },
        mark: 'line',
        yAxis: 'y',
        color: primaryColor,
        transforms: [],
        visibleInLegend: true,
      },
    ],
    axes: {
      x: { scale: 'linear', domain: { mode: 'auto' } },
      y: { scale: 'linear', unit: primaryUnit, domain: { mode: 'auto' } },
    },
    display: {
      legend: 'hide',
      grid: true,
      tooltip: true,
      timeFilter: 'raw',
      curveSmoothness: 'gentle',
      ...(showPoints ? { showPoints: true } : {}),
    },
    interaction: { panZoom: true, touchExplore: true, connectGaps: false },
  };
}

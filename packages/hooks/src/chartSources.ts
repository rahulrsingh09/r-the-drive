import { useQueries } from '@tanstack/react-query';
import type { ChartDataset, ChartDatasetField, ChartDefinitionV1, ChartSourceBinding } from '@panda/types';
import { api } from './api';
import { useAuthReady } from './useAuthState';
import type { TripTagFilters } from './useTrips';

export interface ChartRuntimeContext {
  vehicleId: string | null;
  from: string | null;
  to: string | null;
  lifetime?: boolean;
  chargeSessionId?: string | null;
  tripTagFilter?: TripTagFilters;
}

export interface ChartSourceQueryArgs {
  binding: ChartSourceBinding;
  definition: ChartDefinitionV1;
  context: ChartRuntimeContext;
}

export interface ChartSourceAdapter {
  id: string;
  fetch: (args: ChartSourceQueryArgs) => Promise<unknown>;
}

type SourceMethod = 'getMetricSeries' | 'getBatteryMileage' | 'getDegradation' | 'getPhantomDrain' | 'getChargingChartSeries' | 'getChargeCurve' | 'getChargeCurveAnalysis' | 'getEfficiencyTrend' | 'getEfficiencyVsTemp' | 'getEfficiencyByMode' | 'getEfficiencyByTag' | 'getTirePressureTimeline';
const sourceContracts: Array<[string, SourceMethod, 'metric' | 'range' | 'tagged' | 'curve']> = [
  ['metrics.series', 'getMetricSeries', 'metric'], ['battery.mileage', 'getBatteryMileage', 'range'], ['battery.degradation', 'getDegradation', 'range'],
  ['battery.idle-drain', 'getPhantomDrain', 'range'], ['charging.sessions', 'getChargingChartSeries', 'range'], ['charging.charge-curve', 'getChargeCurve', 'curve'],
  ['charging.curve-analysis', 'getChargeCurveAnalysis', 'range'], ['efficiency.trend', 'getEfficiencyTrend', 'tagged'], ['efficiency.temperature', 'getEfficiencyVsTemp', 'tagged'],
  ['efficiency.drive-mode', 'getEfficiencyByMode', 'tagged'], ['efficiency.trip-tags', 'getEfficiencyByTag', 'tagged'], ['trips.tire-pressure-timeline', 'getTirePressureTimeline', 'tagged'],
];

export const CHART_SOURCE_ADAPTERS: Readonly<Record<string, ChartSourceAdapter>> = Object.fromEntries(sourceContracts.map(([id, method, kind]) => [id, {
  id,
  async fetch({ binding, definition, context }: ChartSourceQueryArgs) {
    if (!context.vehicleId) return method === 'getTirePressureTimeline' ? { samples: [], trips: [] } : [];
    const timeframe: unknown[] = [context.vehicleId, context.from, context.to];
    let parameters: unknown[];
    if (kind === 'metric') {
      const configured = (binding.params as Record<string, unknown>).metric;
      const metric = typeof configured === 'string' && configured ? configured : definition.series[0]?.y.field ?? null;
      if (!metric) return [];
      parameters = [context.vehicleId, metric, context.from, context.to, 'auto'];
    } else if (kind === 'curve') {
      if (!context.chargeSessionId) return [];
      parameters = [context.chargeSessionId, context.vehicleId];
    } else {
      parameters = [...timeframe, context.lifetime ?? (!context.from && !context.to)];
      if (kind === 'tagged') parameters.push(context.tripTagFilter);
    }
    const request = api[method] as (...args: unknown[]) => Promise<unknown>;
    return request.apply(api, parameters);
  },
}]));

const fieldAliases: Record<string, string[]> = {
  timestamp: ['ts', 'timestamp', 'started_at'], day_start: ['day_start', 'day', 'ts'], odometer_miles: ['odometer_miles', 'odometer_mi'],
  soc_lost_pct: ['soc_lost_pct', 'total_soc_lost'], parked_hours: ['parked_hours', 'hours_parked'], drain_rate_pct_per_hour: ['drain_rate_pct_per_hour', 'avg_drain_rate'],
  elapsed_minutes: ['elapsed_minutes', 'minutes_elapsed'], power_kw: ['power_kw', 'charge_rate_kw'], avg_efficiency_wh_mi: ['avg_efficiency_wh_mi', 'avg_efficiency'],
  front_left_psi: ['front_left_psi', 'tire_fl_psi'], front_right_psi: ['front_right_psi', 'tire_fr_psi'], rear_left_psi: ['rear_left_psi', 'tire_rl_psi'], rear_right_psi: ['rear_right_psi', 'tire_rr_psi'],
};

function sourceRows(raw: unknown): Array<Record<string, unknown>> {
  let value = raw;
  while (value && typeof value === 'object') {
    if (Array.isArray(value)) return value.filter((row): row is Record<string, unknown> => Boolean(row) && typeof row === 'object');
    const object = value as Record<string, unknown>;
    value = ['daily', 'samples', 'data', 'rows'].map((key) => object[key]).find(Array.isArray);
  }
  return [];
}

export function normalizeChartDataset(raw: unknown, binding: ChartSourceBinding, definition: ChartDefinitionV1): ChartDataset {
  const bound = definition.series.filter((series) => series.y.sourceBindingId === binding.id);
  const primaryX = definition.x.field;
  const xField = primaryX.sourceBindingId === binding.id ? primaryX.field : bound.find((series) => series.x?.sourceBindingId === binding.id)?.x?.field ?? primaryX.field;
  const orderedFields = new Set([xField]);
  bound.forEach((series) => orderedFields.add(series.y.field));
  definition.series.forEach((series) => { if (series.x?.sourceBindingId === binding.id) orderedFields.add(series.x.field); });
  const fields: Record<string, ChartDatasetField> = {};
  const columns = Array.from(orderedFields, (name) => {
    const isX = definition.series.some((series) => series.x?.field === name && series.x.sourceBindingId === binding.id);
    const isY = definition.series.some((series) => series.y.field === name);
    fields[name] = { kind: isX ? definition.x.kind : isY ? 'number' : definition.x.kind, values: [] };
    const aliases = [name, ...(fieldAliases[name] ?? []), 'value'];
    return { name, aliases };
  });
  const rows = sourceRows(raw);
  const values: Array<string | number> = [];
  for (const row of rows) {
    const cells = columns.map(({ name, aliases }) => {
      const key = aliases.find((key) => key in row);
      const value = key === undefined ? (binding.params as Record<string, unknown>).metric === name ? row.value : null : row[key];
      return typeof value === 'string' || typeof value === 'number' ? value : null;
    });
    const domain = cells[0];
    if (domain == null) continue;
    values.push(domain);
    columns.forEach(({ name }, index) => fields[name]!.values.push(cells[index]!));
  }
  return { sourceBindingId: binding.id, domain: { kind: definition.x.kind, field: xField, values }, fields, meta: { sourceId: binding.sourceId, sampled: false, partial: false, sourcePointCount: rows.length } };
}

export function useChartDatasets(definition: ChartDefinitionV1 | null, context: ChartRuntimeContext) {
  const authReady = useAuthReady();
  const bindings = definition?.sources ?? [];
  const queries = bindings.map((binding) => ({
    queryKey: ['chart-dataset', binding.sourceId, binding.id, binding.params, binding.filters, context],
    enabled: Boolean(authReady && context.vehicleId && definition),
    staleTime: 120_000,
    meta: { persist: false },
    queryFn: async () => {
      const source = CHART_SOURCE_ADAPTERS[binding.sourceId];
      if (source) return source.fetch({ binding, definition: definition!, context });
      throw new Error(`Chart source '${binding.sourceId}' is unavailable`);
    },
  }));
  const results = useQueries({ queries });
  const summary = { datasets: [] as ChartDataset[], isLoading: false, isFetching: false, isPartial: false, errors: [] as Error[] };
  for (let index = 0; index < results.length; index++) {
    const result = results[index]!;
    summary.isLoading ||= result.isLoading;
    summary.isFetching ||= result.isFetching;
    summary.isPartial ||= result.isError;
    if (result.error) summary.errors.push(result.error);
    if (definition && result.data !== undefined) summary.datasets.push(normalizeChartDataset(result.data, bindings[index]!, definition));
  }
  return { ...summary, results };
}

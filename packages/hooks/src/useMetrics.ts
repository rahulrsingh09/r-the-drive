import { canonicalTagFilter, useApiQuery, queryPolicies } from './queryRuntime';
import type { MetricBatchMetricRequest } from '@panda/types';
import type { TripTagFilters } from './useTrips';

export type MetricSeriesBucket = 'auto' | 'raw' | 'minute' | '5min' | '15min' | 'hour' | 'day';

export function chooseMetricSeriesBucket(from: string | null, to: string | null): MetricSeriesBucket {
  const duration = from && to ? Date.parse(to) - Date.parse(from) : NaN;
  const thresholds: Array<[number, MetricSeriesBucket]> = [[60, 'minute'], [360, '5min'], [1440, '15min'], [10080, 'hour']];
  if (!(duration > 0)) return 'day';
  return thresholds.find(([minutes]) => duration <= minutes * 60_000)?.[1] ?? 'day';
}

export function useMetricCatalog() {
  return useApiQuery(['metrics', 'catalog'], 'getMetricCatalog', [], [], {
    ...queryPolicies.history,
    staleTime: 60 * 60 * 1000
  });
}

export function useMetricValue(
  vehicleId: string | null,
  metric: string | null,
  from: string | null = null,
  to: string | null = null,
) {
  const lifetime = !from && !to;
  return useApiQuery(['metrics', 'value', vehicleId, metric, from, to, lifetime], 'getMetricValue', [vehicleId!, metric!, from, to, lifetime], [vehicleId, metric], {
    ...queryPolicies.history,
    staleTime: 2 * 60 * 1000
  });
}

export function useMetricSeries(
  vehicleId: string | null,
  metric: string | null,
  from: string | null,
  to: string | null,
  bucket: MetricSeriesBucket = 'auto',
) {
  const lifetime = !from && !to;
  const resolvedBucket = bucket === 'auto' ? chooseMetricSeriesBucket(from, to) : bucket;
  return useApiQuery(['metrics', 'series', vehicleId, metric, from, to, lifetime, resolvedBucket], 'getMetricSeries', [vehicleId!, metric!, from, to, resolvedBucket, lifetime], [vehicleId, metric], {
    ...queryPolicies.history,
    staleTime: 2 * 60 * 1000
  });
}

/**
 * Fetches the compact metric payload used by dashboard chips in one request.
 * It intentionally does not persist: the page-level provider already keeps
 * it warm in memory and a multi-dashboard localStorage cache is counterproductive.
 */
export function useMetricBatch(
  vehicleId: string | null,
  metrics: readonly MetricBatchMetricRequest[],
  from: string | null,
  to: string | null,
  lifetime = false,
  filters?: TripTagFilters,
) {
  const stableMetrics: Array<{ metric: string; include_latest: boolean; include_series: boolean }> = [];
  const keyParts: string[] = [];
  for (const item of metrics) {
    const flags = { include_latest: item.include_latest !== false, include_series: item.include_series !== false };
    stableMetrics.push({ metric: item.metric, ...flags });
    keyParts.push([item.metric, +flags.include_latest, +flags.include_series].join(':'));
  }
  const metricKey = keyParts.join('|');
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['metrics', 'batch', vehicleId, from, to, lifetime, metricKey, tagFilters], 'getMetricBatch', [{
      vehicle_id: vehicleId!,
      metrics: stableMetrics,
      from,
      to,
      lifetime,
      density: 'full',
      bucket: 'raw',
      ...(tagFilters.tagIds.length ? { tag_ids: tagFilters.tagIds.join(','), tag_match: tagFilters.tagMatch } : {}),
      ...(tagFilters.untagged ? { untagged: true } : {}),
    }], [vehicleId, stableMetrics.length > 0], {
    ...queryPolicies.history,
    staleTime: 2 * 60 * 1000,
    meta: { persist: false }
  });
}

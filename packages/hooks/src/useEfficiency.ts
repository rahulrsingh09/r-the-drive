import { useApiQuery, queryPolicies } from './queryRuntime';
import { canonicalTagFilter } from './queryRuntime';
import type { TripTagFilters } from './useTrips';


export function useEfficiencySummary(vehicleId: string | null, from: string | null, to: string | null, filters?: TripTagFilters) {
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['efficiency', 'summary', vehicleId, from, to, lifetime, tagFilters], 'getEfficiencySummary', [vehicleId!, from, to, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000,
    refetchOnMount: 'always'
  });
}

export function useEfficiencyByMode(vehicleId: string | null, from: string | null, to: string | null, filters?: TripTagFilters) {
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['efficiency', 'by-mode', vehicleId, from, to, lifetime, tagFilters], 'getEfficiencyByMode', [vehicleId!, from, to, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000,
    refetchOnMount: 'always'
  });
}

export function useEfficiencyTrend(vehicleId: string | null, from: string | null, to: string | null, filters?: TripTagFilters) {
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['efficiency', 'trend', vehicleId, from, to, lifetime, tagFilters], 'getEfficiencyTrend', [vehicleId!, from, to, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000,
    refetchOnMount: 'always'
  });
}

export function useEfficiencyVsTemp(vehicleId: string | null, from: string | null, to: string | null, filters?: TripTagFilters) {
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['efficiency', 'vs-temp', vehicleId, from, to, lifetime, tagFilters], 'getEfficiencyVsTemp', [vehicleId!, from, to, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 10 * 60 * 1000,
    refetchOnMount: 'always'
  });
}

export function useEfficiencyByTag(vehicleId: string | null, from: string | null, to: string | null, filters?: TripTagFilters) {
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['efficiency', 'by-tag', vehicleId, from, to, lifetime, tagFilters], 'getEfficiencyByTag', [vehicleId!, from, to, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000,
    refetchOnMount: 'always'
  });
}

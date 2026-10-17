import { useApiQuery, queryPolicies } from './queryRuntime';

export function useSummaryStats(vehicleId: string | null) {
  return useApiQuery(['stats', vehicleId], 'getStats', [vehicleId!], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 10 * 60 * 1000
  });
}

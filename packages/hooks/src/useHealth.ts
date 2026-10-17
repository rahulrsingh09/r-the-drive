import { useApiQuery, queryPolicies } from './queryRuntime';
import { queryKeys } from './queryKeys';

export function useVehicleHealth(vehicleId: string | null) {
  return useApiQuery(queryKeys.vehicle.health(vehicleId), 'getVehicleHealth', [vehicleId!], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 60 * 1000,
    refetchOnMount: 'always'
  });
}

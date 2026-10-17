import { useApiQuery, queryPolicies } from './queryRuntime';
import { queryKeys } from './queryKeys';

export function useVehicles() {
  return useApiQuery(queryKeys.vehicles.all, 'listVehicles', [], [], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000
  });
}

export function useDefaultVehicleId(): string | null {
  try {
    const persisted = JSON.parse(localStorage.getItem('rm-auth') ?? 'null') as { state?: { defaultVehicleId?: string | null } } | null;
    return persisted?.state?.defaultVehicleId ?? null;
  } catch { return null; }
}

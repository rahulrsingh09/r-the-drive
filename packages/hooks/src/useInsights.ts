import { useQuery } from '@tanstack/react-query';
import type { InsightScope, InsightsResponse } from '@panda/types';
import { transport } from './api/transport';
import { useAuthReady } from './useAuthState';

export const insightsQueryKey = (vehicleId: string | null, scope: InsightScope) =>
  ['vehicles', 'insights', vehicleId, scope] as const;

export async function fetchInsights(vehicleId: string, scope: InsightScope): Promise<InsightsResponse> {
  const response = await transport.requestResponse(
    'GET',
    `/v1/vehicles/${encodeURIComponent(vehicleId)}/insights`,
    undefined,
    { scope },
  );
  return (await response.json()) as InsightsResponse;
}

/**
 * Rule-based insights for one vehicle. `scope` is `now`, `trip:{id}` or
 * `charge:{id}`. Pass `null` to wait for a trip or charge id.
 */
export function useInsights(vehicleId: string | null, scope: InsightScope | null) {
  const authReady = useAuthReady();
  return useQuery({
    queryKey: insightsQueryKey(vehicleId, scope ?? 'now'),
    queryFn: () => fetchInsights(vehicleId!, scope!),
    enabled: authReady && !!vehicleId && !!scope,
    staleTime: 60 * 1000,
    placeholderData: (previous) => previous,
  });
}

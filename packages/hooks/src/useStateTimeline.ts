import { useQuery } from '@tanstack/react-query';
import type { StateTimelineResponse } from '@panda/types';
import { api } from './api';
import { queryKeys } from './queryKeys';
import { useAuthReady } from './useAuthState';
import { queryPolicies } from './queryRuntime';

/** Fetches vehicle state timeline periods (sleep, idle, awake, standby) over [from, to]. */
export function useStateTimeline(
  vehicleId: string | null,
  from: string | null,
  to: string | null,
  limit = 500,
) {
  const authReady = useAuthReady();
  return useQuery<StateTimelineResponse>({
    queryKey: queryKeys.vehicles.stateTimeline(vehicleId, from, to, limit),
    queryFn: () => {
      const qs = new URLSearchParams();
      if (from) qs.set('from', from);
      if (to) qs.set('to', to);
      qs.set('limit', String(limit));
      return api.apiFetch<StateTimelineResponse>('GET', `/v1/vehicles/${vehicleId}/state-timeline?${qs.toString()}`);
    },
    enabled: authReady && Boolean(vehicleId),
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000,
  });
}

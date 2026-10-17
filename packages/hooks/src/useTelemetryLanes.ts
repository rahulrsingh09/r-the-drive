import { useApiQuery, queryPolicies } from './queryRuntime';
import type { TelemetryLaneQuery } from '@panda/types';

export function useTelemetryLanes(
  vehicleId: string | null,
  query: TelemetryLaneQuery = {},
) {
  return useApiQuery(['vehicles', 'telemetry-lanes', vehicleId, query], 'getTelemetryLanes', [vehicleId!, query], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 2 * 60 * 1000
  });
}

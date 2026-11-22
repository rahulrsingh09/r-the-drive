import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from './api';
import { useAuthReady } from './useAuthState';
import { invalidateChargingData } from './useCharging';

export const chargeBackfillQueryKey = (vehicleId: string | null) => ['vehicles', 'backfill-status', vehicleId] as const;

/** Charging-history backfill state. Polls every 3 s while a backfill runs. */
export function useChargeBackfillStatus(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery({
    queryKey: chargeBackfillQueryKey(vehicleId),
    queryFn: () => api.getBackfillStatus(vehicleId!),
    enabled: authReady && !!vehicleId,
    retry: false,
    refetchInterval: (query) => (query.state.data?.status === 'running' ? 3_000 : false),
  });
}

/** Starts the charging-history backfill ("Recover missed charges"). */
export function useTriggerChargeBackfill(vehicleId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => api.triggerBackfill(vehicleId!),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: chargeBackfillQueryKey(vehicleId) });
      invalidateChargingData(queryClient);
    },
  });
}

/** Signs out every other device of the signed-in account. */
export function useRevokeOtherAuthSessions() {
  return useMutation({
    mutationFn: () => api.revokeOtherAuthSessions(),
  });
}

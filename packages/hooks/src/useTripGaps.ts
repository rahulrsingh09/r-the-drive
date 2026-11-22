import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api } from './api';
import { useAuthReady } from './useAuthState';
import { queryKeys } from './queryKeys';

/** Drives with telemetry but no trip, plus whether Parallax went quiet while moving. */
export function useTripGaps(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery({
    queryKey: queryKeys.vehicle.tripGaps(vehicleId),
    queryFn: () => api.getTripGaps(vehicleId!),
    enabled: authReady && !!vehicleId,
    // Listing replays a week of telemetry on the server.
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
    retry: false,
  });
}

/** Saves a listed gap as a trip. Owners and managers only. */
export function useRecoverTripGap(vehicleId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (gapId: string) => api.recoverTripGap(vehicleId!, gapId),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicle.tripGaps(vehicleId) }),
        queryClient.invalidateQueries({ queryKey: ['trips'] }),
      ]);
    },
  });
}

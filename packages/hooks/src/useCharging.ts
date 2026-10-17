import { useApiQuery, queryPolicies } from './queryRuntime';
import type { QueryClient } from '@tanstack/react-query';
import { cacheInvalidations, invalidateKeys, useCacheMutation } from './cacheRuntime';
import { api } from './api';
import type { DepartureScheduleInput, ChargingScheduleInput } from './api';
import type { ChargeSession, ChargeSessionUpdate, ChargingNetworkPreference } from '@panda/types';

type PlaceQueryKey = readonly ['places'];

type UpdateChargeSessionLocationPayload = {
  sessionId: string;
  placeId: string | null;
  placeName: string | null;
};

export type UpdateChargeSessionPayload = ChargeSessionUpdate & { sessionId: string };

type UpdateChargeSessionLocationResponse = {
  session?: {
    location_name?: string | null;
  };
};

const chargingInvalidationKeys = [['charging'], ['metrics', 'batch'], ['trips']] as const;

export function invalidateChargingData(queryClient: QueryClient): void {
  invalidateKeys(queryClient, chargingInvalidationKeys);
}

export function useChargeSessions(
  vehicleId: string | null,
  from: string | null,
  to: string | null,
  page = 1,
  perPage = 25,
  search = '',
  chargeSessionDayLocal: string | null = null,
) {
  const normalizedSearch = search.trim();
  const lifetime = !from && !to;
  return useApiQuery(['charging', 'list', vehicleId, from, to, lifetime, page, perPage, normalizedSearch, chargeSessionDayLocal], 'listChargeSessions', [vehicleId!, from, to, page, perPage, normalizedSearch, lifetime, chargeSessionDayLocal], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 60 * 1000,
    refetchInterval: (query) => activeChargePoll(query.state.data?.items ?? [])
  });
}

export function useChargeSession(sessionId: string | null, vehicleId: string | null) {
  return useApiQuery(['charging', 'detail', sessionId, vehicleId], 'getChargeSession', [sessionId!, vehicleId!], [sessionId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    refetchInterval: (query) => activeChargePoll(query.state.data ? [query.state.data] : [])
  });
}

export function useSavedPlaces() {
  return useApiQuery(['places'] as PlaceQueryKey, 'listPlaces', [], [], {
    ...queryPolicies.detail,
    staleTime: 60 * 60 * 1000,
    previous: false
  });
}

export function useUpdateChargeSessionLocation(vehicleId: string | null) {
  return useCacheMutation<UpdateChargeSessionLocationResponse, UpdateChargeSessionLocationPayload>(
    async ({ sessionId, placeId }) => {
      const vehicle = requireVehicle(vehicleId);
      return { session: await api.updateChargeSession(vehicle, sessionId, { place_id: placeId }) };
    },
    (result, { sessionId }) => [
      { key: ['charging', 'detail', sessionId, vehicleId], patch: { location_name: result.session?.location_name ?? null } },
      ...cacheInvalidations(chargingInvalidationKeys),
    ],
    ({ sessionId, placeName }) => ({ key: ['charging', 'detail', sessionId, vehicleId], patch: { location_name: placeName }, restoreMissing: true }),
  );
}

function requireVehicle(vehicleId: string | null): string {
  if (vehicleId) return vehicleId;
  throw new Error('vehicle_id is required');
}

export function useUpdateChargeSession(vehicleId: string | null) {
  return useCacheMutation<ChargeSession, UpdateChargeSessionPayload>(
    ({ sessionId, ...body }) => api.updateChargeSession(requireVehicle(vehicleId), sessionId, body),
    (session, variables) => [
      { key: ['charging', 'detail', variables.sessionId, vehicleId], value: session },
      ...cacheInvalidations(chargingInvalidationKeys),
    ],
  );
}

export function useChargingNetworkPreferences(vehicleId: string | null) {
  return useApiQuery(['charging', 'network-preferences', vehicleId], 'listChargingNetworkPreferences', [vehicleId!], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 60 * 1000,
    previous: false
  });
}

export function useUpdateChargingNetworkPreference(vehicleId: string | null) {
  return useCacheMutation(
    ({ networkVendor, costMode }: { networkVendor: string; costMode: ChargingNetworkPreference['cost_mode'] }) =>
      api.updateChargingNetworkPreference(requireVehicle(vehicleId), networkVendor, costMode),
    () => cacheInvalidations([['charging', 'network-preferences', vehicleId], ...chargingInvalidationKeys]),
  );
}

export function useChargeCurve(sessionId: string | null, vehicleId: string | null, active = false) {
  return useApiQuery(['charging', 'curve', sessionId, vehicleId], 'getChargeCurve', [sessionId!, vehicleId!], [sessionId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    refetchInterval: active ? 30 * 1000 : false
  });
}

export function useChargeCurveAnalysis(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['charging', 'curve-analysis', vehicleId, from, to, lifetime], 'getChargeCurveAnalysis', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000
  });
}

export function useChargingSummary(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['charging', 'summary', vehicleId, from, to, lifetime], 'getChargingSummary', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000
  });
}

export function useChargingChartSeries(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['charging', 'chart-series', vehicleId, from, to, lifetime], 'getChargingChartSeries', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000
  });
}

export function useChargingSchedule(vehicleId: string | null) {
  return useApiQuery(['schedules', 'charging', vehicleId], 'getChargingSchedule', [vehicleId!], [vehicleId], {
    staleTime: 5 * 60 * 1000
  });
}

export function useUpdateChargingSchedule(vehicleId: string | null) {
  return useCacheMutation((body: ChargingScheduleInput) => api.putChargingSchedule(vehicleId!, body), () => cacheInvalidations([['schedules', 'charging', vehicleId]]));
}

export function useDepartureSchedules(vehicleId: string | null) {
  return useApiQuery(['schedules', 'departure', vehicleId], 'listDepartureSchedules', [vehicleId!], [vehicleId], {
    staleTime: 5 * 60 * 1000
  });
}

export function useCreateDepartureSchedule(vehicleId: string | null) {
  return useCacheMutation((body: DepartureScheduleInput) => api.createDepartureSchedule(vehicleId!, body), () => cacheInvalidations([['schedules', 'departure', vehicleId]]));
}

export function useUpdateDepartureSchedule(vehicleId: string | null) {
  return useCacheMutation(({ scheduleId, body }: { scheduleId: string; body: DepartureScheduleInput }) => api.updateDepartureSchedule(vehicleId!, scheduleId, body), () => cacheInvalidations([['schedules', 'departure', vehicleId]]));
}

export function useDeleteDepartureSchedule(vehicleId: string | null) {
  return useCacheMutation((scheduleId: string) => api.deleteDepartureSchedule(vehicleId!, scheduleId), () => cacheInvalidations([['schedules', 'departure', vehicleId]]));
}

export function useLiveSession(vehicleId: string | null, active = true) {
  return useApiQuery(['live-session', vehicleId], 'getLiveSession', [vehicleId!], [vehicleId, active], {
    refetchInterval: active ? 30 * 1000 : false,
    staleTime: 0
  });
}

function activeChargePoll(sessions: readonly Pick<ChargeSession, 'ended_at'>[]): number | false {
  return sessions.findIndex((session) => session.ended_at == null) < 0 ? false : 30_000;
}

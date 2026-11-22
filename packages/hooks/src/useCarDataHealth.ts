import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { ChargeGapsResponse, Trip } from '@panda/types';
import { api, type BackfillStatus } from './api';
import { transport } from './api/transport';
import { useAuthReady } from './useAuthState';

/** Query keys for the Car tab data-health checks. */
export const carDataHealthKeys = {
  chargeGaps: (vehicleId: string | null) => ['vehicles', 'charge-gaps', vehicleId] as const,
  backfillStatus: (vehicleId: string | null) => ['vehicles', 'backfill-status', vehicleId] as const,
  lifetimeTrips: (vehicleId: string | null) => ['trips', 'lifetime-odometer', vehicleId] as const,
};

/**
 * Possible missed charges: the battery rose while parked and no charge covers it.
 * The Car tab reports its own failure ("Not checked"), so there is no global error toast.
 */
export async function fetchChargeGaps(vehicleId: string): Promise<ChargeGapsResponse> {
  const response = await transport.requestResponse(
    'GET', `/v1/vehicles/${encodeURIComponent(vehicleId)}/charge-gaps`, undefined, undefined, true, false,
  );
  return normalizeChargeGaps(vehicleId, await response.json());
}

const CHARGE_GAP_LOOKBACK_DAYS = 30;

function text(value: unknown) {
  return typeof value === 'string' ? value : null;
}

function number(value: unknown) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/**
 * Accepts the API list (`[{ start, end, battery_from, battery_to }]`) or the
 * object form (`{ gaps, parked_periods_checked, daily_samples }`). Rows without
 * times or battery levels are dropped.
 */
export function normalizeChargeGaps(vehicleId: string, raw: unknown): ChargeGapsResponse {
  const body = (raw && typeof raw === 'object' && !Array.isArray(raw) ? raw : {}) as Record<string, unknown>;
  const rows = Array.isArray(raw) ? raw : Array.isArray(body.gaps) ? body.gaps : [];
  const gaps = rows.flatMap((row): ChargeGapsResponse['gaps'] => {
    const item = (row && typeof row === 'object' ? row : {}) as Record<string, unknown>;
    const started = text(item.started_at) ?? text(item.start);
    const ended = text(item.ended_at) ?? text(item.end);
    const from = number(item.soc_start) ?? number(item.battery_from);
    const to = number(item.soc_end) ?? number(item.battery_to);
    if (!started || !ended || from === null || to === null) return [];
    return [{
      id: text(item.id) ?? started, started_at: started, ended_at: ended, soc_start: from, soc_end: to,
      latitude: number(item.latitude), longitude: number(item.longitude),
    }];
  });
  const samples = Array.isArray(body.daily_samples)
    ? body.daily_samples.flatMap((row) => {
      const item = (row && typeof row === 'object' ? row : {}) as Record<string, unknown>;
      const day = text(item.day);
      const count = number(item.samples);
      return day && count !== null ? [{ day, samples: count }] : [];
    })
    : null;
  return {
    vehicle_id: text(body.vehicle_id) ?? vehicleId,
    lookback_days: number(body.lookback_days) ?? CHARGE_GAP_LOOKBACK_DAYS,
    parked_periods_checked: number(body.parked_periods_checked),
    gaps,
    daily_samples: samples,
  };
}

export function useChargeGaps(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery({
    queryKey: carDataHealthKeys.chargeGaps(vehicleId),
    queryFn: () => fetchChargeGaps(vehicleId!),
    enabled: authReady && !!vehicleId,
    // The server scans 30 days of telemetry.
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
    retry: false,
  });
}

/** Charging-history recovery status. Polls while a recovery runs. */
export function useChargeHistoryBackfillStatus(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery<BackfillStatus>({
    queryKey: carDataHealthKeys.backfillStatus(vehicleId),
    queryFn: () => api.getBackfillStatus(vehicleId!),
    enabled: authReady && !!vehicleId,
    staleTime: 10 * 1000,
    refetchInterval: (query) => (query.state.data?.status === 'running' ? 3000 : false),
    retry: false,
  });
}

/**
 * Asks Rivian for the charging history and adds any charge the app missed.
 * Owners and managers only. A 409 means a recovery already runs.
 */
export function useRecoverMissedCharges(vehicleId: string | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      try {
        await api.triggerBackfill(vehicleId!);
      } catch (error) {
        if ((error as { status?: number } | null)?.status !== 409) throw error;
      }
    },
    onSuccess: () => {
      queryClient.setQueryData<BackfillStatus | undefined>(
        carDataHealthKeys.backfillStatus(vehicleId),
        (previous) => (previous ? { ...previous, status: 'running' } : previous),
      );
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: carDataHealthKeys.backfillStatus(vehicleId) }),
        queryClient.invalidateQueries({ queryKey: ['jobs', 'backfill-status', vehicleId] }),
      ]);
    },
  });
}

const LIFETIME_PAGE_SIZE = 200;
const LIFETIME_PAGE_LIMIT = 25;

/** Every recorded trip (newest first), for the odometer-against-trips check. */
export async function fetchLifetimeTrips(vehicleId: string): Promise<Trip[]> {
  const trips: Trip[] = [];
  for (let page = 1; page <= LIFETIME_PAGE_LIMIT; page += 1) {
    const response = await api.listTrips(vehicleId, null, null, page, LIFETIME_PAGE_SIZE, '', true);
    trips.push(...response.items);
    if (response.items.length < LIFETIME_PAGE_SIZE || trips.length >= response.total) break;
  }
  return trips;
}

export function useLifetimeTrips(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery({
    queryKey: carDataHealthKeys.lifetimeTrips(vehicleId),
    queryFn: () => fetchLifetimeTrips(vehicleId!),
    enabled: authReady && !!vehicleId,
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
  });
}

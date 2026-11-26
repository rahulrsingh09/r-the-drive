import { useQueries, useQuery } from '@tanstack/react-query';
import { api, useAuthReady } from '@panda/hooks';
import type { ChargeCurvePoint, ChargeSession, Trip } from '@panda/types';

/** The list endpoints return at most this many rows per page. */
const PAGE = 200;
/** A guard against a runaway loop; 100 pages is 20,000 rows while still breaking early once all items arrive. */
const MAX_PAGES = 100;

async function readAll<T>(load: (page: number) => Promise<{ items: T[]; total: number }>): Promise<T[]> {
  const out: T[] = [];
  for (let page = 1; page <= MAX_PAGES; page += 1) {
    const response = await load(page);
    out.push(...response.items);
    if (!response.items.length || out.length >= response.total || response.items.length < PAGE) break;
  }
  return out;
}

/** Every trip since the first day of data. The key starts with "trips" so trip mutations refresh it. */
export function useAllTrips(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery<Trip[]>({
    queryKey: ['trips', 'list', 'insights-all', vehicleId],
    queryFn: () => readAll(page => api.listTrips(vehicleId!, null, null, page, PAGE, '', true) as Promise<{ items: Trip[]; total: number }>),
    enabled: authReady && !!vehicleId,
    staleTime: 60 * 1000,
    placeholderData: previous => previous,
  });
}

/** Every charge since the first day of data. The key starts with "charging" so charge and place mutations refresh it. */
export function useAllCharges(vehicleId: string | null) {
  const authReady = useAuthReady();
  return useQuery<ChargeSession[]>({
    queryKey: ['charging', 'list', 'insights-all', vehicleId],
    queryFn: () => readAll(page => api.listChargeSessions(vehicleId!, null, null, page, PAGE, '', true) as Promise<{ items: ChargeSession[]; total: number }>),
    enabled: authReady && !!vehicleId,
    staleTime: 60 * 1000,
    placeholderData: previous => previous,
  });
}

/** The battery-level curve of each charge. Shares the cache with the charge detail page. */
export function useChargeCurves(vehicleId: string | null, sessionIds: readonly string[]) {
  const authReady = useAuthReady();
  const results = useQueries({
    queries: sessionIds.map(id => ({
      queryKey: ['charging', 'curve', id, vehicleId],
      queryFn: () => api.getChargeCurve(id, vehicleId!),
      enabled: authReady && !!vehicleId,
      staleTime: 5 * 60 * 1000,
      refetchOnWindowFocus: false,
    })),
  });
  const curves = new Map<string, ChargeCurvePoint[]>();
  results.forEach((result, index) => { if (result.data) curves.set(sessionIds[index]!, result.data); });
  return { curves, loading: results.some(result => result.isLoading) };
}

import React from 'react';
import { useQuery } from '@tanstack/react-query';
import type { VehicleStatus } from '@panda/types';
import { api, useAuthReady } from '@panda/hooks';

/** The trip in progress, from the server's trip checkpoint. */
export interface ActiveTrip {
  started_at: string;
  start_odometer_mi: number | null;
  soc_start: number | null;
  range_start_mi: number | null;
  points: Array<{ ts: string; lat: number; lng: number }>;
}

function isActiveTrip(value: unknown): value is ActiveTrip {
  const trip = value as ActiveTrip | null;
  return Boolean(trip) && typeof trip!.started_at === 'string' && Array.isArray(trip!.points);
}

/**
 * The API does not serve GET /v1/vehicles/{id}/active-trip yet. Until it does, the app does not
 * call it, and the live view works from the vehicle status and the last trip alone.
 * Set this to true in the same change that adds the endpoint to the API.
 */
export const ACTIVE_TRIP_API_AVAILABLE = false;

/**
 * GET /v1/vehicles/{id}/active-trip while the car drives. The read is optional: a server
 * without it (404), with no trip in progress (204) or with an error gives null, and the
 * live view then works from the vehicle status alone. Errors are not reported to the owner.
 */
export async function fetchActiveTrip(vehicleId: string): Promise<ActiveTrip | null> {
  try {
    const response = await api.requestResponse('GET', `/v1/vehicles/${vehicleId}/active-trip`, undefined, undefined, true, false);
    if (response.status === 204) return null;
    const body: unknown = await response.json().catch(() => null);
    return isActiveTrip(body) ? body : null;
  } catch {
    return null;
  }
}

export function useActiveTrip(vehicleId: string, enabled: boolean) {
  const authReady = useAuthReady();
  return useQuery({
    queryKey: ['now', 'active-trip', vehicleId],
    queryFn: () => fetchActiveTrip(vehicleId),
    enabled: ACTIVE_TRIP_API_AVAILABLE && authReady && enabled,
    refetchInterval: ACTIVE_TRIP_API_AVAILABLE && enabled ? 30_000 : false,
    retry: false,
    staleTime: 0,
  });
}

export interface LiveSample { at: number; lat: number | null; lng: number | null; battery: number | null }

/**
 * Vehicle positions and battery readings seen while this page is open and the car drives.
 * A new sample is kept when the position or the battery changes. The list empties when the drive ends.
 */
export function useLiveSamples(status: VehicleStatus | null, enabled: boolean) {
  const [samples, setSamples] = React.useState<LiveSample[]>([]);
  const lat = status?.latitude ?? null;
  const lng = status?.longitude ?? null;
  const battery = status?.battery_level ?? null;
  React.useEffect(() => {
    if (!enabled) { setSamples(current => (current.length ? [] : current)); return; }
    if (lat == null && battery == null) return;
    setSamples(current => {
      const last = current.at(-1);
      if (last && last.lat === lat && last.lng === lng && last.battery === battery) return current;
      return [...current, { at: Date.now(), lat, lng, battery }].slice(-2000);
    });
  }, [enabled, lat, lng, battery]);
  return samples;
}

/** Ticks every `ms` while enabled, for live durations. */
export function useClock(enabled: boolean, ms = 15_000) {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    if (!enabled) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(timer);
  }, [enabled, ms]);
  return now;
}

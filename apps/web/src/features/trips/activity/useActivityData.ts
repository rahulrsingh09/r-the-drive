import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { api, useAuthReady, useSavedPlaces } from '@panda/hooks';
import type { ChargeSession, Trip } from '@panda/types';
import { startOfAppDay } from '@panda/ui/lib/dateTime';
import { buildActivityData, dayKey, periodStartDay, shiftDay, type ActivityPeriod, type RouteCoordinate } from './activityModel';
import { inferRoadTrips, useRoadTripOverrides } from './roadTripModel';

/** The API returns at most 200 rows per page. */
const PAGE_SIZE = 200;
/** Upper bound for "All time": 10 000 trips or charges. */
const MAX_PAGES = 50;

async function readAll<T>(read: (page: number) => Promise<{ items: T[]; total: number }>) {
  const items: T[] = [];
  for (let page = 1; page <= MAX_PAGES; page += 1) {
    const response = await read(page);
    items.push(...response.items);
    if (response.items.length < PAGE_SIZE || items.length >= response.total) break;
  }
  return items;
}

/** ISO bounds for the period. One extra day before the period lets parked time and odometer gaps cross its first edge. */
export function periodBounds(period: ActivityPeriod, now = new Date()) {
  const today = dayKey(now);
  const first = periodStartDay(period, today);
  if (!first) return { today, from: null, to: null };
  const from = startOfAppDay(new Date(`${shiftDay(first, -1)}T12:00:00Z`)).toISOString();
  return { today, from, to: now.toISOString() };
}

const QUERY_OPTIONS = {
  staleTime: 60 * 1000,
  refetchOnWindowFocus: false,
  refetchOnReconnect: false,
  placeholderData: <T,>(previous: T | undefined) => previous,
} as const;

/** Trips, charges, saved places and route previews for one Activity period. */
export function useActivityData(vehicleId: string | null, period: ActivityPeriod) {
  const authReady = useAuthReady();
  // The period bounds move with the day, not with every render.
  const today = dayKey(new Date());
  const { from, to } = React.useMemo(() => periodBounds(period), [period, today]); // eslint-disable-line react-hooks/exhaustive-deps
  const lifetime = from == null;
  const enabled = authReady && !!vehicleId;

  const trips = useQuery({
    queryKey: ['trips', 'activity', 'v1', vehicleId, period, from],
    queryFn: () => readAll<Trip>((page) => api.listTrips(vehicleId!, from, to, page, PAGE_SIZE, '', lifetime)),
    enabled,
    ...QUERY_OPTIONS,
  });
  const charges = useQuery({
    queryKey: ['charging', 'activity', 'v1', vehicleId, period, from],
    queryFn: () => readAll<ChargeSession>((page) => api.listChargeSessions(vehicleId!, from, to, page, PAGE_SIZE, '', lifetime)),
    enabled,
    ...QUERY_OPTIONS,
  });
  const routes = useQuery({
    queryKey: ['trips', 'activity-routes', 'v1', vehicleId, period, from],
    queryFn: () => api.getTripMap(vehicleId!, from, to, '', lifetime),
    enabled,
    ...QUERY_OPTIONS,
    meta: { persist: false },
  });
  const places = useSavedPlaces();
  const roadTripOverrides = useRoadTripOverrides(vehicleId);

  const routeMap = React.useMemo(() => new Map<string, RouteCoordinate[]>(
    (routes.data?.routes ?? []).map((route) => [route.trip_id, route.coordinates as RouteCoordinate[]]),
  ), [routes.data]);
  const data = React.useMemo(() => {
    const base = buildActivityData(trips.data ?? [], charges.data ?? [], places.data ?? [], routeMap);
    const roadTrips = inferRoadTrips(base.trips, base.charges, places.data ?? [], roadTripOverrides.overrides);
    return { ...base, roadTrips };
  }, [charges.data, places.data, roadTripOverrides.overrides, routeMap, trips.data]);

  return {
    data,
    today,
    routes: routeMap,
    places: places.data ?? [],
    roadTripControls: {
      rename: roadTripOverrides.rename,
      split: roadTripOverrides.split,
    },
    loading: (trips.isLoading && !trips.data) || (charges.isLoading && !charges.data),
    error: (trips.isError && !trips.data) || (charges.isError && !charges.data),
    retry: () => { void trips.refetch(); void charges.refetch(); },
  };
}

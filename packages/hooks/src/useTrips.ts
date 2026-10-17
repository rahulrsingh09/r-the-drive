import { useApiQuery, queryPolicies } from './queryRuntime';
import { canonicalTagFilter } from './queryRuntime';
import { cacheInvalidations, useCacheMutation } from './cacheRuntime';
import type { TripTagAssignmentRequest } from '@panda/types';
import { api } from './api';

const TRIPS_LIST_QUERY_VERSION = 'v2';

export interface TripTagFilters {
  tagIds?: string[];
  tagMatch?: 'all' | 'any';
  untagged?: boolean;
}


export function useTrips(vehicleId: string | null, from: string | null, to: string | null, page = 1, perPage = 25, search = '', filters?: TripTagFilters) {
  const normalizedSearch = search.trim();
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['trips', 'list', TRIPS_LIST_QUERY_VERSION, vehicleId, from, to, lifetime, page, perPage, normalizedSearch, tagFilters], 'listTrips', [vehicleId!, from, to, page, perPage, normalizedSearch, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 60 * 1000,
    previous: false
  });
}

export function useTripMapRoutes(
  vehicleId: string | null,
  from: string | null,
  to: string | null,
  search = '',
  filters?: TripTagFilters,
) {
  const normalizedSearch = search.trim();
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['trips', 'map', 'v1', vehicleId, from, to, lifetime, normalizedSearch, tagFilters], 'getTripMap', [vehicleId!, from, to, normalizedSearch, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 60 * 1000,
    gcTime: 10 * 60 * 1000,
    meta: { persist: false },
    previous: false
  });
}

export function useTirePressureTimeline(
  vehicleId: string | null,
  from: string | null,
  to: string | null,
  filters?: TripTagFilters,
) {
  const lifetime = !from && !to;
  const tagFilters = canonicalTagFilter(filters);
  return useApiQuery(['trips', 'tire-pressure-timeline', 'v1', vehicleId, from, to, lifetime, tagFilters], 'getTirePressureTimeline', [vehicleId!, from, to, lifetime, tagFilters], [vehicleId], {
    ...queryPolicies.detail,
    staleTime: 60 * 1000,
    gcTime: 10 * 60 * 1000,
    meta: { persist: false },
    previous: false
  });
}

export function useTripTags(vehicleId: string | null) {
  return useApiQuery(['trip-tags', vehicleId], 'listTripTags', [vehicleId!], [vehicleId], {
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false
  });
}

export function useCreateTripTag(vehicleId: string | null) {
  return useCacheMutation((body: { name: string }) => api.createTripTag(vehicleId!, body), () => tripTagInvalidations(vehicleId));
}

export function useUpdateTripTag(vehicleId: string | null) {
  return useCacheMutation(({ tagId, ...body }: { tagId: string; name?: string }) => api.updateTripTag(vehicleId!, tagId, body), () => tripTagInvalidations(vehicleId));
}

export function useDeleteTripTag(vehicleId: string | null) {
  return useCacheMutation((tagId: string) => api.deleteTripTag(vehicleId!, tagId), () => tripTagInvalidations(vehicleId));
}

export function useUpdateTripTagAssignments(vehicleId: string | null) {
  return useCacheMutation((body: TripTagAssignmentRequest) => api.updateTripTagAssignments(vehicleId!, body), () => tripTagInvalidations(vehicleId));
}

function tripTagInvalidations(vehicleId: string | null) {
  return cacheInvalidations(vehicleId ? [['trip-tags', vehicleId], ['trips']] : []);
}

export function useTrip(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'detail', tripId, vehicleId], 'getTrip', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000
  });
}

export function useTripDetailData(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'detail-data', 'v1', tripId, vehicleId], 'getTripDetailData', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: Infinity,
    gcTime: 30 * 60 * 1000,
    meta: { persist: false }
  });
}

export function useTripTrack(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'track', tripId, vehicleId], 'getTripTrack', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    meta: { persist: false }
  });
}

export function useSpeedProfile(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'speed', tripId, vehicleId], 'getSpeedProfile', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    meta: { persist: false }
  });
}

export function useElevationProfile(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'elevation', tripId, vehicleId], 'getElevationProfile', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    meta: { persist: false }
  });
}

export function useTripPowerProfile(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'power', tripId, vehicleId], 'getTripPowerProfile', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    meta: { persist: false }
  });
}

export function useTripDetailSeries(tripId: string | null, vehicleId: string | null) {
  return useApiQuery(['trips', 'series', tripId, vehicleId], 'getTripDetailSeries', [tripId!, vehicleId!], [tripId, vehicleId], {
    ...queryPolicies.detail,
    staleTime: 5 * 60 * 1000,
    meta: { persist: false }
  });
}

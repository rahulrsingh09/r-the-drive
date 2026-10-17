import { useResourceQuery } from './queryRuntime';
import { useCacheMutation } from './cacheRuntime';
import type { MapStylePreference, UserPreferencesResponse } from '@panda/types';
import { api } from './api';
import { queryKeys } from './queryKeys';

/**
 * The combined preference endpoint deliberately shares the long-standing
 * unit-preference key so existing callers continue to receive fresh data.
 */
export function useUserPreferences() {
  return useResourceQuery(queryKeys.unitPreferences.current, () => api.apiFetch<UserPreferencesResponse>('GET', '/v1/auth/preferences'), [], {
    staleTime: 5 * 60 * 1000
  });
}

export function useUpdateMapStyle() {
  const key = queryKeys.unitPreferences.current;
  return useCacheMutation(
    (mapStyle: MapStylePreference) => api.apiFetch<{ map_style: MapStylePreference }>('PUT', '/v1/auth/preferences/map-style', { map_style: mapStyle }),
    () => [],
    (mapStyle) => ({ key, patch: { map_style: mapStyle } }),
    [key],
  );
}

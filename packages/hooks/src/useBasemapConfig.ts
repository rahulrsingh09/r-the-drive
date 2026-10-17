import { useResourceQuery } from './queryRuntime';
import { api } from './api';
import type { BasemapConfigPayload } from '@panda/types';

export type { BasemapConfigPayload } from '@panda/types';

export const BASEMAP_CONFIG_QUERY_KEY = ['external', 'basemap', 'config', 'v2'] as const;

/**
 * Resolves the authenticated first-party basemap configuration for connected
 * map surfaces. Rendering packages receive only this data, never transport.
 */
export function useBasemapConfig() {
  return useResourceQuery(BASEMAP_CONFIG_QUERY_KEY, () => api.apiFetch<BasemapConfigPayload>('GET', '/v1/external/basemap/config'), [], {
    staleTime: 5 * 60 * 1000,
    retry: 1,
    refetchOnWindowFocus: false
  });
}

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { api, invalidateChargingData } from '@panda/hooks';
import type { PlaceCategory } from '../../trips/activity/activityModel';

/** API tag names. The app calls "public" places Other. */
export type PlaceTagName = 'home' | 'office' | 'other';

export interface PlaceTagResponse {
  tag: PlaceTagName;
  place_id: string | null;
  updated: number;
  session_ids: string[];
}

export function placeTagName(category: PlaceCategory): PlaceTagName {
  return category === 'public' ? 'other' : category;
}

export function placeTagPath(vehicleId: string, sessionId: string) {
  return `/v1/vehicles/${encodeURIComponent(vehicleId)}/charging-sessions/${encodeURIComponent(sessionId)}/place-tag`;
}

/**
 * Tags a charge's place Home, Office or Other. By default (`scope = 'session'`),
 * only this single charge is updated so other sessions are untouched. Callers
 * that explicitly pass `scope = 'place'` apply the tag to every charge
 * physically within 150 m. The API answers 422 when no Home or Office place
 * is saved.
 */
export function usePlaceTag(vehicleId: string | null) {
  const queryClient = useQueryClient();
  return useMutation<
    PlaceTagResponse,
    Error,
    { sessionId: string; category: PlaceCategory; scope?: 'session' | 'place' }
  >({
    mutationFn: ({ sessionId, category, scope = 'session' }) => {
      if (!vehicleId) throw new Error('vehicle_id is required');
      return api.apiFetch<PlaceTagResponse>('POST', placeTagPath(vehicleId, sessionId), {
        tag: placeTagName(category),
        scope,
      });
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['places'] });
      void queryClient.invalidateQueries({ queryKey: ['vehicles', 'insights'] });
      invalidateChargingData(queryClient);
    },
  });
}

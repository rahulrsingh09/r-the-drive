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
 * Tags a charge's place Home, Office or Other. The API applies the tag to this
 * charge and every charge of the vehicle at the same place (the same saved
 * place, or within 150 m): Home and Office link the saved Home or Office place
 * and its rate; Other removes the saved place and its rate. The API answers 422
 * when no Home or Office place is saved.
 */
export function usePlaceTag(vehicleId: string | null) {
  const queryClient = useQueryClient();
  return useMutation<PlaceTagResponse, Error, { sessionId: string; category: PlaceCategory }>({
    mutationFn: ({ sessionId, category }) => {
      if (!vehicleId) throw new Error('vehicle_id is required');
      return api.apiFetch<PlaceTagResponse>('POST', placeTagPath(vehicleId, sessionId), { tag: placeTagName(category) });
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['places'] });
      void queryClient.invalidateQueries({ queryKey: ['vehicles', 'insights'] });
      invalidateChargingData(queryClient);
    },
  });
}

import React from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { api, invalidateChargingData } from '@panda/hooks';
import type { Place, PlaceChargingInput, PlaceChargingProfile } from '@panda/types';
import { networkTitle } from '../charging/detail/chargeDetailModel';
import { nearestTown } from '../../lib/placeNames';
import { CATEGORY_LABEL, type Category, type ChargeRow, type PlaceGroup } from './insightsModel';

/** The saved place whose rate a category uses: the Home or the Office place, the one with a cost profile first. */
export function ratePlace(places: readonly Place[], category: Category): Place | null {
  if (category === 'other') return null;
  const flagged = places.filter(place => (category === 'home' ? place.is_home : place.is_work));
  return flagged.find(place => place.charging) ?? flagged[0] ?? null;
}

/** The source of a rate from its profile name: "Home charging (PSE Schedule 7)" gives "PSE Schedule 7". */
export function profileSource(name: string | null | undefined) {
  const trimmed = name?.trim() ?? '';
  const inner = /\(([^)]+)\)\s*$/.exec(trimmed)?.[1]?.trim() ?? trimmed;
  if (!inner || /^(free|paid|home|office|work)$/i.test(inner) || /\bcharging$/i.test(inner)) return null;
  return inner;
}

/** "Kirkland · PSE Schedule 7": the town of the place, then the network or the rate source. */
export function rateDetail(place: Place | null, charges: readonly ChargeRow[]) {
  if (!place) return 'No place tagged yet';
  const town = place.address?.city?.trim() || nearestTown(place.latitude, place.longitude);
  const vendors = new Map<string, number>();
  for (const charge of charges) {
    const vendor = charge.place?.id === place.id ? networkTitle(charge.session.network_vendor) : null;
    if (vendor) vendors.set(vendor, (vendors.get(vendor) ?? 0) + 1);
  }
  const network = [...vendors.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
  return [town, network ?? profileSource(place.charging?.name)].filter(Boolean).join(' · ') || place.name;
}

/** A place cost profile as a place input. Free becomes $0 per kWh; per-minute profiles are not supported here. */
export function profileInput(profile: PlaceChargingProfile | null | undefined): PlaceChargingInput | null {
  if (!profile) return null;
  const base = { ...(profile.name ? { name: profile.name } : {}), session_fee: profile.session_fee ?? 0, currency: profile.currency || 'USD', timezone: profile.timezone };
  if (profile.billing_type === 'free') return { ...base, billing_type: 'per_kwh', rate: 0, session_fee: 0 };
  if (profile.billing_type === 'per_kwh' || profile.billing_type === 'flat') return { ...base, billing_type: 'per_kwh', rate: profile.rate };
  if (profile.billing_type === 'tou') return { ...base, billing_type: 'tou', rate: profile.rate, tou_periods: profile.tou_periods };
  return null;
}

/**
 * Applies a tag to a single-charge place with session scope: Home and Office link the charge to the
 * saved Home or Office place (its rate prices it); Other removes the place and its rate.
 * Multi-charge groups are never bulk-tagged so previous charges stay untouched.
 */
export function usePlaceTag(vehicleId: string) {
  const queryClient = useQueryClient();
  const [pending, setPending] = React.useState<string | null>(null);
  const [message, setMessage] = React.useState<{ text: string; error: boolean } | null>(null);
  const apply = React.useCallback(async (group: PlaceGroup, category: Category) => {
    const anchor = group.charges[0];
    if (!anchor || group.charges.length !== 1 || group.category === category) return;
    setPending(group.key);
    setMessage(null);
    try {
      const result = await api.applyChargePlaceTag(vehicleId, anchor.id, category, 'session');
      const count = result.updated;
      setMessage({ text: `${group.name} is now tagged ${CATEGORY_LABEL[category]}. ${count} ${count === 1 ? 'charge was' : 'charges were'} updated.`, error: false });
    } catch (cause) {
      setMessage({ text: `The tag did not save. ${cause instanceof Error ? cause.message : ''} Try again.`.replace(/\s+/g, ' ').trim(), error: true });
    } finally {
      setPending(null);
      void queryClient.invalidateQueries({ queryKey: ['places'] });
      invalidateChargingData(queryClient);
    }
  }, [queryClient, vehicleId]);
  return { apply, pending, message };
}

import type { Place } from '@panda/types';

export const TRIP_LOCATION_UNAVAILABLE_COPY = 'Location unavailable';

export interface ResolvedTripLocation {
  label: string;
  title: string;
  source: 'place' | 'address' | 'coordinates' | 'unavailable';
}

const PLACE_FIELDS = ['place', 'place_name', 'location_name', 'location', 'label'];
const ADDRESS_FIELDS = ['address', 'address_display_name', 'address_name', 'address_label'];
export function resolveTripLocation(trip: Record<string, unknown>, kind: 'start' | 'end', places: Place[] = []): ResolvedTripLocation {
  const labelFrom = (fields: string[]) => fields.map(field => trip[`${kind}_${field}`]).find((value): value is string => typeof value === 'string' && value.trim() !== '');
  const named = labelFrom(PLACE_FIELDS);
  if (named) return { label: named, title: named, source: 'place' };
  const lat = trip[`${kind}_lat`], lng = trip[`${kind}_lng`];
  const located = typeof lat === 'number' && Number.isFinite(lat) && typeof lng === 'number' && Number.isFinite(lng) && !(lat === 0 && lng === 0);
  const nearest = located ? places.map(place => ({ place, distance: distanceMeters(lat, lng, place.latitude, place.longitude) }))
    .filter(candidate => candidate.distance <= candidate.place.radius_m)
    .sort((a, b) => a.distance - b.distance)[0]?.place : undefined;
  if (nearest) return { label: nearest.name, title: nearest.address?.display_name ?? nearest.name, source: 'place' };
  const address = labelFrom(ADDRESS_FIELDS);
  const source = address ? 'address' : located ? 'coordinates' : 'unavailable';
  const label = address ?? (located ? `${lat.toFixed(5)}, ${lng.toFixed(5)}` : TRIP_LOCATION_UNAVAILABLE_COPY);
  return { source, label, title: label };
}
function distanceMeters(lat1: number, lng1: number, lat2: number, lng2: number) {
  const earthRadius = 6_371_000;
  const toRad = (value: number) => (value * Math.PI) / 180;
  const dLat = toRad(lat2 - lat1);
  const dLng = toRad(lng2 - lng1);
  const a = Math.sin(dLat / 2) ** 2 + Math.cos(toRad(lat1)) * Math.cos(toRad(lat2)) * Math.sin(dLng / 2) ** 2;
  return 2 * earthRadius * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

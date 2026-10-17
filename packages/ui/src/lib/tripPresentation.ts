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
  let nearest: Place | undefined;
  if (located) {
    const latitudeWeight = Math.cos((lat * Math.PI) / 180);
    let minimum = Infinity;
    for (const place of places) {
      const halfAngles = [place.latitude - lat, place.longitude - lng].map(degrees => Math.sin(((degrees * Math.PI) / 180) / 2) ** 2);
      const chord = halfAngles[0]! + latitudeWeight * Math.cos((place.latitude * Math.PI) / 180) * halfAngles[1]!;
      const distance = 12_742_000 * Math.atan2(Math.sqrt(chord), Math.sqrt(1 - chord));
      if (distance <= place.radius_m && distance < minimum) { nearest = place; minimum = distance; }
    }
  }
  if (nearest) return { label: nearest.name, title: nearest.address?.display_name ?? nearest.name, source: 'place' };
  const address = labelFrom(ADDRESS_FIELDS);
  const source = address ? 'address' : located ? 'coordinates' : 'unavailable';
  const label = address ?? (located ? `${lat.toFixed(5)}, ${lng.toFixed(5)}` : TRIP_LOCATION_UNAVAILABLE_COPY);
  return { source, label, title: label };
}

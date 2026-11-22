import type { ChargeSession, Place, Trip } from '@panda/types';

/**
 * One naming rule for places on every page (Activity, charge detail, Insights):
 * 1. the saved place name,
 * 2. else a short address town or a short reported name,
 * 3. else the nearest town, marked approximate ("Near Kirkland"; titles show "Kirkland"),
 * 4. else the coordinates, as a last resort,
 * 5. else "Unknown place".
 */
export interface PlaceLabel {
  name: string;
  /** True when the name is the nearest town, not a saved place or an address. */
  approximate: boolean;
}

/** Town centroids for the nearest-town name. Keep in step with the trial's derive.py TOWNS. */
const TOWNS: ReadonlyArray<readonly [string, number, number]> = [
  ['Kirkland', 47.6769, -122.2060], ['Bellevue', 47.6101, -122.2015], ['Seattle', 47.6062, -122.3321],
  ['Redmond', 47.6740, -122.1215], ['Bothell', 47.7623, -122.2054], ['Mill Creek', 47.8601, -122.2043],
  ['Lynnwood', 47.8209, -122.3151], ['Everett', 47.9790, -122.2021], ['Woodinville', 47.7543, -122.1635],
  ['Kenmore', 47.7573, -122.2440], ['Issaquah', 47.5301, -122.0326], ['Renton', 47.4829, -122.2171],
  ['Tukwila', 47.4740, -122.2610], ['Federal Way', 47.3223, -122.3126], ['Tacoma', 47.2529, -122.4443],
  ['Lacey', 47.0343, -122.8232], ['Olympia', 47.0379, -122.9007], ['Centralia', 46.7162, -122.9543],
  ['Chehalis', 46.6621, -122.9640], ['Toledo', 46.4401, -122.8468], ['Castle Rock', 46.2751, -122.9076],
  ['Kelso', 46.1468, -122.9084], ['Longview', 46.1382, -122.9382], ['Woodland', 45.9045, -122.7437],
  ['Ridgefield', 45.8151, -122.7426], ['Vancouver', 45.6387, -122.6615], ['Portland', 45.5152, -122.6784],
  ['Shoreline', 47.7557, -122.3415], ['Sammamish', 47.6163, -122.0356], ['Snohomish', 47.9129, -122.0982],
];

/** Farther than this from every known town, the place has no town name. */
const MAX_TOWN_KM = 60;

const COORDINATE_LABEL = /^-?\d+(\.\d+)?\s*,\s*-?\d+(\.\d+)?$/;

export const UNKNOWN_PLACE = 'Unknown place';

export function distanceKm(lat1: number, lng1: number, lat2: number, lng2: number) {
  const r = Math.PI / 180;
  const a = Math.sin(((lat2 - lat1) * r) / 2) ** 2
    + Math.cos(lat1 * r) * Math.cos(lat2 * r) * Math.sin(((lng2 - lng1) * r) / 2) ** 2;
  return 6371 * 2 * Math.asin(Math.min(1, Math.sqrt(a)));
}

function hasPosition(lat: number | null | undefined, lng: number | null | undefined): lat is number {
  return lat != null && lng != null && Number.isFinite(lat) && Number.isFinite(lng) && !(lat === 0 && lng === 0);
}

export function nearestTown(lat: number | null | undefined, lng: number | null | undefined): string | null {
  if (!hasPosition(lat, lng)) return null;
  let best: string | null = null;
  let bestKm = Number.POSITIVE_INFINITY;
  for (const [name, townLat, townLng] of TOWNS) {
    const km = distanceKm(lat, lng!, townLat, townLng);
    if (km < bestKm) { best = name; bestKm = km; }
  }
  return bestKm <= MAX_TOWN_KM ? best : null;
}

/** True for a "45.8178, -122.6827" label. */
export function isCoordinateLabel(name: string | null | undefined) {
  return name != null && COORDINATE_LABEL.test(name.trim());
}

export function coordinateLabel(lat: number | null | undefined, lng: number | null | undefined): string | null {
  return hasPosition(lat, lng) ? `${lat.toFixed(4)}, ${lng!.toFixed(4)}` : null;
}

/** The name without the "Near " mark, for titles and short labels. */
export function shortPlace(name: string) {
  return name.replace(/^Near /, '');
}

/** The saved place whose geofence holds the position (the nearest one). */
export function savedPlaceAt(lat: number | null | undefined, lng: number | null | undefined, places: readonly Place[]) {
  if (!hasPosition(lat, lng)) return null;
  let best: Place | null = null;
  let bestM = Number.POSITIVE_INFINITY;
  for (const place of places) {
    const meters = distanceKm(lat, lng!, place.latitude, place.longitude) * 1000;
    if (meters <= place.radius_m && meters < bestM) { best = place; bestM = meters; }
  }
  return best;
}

/** Steps 3 to 5 of the rule: nearest town, coordinates, "Unknown place". */
function fallbackLabel(lat: number | null | undefined, lng: number | null | undefined, city?: string | null): PlaceLabel {
  const town = nearestTown(lat, lng) ?? (city?.trim() || null);
  if (town) return { name: `Near ${town}`, approximate: true };
  return { name: coordinateLabel(lat, lng) ?? UNKNOWN_PLACE, approximate: false };
}

/** The start or end of a trip. */
export function tripPlace(
  trip: Pick<Trip, 'start_lat' | 'start_lng' | 'end_lat' | 'end_lng' | 'start_place' | 'end_place' | 'start_address' | 'end_address'>,
  kind: 'start' | 'end',
  places: readonly Place[],
): PlaceLabel {
  const lat = kind === 'start' ? trip.start_lat : trip.end_lat;
  const lng = kind === 'start' ? trip.start_lng : trip.end_lng;
  const saved = savedPlaceAt(lat, lng, places);
  if (saved) return { name: saved.name, approximate: false };
  const raw = (kind === 'start' ? trip.start_place : trip.end_place)?.trim();
  const placeName = raw && !isCoordinateLabel(raw) ? raw : null;
  const address = kind === 'start' ? trip.start_address : trip.end_address;
  // Without an address the API place name is a saved place name.
  if (placeName && !address) return { name: placeName, approximate: false };
  if (placeName && address) {
    const town = placeName.split(',').at(-1)?.trim();
    if (town) return { name: town, approximate: false };
  }
  return fallbackLabel(lat, lng);
}

/** The place of a charge. `savedPlace` is the saved place the caller matched, or null. */
export function chargePlace(
  session: Pick<ChargeSession, 'location_name' | 'location_lat' | 'location_lng' | 'rivian_city'>,
  savedPlace: Place | null,
): PlaceLabel {
  if (savedPlace) return { name: savedPlace.name, approximate: false };
  const name = session.location_name?.trim();
  // A full address display name is too long for a title; the nearest town replaces it.
  const isAddress = name != null && name.split(',').length > 2;
  if (name && !isCoordinateLabel(name) && !isAddress) return { name, approximate: false };
  const fallback = fallbackLabel(session.location_lat, session.location_lng, session.rivian_city);
  if (fallback.approximate) return fallback;
  if (name && isAddress) return { name: name.split(',').slice(0, 2).join(',').trim(), approximate: false };
  if (fallback.name === UNKNOWN_PLACE && name) return { name, approximate: false };
  return fallback;
}

/** "Home → Kirkland", or "Round trip · Home". */
export function tripTitleOf(from: PlaceLabel, to: PlaceLabel) {
  const a = shortPlace(from.name);
  const b = shortPlace(to.name);
  return a === b ? `Round trip · ${a}` : `${a} → ${b}`;
}

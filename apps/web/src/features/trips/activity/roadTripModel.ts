import React from 'react';
import type { Place } from '@panda/types';
import { getAppDateParts } from '@panda/ui/lib/dateTime';
import { shortPlace } from '../../../lib/placeNames';
import {
  averageEfficiency,
  dayKey,
  type ActivityCharge,
  type ActivityEvent,
  type ActivityRoadTrip,
  type ActivityTrip,
  type RoadTripDayBreakdown,
  type RouteCoordinate,
} from './activityModel';

export type { ActivityRoadTrip, RoadTripDayBreakdown };

export interface HomeAnchor {
  lat: number;
  lng: number;
  radiusMi: number;
  inferred: boolean;
}

export interface RoadTripOverrides {
  names: Record<string, string>;
  splitIds: string[];
}

const EMPTY_OVERRIDES: RoadTripOverrides = { names: {}, splitIds: [] };
const STORAGE_PREFIX = 'r-road-trips:v1:';
const OVERRIDE_EVENT = 'r-road-trip-overrides-changed';

export const ROAD_TRIP_FAR_MI = 60;
export const ROAD_TRIP_OVERNIGHT_AWAY_MI = 25;
export const ROAD_TRIP_OVERNIGHT_TOTAL_MI = 70;
export const ROAD_TRIP_DC_AWAY_MI = 30;
export const ROAD_TRIP_DC_TOTAL_MI = 100;
const HOME_BOUNDARY_MI = 0.5;

export function haversineMiles(lat1: number, lng1: number, lat2: number, lng2: number): number {
  const rad = Math.PI / 180;
  const dLat = (lat2 - lat1) * rad;
  const dLng = (lng2 - lng1) * rad;
  const h = Math.sin(dLat / 2) ** 2
    + Math.cos(lat1 * rad) * Math.cos(lat2 * rad) * Math.sin(dLng / 2) ** 2;
  return 3958.7613 * 2 * Math.asin(Math.min(1, Math.sqrt(h)));
}

function validCoord(lat: number | null | undefined, lng: number | null | undefined): lat is number {
  return lat != null && lng != null && Number.isFinite(lat) && Number.isFinite(lng) && !(lat === 0 && lng === 0);
}

/** Resolve Home from saved places or fall back to the most frequent overnight parking cluster. */
export function resolveHomeAnchor(trips: readonly ActivityTrip[], places: readonly Place[]): HomeAnchor | null {
  const savedHome = places.find((place) => place.is_home);
  if (savedHome && validCoord(savedHome.latitude, savedHome.longitude)) {
    return {
      lat: savedHome.latitude,
      lng: savedHome.longitude,
      radiusMi: Math.max(HOME_BOUNDARY_MI, savedHome.radius_m / 1609.344),
      inferred: false,
    };
  }

  // Fallback: detect repeated overnight trip endpoints (>= 40% of distinct nights, min 2).
  const overnightPoints: Array<{ lat: number; lng: number; day: string }> = [];
  for (const trip of trips) {
    const lat = trip.trip.end_lat;
    const lng = trip.trip.end_lng;
    if (typeof lat !== 'number' || typeof lng !== 'number' || !validCoord(lat, lng)) continue;
    const parts = getAppDateParts(trip.end);
    if (!parts) continue;
    if (parts.hour >= 22 || parts.hour <= 6) {
      overnightPoints.push({ lat, lng, day: dayKey(trip.end) });
    }
  }
  const uniqueNights = new Set(overnightPoints.map((p) => p.day)).size;
  if (uniqueNights < 2) return null;

  let bestCluster: { lat: number; lng: number; nights: Set<string> } | null = null;
  for (const point of overnightPoints) {
    const nights = new Set<string>();
    for (const candidate of overnightPoints) {
      if (haversineMiles(point.lat, point.lng, candidate.lat, candidate.lng) <= 0.25) {
        nights.add(candidate.day);
      }
    }
    if (!bestCluster || nights.size > bestCluster.nights.size) {
      bestCluster = { lat: point.lat, lng: point.lng, nights };
    }
  }

  if (!bestCluster || bestCluster.nights.size / uniqueNights < 0.4) return null;
  return {
    lat: bestCluster.lat,
    lng: bestCluster.lng,
    radiusMi: HOME_BOUNDARY_MI,
    inferred: true,
  };
}

function pointNearHome(
  lat: number | null | undefined,
  lng: number | null | undefined,
  placeName: string | undefined,
  home: HomeAnchor | null,
): boolean {
  if (placeName && placeName.trim().toLowerCase() === 'home') return true;
  if (!home || !validCoord(lat, lng)) return false;
  return haversineMiles(home.lat, home.lng, lat!, lng!) <= home.radiusMi;
}

function tripDistanceFromHome(trip: ActivityTrip, home: HomeAnchor | null): number {
  if (!home) return 0;
  let maxMi = 0;
  if (validCoord(trip.trip.start_lat, trip.trip.start_lng)) {
    maxMi = Math.max(maxMi, haversineMiles(home.lat, home.lng, trip.trip.start_lat!, trip.trip.start_lng!));
  }
  if (validCoord(trip.trip.end_lat, trip.trip.end_lng)) {
    maxMi = Math.max(maxMi, haversineMiles(home.lat, home.lng, trip.trip.end_lat!, trip.trip.end_lng!));
  }
  if (trip.route) {
    for (const [lng, lat] of trip.route) {
      maxMi = Math.max(maxMi, haversineMiles(home.lat, home.lng, lat, lng));
    }
  }
  return maxMi;
}

function chargeDistanceFromHome(charge: ActivityCharge, home: HomeAnchor | null): number {
  if (!home || !validCoord(charge.session.location_lat, charge.session.location_lng)) return 0;
  return haversineMiles(home.lat, home.lng, charge.session.location_lat!, charge.session.location_lng!);
}

function parkedOvernightAway(prev: ActivityTrip, next: ActivityTrip, home: HomeAnchor | null): boolean {
  const hours = (next.start.getTime() - prev.end.getTime()) / 3_600_000;
  if (hours < 6) return false;
  const distMi = validCoord(prev.trip.end_lat, prev.trip.end_lng) && home
    ? haversineMiles(home.lat, home.lng, prev.trip.end_lat!, prev.trip.end_lng!)
    : 0;
  if (distMi <= ROAD_TRIP_OVERNIGHT_AWAY_MI) return false;
  const startDay = dayKey(prev.end);
  const endDay = dayKey(next.start);
  if (endDay > startDay) return true;
  const startParts = getAppDateParts(prev.end);
  const endParts = getAppDateParts(next.start);
  return Boolean(startParts && endParts && startParts.hour <= 5 && endParts.hour >= 1);
}

function combinedRoute(trips: readonly ActivityTrip[]): RouteCoordinate[] {
  const points: RouteCoordinate[] = [];
  for (const trip of trips) {
    if (trip.route && trip.route.length > 1) {
      for (const coord of trip.route) points.push(coord);
    } else {
      if (validCoord(trip.trip.start_lat, trip.trip.start_lng)) {
        points.push([trip.trip.start_lng!, trip.trip.start_lat!]);
      }
      if (validCoord(trip.trip.end_lat, trip.trip.end_lng)) {
        points.push([trip.trip.end_lng!, trip.trip.end_lat!]);
      }
    }
  }
  return points;
}

function autoRoadTripTitle(
  trips: readonly ActivityTrip[],
  home: HomeAnchor | null,
  startDay: string,
  endDay: string,
): string {
  let destination = '';
  let maxDist = -1;
  for (const trip of trips) {
    const endDist = home && validCoord(trip.trip.end_lat, trip.trip.end_lng)
      ? haversineMiles(home.lat, home.lng, trip.trip.end_lat!, trip.trip.end_lng!)
      : trip.miles;
    const candidate = shortPlace(trip.to.name);
    if (candidate && candidate.toLowerCase() !== 'home' && endDist >= maxDist) {
      maxDist = endDist;
      destination = candidate;
    }
  }
  const base = destination || 'Road Trip';
  if (startDay === endDay) return destination ? `${base} Day Trip` : 'Day Road Trip';
  const startParts = getAppDateParts(trips[0]!.start);
  const endParts = getAppDateParts(trips.at(-1)!.end);
  if (startParts && endParts) {
    const startDow = new Date(Date.UTC(startParts.year, startParts.month - 1, startParts.day)).getUTCDay();
    const spanDays = Math.round((trips.at(-1)!.end.getTime() - trips[0]!.start.getTime()) / 86_400_000);
    if (spanDays <= 3 && (startDow === 5 || startDow === 6)) {
      return destination ? `${base} Weekend` : 'Weekend Road Trip';
    }
  }
  return destination ? `${base} Road Trip` : 'Road Trip';
}

function stableRoadTripId(trips: readonly ActivityTrip[]): string {
  return `rt-${trips[0]!.id}`;
}

/**
 * Groups consecutive Home -> Away -> Home loops into inferred road trips while
 * leaving daily commutes, local errands, and airport stays unbundled.
 */
export function inferRoadTrips(
  trips: readonly ActivityTrip[],
  charges: readonly ActivityCharge[],
  places: readonly Place[],
  overrides: RoadTripOverrides = EMPTY_OVERRIDES,
): ActivityRoadTrip[] {
  const home = resolveHomeAnchor(trips, places);
  if (!home || !trips.length) return [];

  const sortedTrips = [...trips].sort((a, b) => a.start.getTime() - b.start.getTime());
  const sortedCharges = [...charges].sort((a, b) => a.start.getTime() - b.start.getTime());
  const splitSet = new Set(overrides.splitIds);

  const loops: ActivityTrip[][] = [];
  let currentLoop: ActivityTrip[] = [];

  for (const trip of sortedTrips) {
    const startsHome = pointNearHome(trip.trip.start_lat, trip.trip.start_lng, trip.from.name, home);
    const endsHome = pointNearHome(trip.trip.end_lat, trip.trip.end_lng, trip.to.name, home);

    if (!currentLoop.length) {
      if (startsHome && !endsHome) {
        currentLoop = [trip];
      }
      continue;
    }

    currentLoop.push(trip);
    if (endsHome) {
      loops.push(currentLoop);
      currentLoop = [];
    }
  }
  if (currentLoop.length >= 2) {
    loops.push(currentLoop);
  }

  const out: ActivityRoadTrip[] = [];
  for (const loopTrips of loops) {
    const id = stableRoadTripId(loopTrips);
    if (splitSet.has(id)) continue;

    const start = loopTrips[0]!.start;
    const end = loopTrips.at(-1)!.end;
    const loopCharges = sortedCharges.filter(
      (charge) => charge.start.getTime() >= start.getTime() && charge.start.getTime() <= end.getTime(),
    );

    const totalMiles = loopTrips.reduce((sum, t) => sum + t.miles, 0);
    const totalMinutes = loopTrips.reduce((sum, t) => sum + t.minutes, 0);
    let maxDist = 0;
    for (const trip of loopTrips) maxDist = Math.max(maxDist, tripDistanceFromHome(trip, home));
    for (const charge of loopCharges) maxDist = Math.max(maxDist, chargeDistanceFromHome(charge, home));

    let hasOvernightAway = false;
    for (let i = 1; i < loopTrips.length; i += 1) {
      if (parkedOvernightAway(loopTrips[i - 1]!, loopTrips[i]!, home)) {
        hasOvernightAway = true;
        break;
      }
    }

    const hasAwayDc = loopCharges.some((charge) => {
      if (!charge.dc) return false;
      const dist = chargeDistanceFromHome(charge, home);
      return dist > ROAD_TRIP_DC_AWAY_MI || (dist === 0 && !charge.savedPlace?.is_home && maxDist > ROAD_TRIP_DC_AWAY_MI);
    });

    const farExcursion = maxDist >= ROAD_TRIP_FAR_MI;
    const overnightTrip = hasOvernightAway && totalMiles >= ROAD_TRIP_OVERNIGHT_TOTAL_MI;
    const corridorDc = totalMiles >= ROAD_TRIP_DC_TOTAL_MI && hasAwayDc;
    if (!farExcursion && !overnightTrip && !corridorDc) continue;

    const startDay = dayKey(start);
    const endDay = dayKey(end);
    const defaultName = autoRoadTripTitle(loopTrips, home, startDay, endDay);
    const customName = overrides.names[id]?.trim();
    const name = customName || defaultName;

    const reasonParts: string[] = [];
    if (maxDist > 0) reasonParts.push(`${Math.round(maxDist)} mi from Home`);
    if (hasOvernightAway) reasonParts.push('overnight away');
    if (hasAwayDc) reasonParts.push('DC fast charge');
    const inferenceReason = `Auto-bundled: ${reasonParts.join(' + ') || `${Math.round(totalMiles)} mi loop`}`;

    const events: ActivityEvent[] = [...loopTrips, ...loopCharges].sort(
      (a, b) => a.start.getTime() - b.start.getTime(),
    );

    const dayMap = new Map<string, ActivityEvent[]>();
    for (const ev of events) {
      dayMap.set(ev.day, [...(dayMap.get(ev.day) ?? []), ev]);
    }
    const days: RoadTripDayBreakdown[] = [...dayMap.keys()].sort().map((day) => {
      const list = dayMap.get(day)!;
      return {
        day,
        miles: list.reduce((sum, ev) => sum + (ev.kind === 'trip' ? ev.miles : 0), 0),
        kwh: list.reduce((sum, ev) => sum + (ev.kind === 'charge' ? ev.kwh : 0), 0),
        cost: list.reduce((sum, ev) => sum + (ev.kind === 'charge' ? (ev.cost ?? 0) : 0), 0),
        unpricedCount: list.filter((ev) => ev.kind === 'charge' && ev.cost == null && !ev.active).length,
        events: list,
      };
    });

    out.push({
      kind: 'road_trip',
      id,
      start,
      end,
      day: endDay,
      name,
      defaultName,
      inferenceReason,
      miles: totalMiles,
      minutes: totalMinutes,
      kwh: loopCharges.reduce((sum, c) => sum + c.kwh, 0),
      cost: loopCharges.reduce((sum, c) => sum + (c.cost ?? 0), 0),
      unpricedCount: loopCharges.filter((c) => c.cost == null && !c.active).length,
      whPerMile: averageEfficiency(loopTrips),
      maxDistanceFromHomeMi: maxDist,
      trips: loopTrips,
      charges: loopCharges,
      events,
      days,
      route: combinedRoute(loopTrips),
      memberIds: [...loopTrips.map((t) => t.id), ...loopCharges.map((c) => c.id)],
    });
  }

  return out;
}

function readOverrides(vehicleId: string | null | undefined): RoadTripOverrides {
  if (!vehicleId || typeof window === 'undefined') return EMPTY_OVERRIDES;
  try {
    const raw = window.localStorage.getItem(`${STORAGE_PREFIX}${vehicleId}`);
    if (!raw) return EMPTY_OVERRIDES;
    const parsed = JSON.parse(raw) as Partial<RoadTripOverrides> | null;
    return {
      names: parsed?.names && typeof parsed.names === 'object' ? parsed.names : {},
      splitIds: Array.isArray(parsed?.splitIds) ? parsed.splitIds.filter((v): v is string => typeof v === 'string') : [],
    };
  } catch {
    return EMPTY_OVERRIDES;
  }
}

function writeOverrides(vehicleId: string, next: RoadTripOverrides) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(`${STORAGE_PREFIX}${vehicleId}`, JSON.stringify(next));
    window.dispatchEvent(new CustomEvent(OVERRIDE_EVENT, { detail: vehicleId }));
  } catch {
    // Ignore storage quota errors in restricted/private browsing contexts.
  }
}

/** Per-vehicle Rename and Split controls for inferred road trips. */
export function useRoadTripOverrides(vehicleId: string | null | undefined) {
  const [overrides, setOverrides] = React.useState<RoadTripOverrides>(() => readOverrides(vehicleId));

  React.useEffect(() => {
    setOverrides(readOverrides(vehicleId));
    if (typeof window === 'undefined' || !vehicleId) return undefined;
    const sync = (event: Event) => {
      const detail = (event as CustomEvent<string>).detail;
      if (!detail || detail === vehicleId) setOverrides(readOverrides(vehicleId));
    };
    window.addEventListener(OVERRIDE_EVENT, sync);
    window.addEventListener('storage', sync);
    return () => {
      window.removeEventListener(OVERRIDE_EVENT, sync);
      window.removeEventListener('storage', sync);
    };
  }, [vehicleId]);

  const rename = React.useCallback((id: string, name: string) => {
    if (!vehicleId) return;
    const current = readOverrides(vehicleId);
    const trimmed = name.trim();
    const names = { ...current.names };
    if (trimmed) names[id] = trimmed;
    else delete names[id];
    const next = { ...current, names };
    setOverrides(next);
    writeOverrides(vehicleId, next);
  }, [vehicleId]);

  const split = React.useCallback((id: string) => {
    if (!vehicleId) return;
    const current = readOverrides(vehicleId);
    if (current.splitIds.includes(id)) return;
    const next = { ...current, splitIds: [...current.splitIds, id] };
    setOverrides(next);
    writeOverrides(vehicleId, next);
  }, [vehicleId]);

  return { overrides, rename, split };
}

import type { ChargeSession, Place, Trip } from '@panda/types';
import { getAppDateParts } from '@panda/ui/lib/dateTime';
import { chargeKind } from '../../charging/detail/chargeDetailModel';
import { chargePlace, savedPlaceAt, shortPlace, tripPlace, tripTitleOf, type PlaceLabel } from '../../../lib/placeNames';

export { shortPlace, tripPlace, type PlaceLabel };

/** Trips shorter than this have coarse battery readings and stay out of efficiency averages. */
export const SHORT_TRIP_MI = 3;
/** Parked time shows in the timeline from this many hours. */
export const PARKED_MIN_HOURS = 10;
/** An odometer move between trips shows from this many miles. */
export const ODOMETER_GAP_MIN_MI = 1;
/** Odometer moves at or under this many miles are reading noise. */
const ODOMETER_NOISE_MI = 0.3;
/** Parked periods shorter than this are not parked time. */
const PARKED_TRACK_HOURS = 3;

export type ActivityFilter = 'all' | 'trips' | 'charges';
export type ActivityPeriod = '7' | '30' | 'all';
export type PlaceCategory = 'home' | 'office' | 'public';
export type RouteCoordinate = [number, number];

export const CATEGORY_LABEL: Record<PlaceCategory, string> = { home: 'Home', office: 'Office', public: 'Other' };

interface Timed {
  id: string;
  start: Date;
  end: Date;
  /** App-timezone calendar day of `start`, YYYY-MM-DD. */
  day: string;
}

export interface ActivityTrip extends Timed {
  kind: 'trip';
  miles: number;
  minutes: number;
  socStart: number | null;
  socEnd: number | null;
  whPerMile: number | null;
  from: PlaceLabel;
  to: PlaceLabel;
  odometerStart: number | null;
  odometerEnd: number | null;
  route: RouteCoordinate[] | null;
  trip: Trip;
}

export interface ActivityCharge extends Timed {
  kind: 'charge';
  kwh: number;
  socStart: number | null;
  socEnd: number | null;
  dc: boolean;
  /** Place key and display name. Charges with the same place share the tag. */
  place: string;
  placeApproximate: boolean;
  savedPlace: Place | null;
  category: PlaceCategory;
  /** Cost in the session currency; 0 is free; null is not known yet. */
  cost: number | null;
  active: boolean;
  session: ChargeSession;
}

export interface ActivityParked extends Timed {
  kind: 'parked';
  hours: number;
  /** Battery percent lost while parked; null when a reading is missing. */
  loss: number | null;
}

export interface ActivityGap extends Timed {
  kind: 'gap';
  miles: number;
}

export interface RoadTripDayBreakdown {
  day: string;
  miles: number;
  kwh: number;
  cost: number;
  unpricedCount: number;
  events: ActivityEvent[];
}

export interface ActivityRoadTrip extends Timed {
  kind: 'road_trip';
  name: string;
  defaultName: string;
  inferenceReason: string;
  miles: number;
  minutes: number;
  kwh: number;
  cost: number;
  unpricedCount: number;
  whPerMile: number | null;
  maxDistanceFromHomeMi: number;
  trips: ActivityTrip[];
  charges: ActivityCharge[];
  events: ActivityEvent[];
  days: RoadTripDayBreakdown[];
  route: RouteCoordinate[];
  memberIds: string[];
}

export type ActivityItem = ActivityTrip | ActivityCharge | ActivityParked | ActivityGap | ActivityRoadTrip;
export type ActivityEvent = ActivityTrip | ActivityCharge | ActivityRoadTrip;

/* ---------------- days ---------------- */

export function dayKey(value: Date | string | number) {
  const parts = getAppDateParts(value);
  if (!parts) return '';
  return `${parts.year}-${String(parts.month).padStart(2, '0')}-${String(parts.day).padStart(2, '0')}`;
}

export function shiftDay(key: string, days: number) {
  const [y, m, d] = key.split('-').map(Number);
  const date = new Date(Date.UTC(y!, m! - 1, d! + days, 12));
  return date.toISOString().slice(0, 10);
}

/** First day of the period, inclusive. 'all' returns null. */
export function periodStartDay(period: ActivityPeriod, today: string) {
  return period === 'all' ? null : shiftDay(today, -(Number(period) - 1));
}

export function daysBetween(from: string, to: string) {
  const days: string[] = [];
  for (let day = from; day <= to && days.length < 4000; day = shiftDay(day, 1)) days.push(day);
  return days;
}

/* ---------------- places ---------------- */

export function tripTitle(trip: Pick<ActivityTrip, 'from' | 'to'>) {
  return tripTitleOf(trip.from, trip.to);
}

export function chargeTitle(charge: Pick<ActivityCharge, 'dc' | 'place'>) {
  return charge.dc ? `DC fast · ${shortPlace(charge.place)}` : shortPlace(charge.place);
}

function matchChargePlace(session: ChargeSession, places: Place[]) {
  const named = session.location_name?.trim().toLocaleLowerCase();
  const byCoordinates = savedPlaceAt(session.location_lat, session.location_lng, places);
  if (byCoordinates) return byCoordinates;
  return named ? places.find((place) => place.name.trim().toLocaleLowerCase() === named) ?? null : null;
}

export function placeCategory(name: string, savedPlace: Place | null): PlaceCategory {
  if (savedPlace?.is_home) return 'home';
  if (savedPlace?.is_work) return 'office';
  const key = name.trim().toLocaleLowerCase();
  if (!savedPlace && key === 'home') return 'home';
  if (!savedPlace && key === 'office') return 'office';
  return 'public';
}

/* ---------------- cost ---------------- */

const FREE_METHODS = new Set(['user_free', 'network_free', 'rivian_free_session']);

export function chargeCost(session: ChargeSession): number | null {
  if (session.ended_at == null) return null;
  if (session.cost_method && FREE_METHODS.has(session.cost_method)) return 0;
  if (session.cost_usd != null && Number.isFinite(session.cost_usd)) return session.cost_usd;
  return null;
}

/* ---------------- build ---------------- */

function finite(value: number | null | undefined) {
  return value != null && Number.isFinite(value) ? value : null;
}

export function toActivityTrip(trip: Trip, places: Place[], route: RouteCoordinate[] | null = null): ActivityTrip | null {
  const start = new Date(trip.started_at);
  if (Number.isNaN(start.getTime())) return null;
  const minutes = Math.max(0, trip.duration_min || 0);
  const end = trip.ended_at ? new Date(trip.ended_at) : new Date(start.getTime() + minutes * 60_000);
  return {
    kind: 'trip', id: trip.id, start, end, day: dayKey(start),
    miles: Math.max(0, trip.distance_mi || 0),
    minutes: minutes || Math.max(0, (end.getTime() - start.getTime()) / 60_000),
    socStart: finite(trip.soc_start), socEnd: finite(trip.soc_end),
    whPerMile: finite(trip.efficiency_wh_mi) && trip.efficiency_wh_mi! > 0 ? trip.efficiency_wh_mi : null,
    from: tripPlace(trip, 'start', places), to: tripPlace(trip, 'end', places),
    odometerStart: finite(trip.start_odometer_mi), odometerEnd: finite(trip.end_odometer_mi),
    route: route && route.length > 1 ? route : null,
    trip,
  };
}

export function toActivityCharge(session: ChargeSession, places: Place[]): ActivityCharge | null {
  const start = new Date(session.started_at);
  if (Number.isNaN(start.getTime())) return null;
  const end = session.ended_at ? new Date(session.ended_at) : start;
  const savedPlace = session.location_override_mode === 'none' ? null : matchChargePlace(session, places);
  const place = chargePlace(session, savedPlace);
  return {
    kind: 'charge', id: session.id, start, end, day: dayKey(start),
    kwh: Math.max(0, session.energy_added_kwh ?? 0),
    socStart: finite(session.soc_start), socEnd: finite(session.soc_end),
    dc: chargeKind(session) === 'dc',
    place: place.name, placeApproximate: place.approximate, savedPlace,
    category: placeCategory(place.name, savedPlace),
    cost: chargeCost(session), active: session.ended_at == null,
    session,
  };
}

/** Parked periods between consecutive trips and charges, with the battery loss. */
export function parkedPeriods(events: Array<ActivityTrip | ActivityCharge>): ActivityParked[] {
  const sorted = [...events].sort((a, b) => a.start.getTime() - b.start.getTime());
  const out: ActivityParked[] = [];
  let previous: ActivityTrip | ActivityCharge | null = null;
  for (const event of sorted) {
    if (previous && previous.end.getTime() < event.start.getTime()) {
      const hours = (event.start.getTime() - previous.end.getTime()) / 3_600_000;
      if (hours >= PARKED_TRACK_HOURS) {
        const loss = previous.socEnd != null && event.socStart != null ? Math.round((previous.socEnd - event.socStart) * 10) / 10 : null;
        out.push({ kind: 'parked', id: `parked-${previous.id}`, start: previous.end, end: event.start, day: dayKey(previous.end), hours, loss });
      }
    }
    if (!previous || event.end.getTime() > previous.end.getTime()) previous = event;
  }
  return out;
}

/** Odometer moves between consecutive trips that no trip covers. */
export function odometerGaps(trips: ActivityTrip[]): ActivityGap[] {
  const sorted = [...trips].sort((a, b) => a.start.getTime() - b.start.getTime());
  const out: ActivityGap[] = [];
  for (let index = 1; index < sorted.length; index += 1) {
    const previous = sorted[index - 1]!;
    const trip = sorted[index]!;
    if (previous.odometerEnd == null || trip.odometerStart == null) continue;
    const miles = trip.odometerStart - previous.odometerEnd;
    if (miles > ODOMETER_NOISE_MI) {
      out.push({ kind: 'gap', id: `gap-${trip.id}`, start: previous.end, end: trip.start, day: trip.day, miles });
    }
  }
  return out;
}

export interface ActivityData {
  trips: ActivityTrip[];
  charges: ActivityCharge[];
  parked: ActivityParked[];
  gaps: ActivityGap[];
  roadTrips?: ActivityRoadTrip[];
}

export function buildActivityData(trips: Trip[], sessions: ChargeSession[], places: Place[], routes: Map<string, RouteCoordinate[]> = new Map()): ActivityData {
  const activityTrips = trips.flatMap((trip) => toActivityTrip(trip, places, routes.get(trip.id) ?? null) ?? []);
  const activityCharges = sessions.flatMap((session) => toActivityCharge(session, places) ?? []);
  return {
    trips: activityTrips,
    charges: activityCharges,
    parked: parkedPeriods([...activityTrips, ...activityCharges]),
    gaps: odometerGaps(activityTrips),
  };
}

export interface FeedOptions {
  filter: ActivityFilter;
  period: ActivityPeriod;
  place: string | null;
  today: string;
}

function inPeriod(item: Timed, from: string | null, today: string) {
  return (from == null || item.day >= from) && item.day <= today;
}

/** The timeline for the filters, newest first. */
export function activityFeed(data: ActivityData, { filter, period, place, today }: FeedOptions): ActivityItem[] {
  const from = periodStartDay(period, today);
  if (place) return sortNewest(data.charges.filter((charge) => samePlace(charge.place, place) && inPeriod(charge, from, today)));
  if (filter === 'charges') return sortNewest(data.charges.filter((charge) => inPeriod(charge, from, today)));

  const roadTrips = (data.roadTrips ?? []).filter((rt) => inPeriod(rt, from, today));
  const bundledIds = new Set(roadTrips.flatMap((rt) => rt.memberIds));
  const insideRoadTrip = (item: Timed) => roadTrips.some(
    (rt) => item.start.getTime() >= rt.start.getTime() && item.end.getTime() <= rt.end.getTime(),
  );

  let items: ActivityItem[] = [...roadTrips];
  items = items.concat(
    data.trips.filter((trip) => !bundledIds.has(trip.id) && inPeriod(trip, from, today)),
  );
  if (filter === 'all') {
    items = items.concat(
      data.charges.filter((charge) => !bundledIds.has(charge.id) && inPeriod(charge, from, today)),
      data.parked.filter((parked) => parked.hours >= PARKED_MIN_HOURS && inPeriod(parked, from, today) && !insideRoadTrip(parked)),
      data.gaps.filter((gap) => gap.miles >= ODOMETER_GAP_MIN_MI && inPeriod(gap, from, today) && !insideRoadTrip(gap)),
    );
  }
  return sortNewest(items);
}

/** A place filter matches with or without the "Near " mark (Insights links use the short name). */
function samePlace(name: string, filter: string) {
  return shortPlace(name) === shortPlace(filter);
}

function sortNewest<T extends Timed>(items: T[]) {
  return items.sort((a, b) => b.start.getTime() - a.start.getTime());
}

export interface DayGroup {
  day: string;
  items: ActivityItem[];
  miles: number;
  kwh: number;
}

export function groupByDay(items: ActivityItem[]): DayGroup[] {
  const groups = new Map<string, ActivityItem[]>();
  for (const item of items) groups.set(item.day, [...(groups.get(item.day) ?? []), item]);
  return [...groups.keys()].sort().reverse().map((day) => {
    const list = groups.get(day)!;
    return {
      day, items: list,
      miles: list.reduce((sum, item) => sum + (item.kind === 'trip' || item.kind === 'road_trip' ? item.miles : 0), 0),
      kwh: list.reduce((sum, item) => sum + (item.kind === 'charge' || item.kind === 'road_trip' ? item.kwh : 0), 0),
    };
  });
}

/** Trips, road trips and charges only, in timeline order: the newer/older order of the detail. */
export function detailOrder(items: ActivityItem[]): ActivityEvent[] {
  return items.filter((item): item is ActivityEvent => item.kind === 'trip' || item.kind === 'charge' || item.kind === 'road_trip');
}

/** Distance-weighted efficiency of trips that are 3 mi or longer. */
export function averageEfficiency(trips: ActivityTrip[]) {
  const measured = trips.filter((trip) => trip.whPerMile != null && trip.miles >= SHORT_TRIP_MI);
  const miles = measured.reduce((sum, trip) => sum + trip.miles, 0);
  return miles ? measured.reduce((sum, trip) => sum + trip.whPerMile! * trip.miles, 0) / miles : null;
}

export interface TripSummary {
  kind: 'trips';
  miles: number;
  count: number;
  days: string[];
  whPerMile: number | null;
  averageMiles: number | null;
  driveHours: number;
  perDay: Array<{ day: string; miles: number }>;
}

export interface ChargeSummary {
  kind: 'charges';
  kwh: number;
  count: number;
  days: string[];
  cost: number;
  homePercent: number;
  unpriced: number;
  perDay: Array<{ day: string; home: number; office: number; public: number }>;
}

export function summarize(data: ActivityData, { filter, period, place, today }: FeedOptions): TripSummary | ChargeSummary {
  const from = periodStartDay(period, today);
  const first = from ?? [...data.trips, ...data.charges].reduce((min, item) => (item.day && item.day < min ? item.day : min), today);
  const days = daysBetween(first, today);
  if (filter === 'charges' || place) {
    const charges = data.charges.filter((charge) => inPeriod(charge, from, today) && (!place || samePlace(charge.place, place)));
    const kwh = charges.reduce((sum, charge) => sum + charge.kwh, 0);
    const home = charges.filter((charge) => charge.category === 'home').reduce((sum, charge) => sum + charge.kwh, 0);
    return {
      kind: 'charges', kwh, count: charges.length, days,
      cost: charges.reduce((sum, charge) => sum + (charge.cost ?? 0), 0),
      homePercent: kwh ? (home / kwh) * 100 : 0,
      unpriced: charges.filter((charge) => charge.cost == null && !charge.active).length,
      perDay: days.map((day) => {
        const list = charges.filter((charge) => charge.day === day);
        const by = (category: PlaceCategory) => list.filter((charge) => charge.category === category).reduce((sum, charge) => sum + charge.kwh, 0);
        return { day, home: by('home'), office: by('office'), public: by('public') };
      }),
    };
  }
  const trips = data.trips.filter((trip) => inPeriod(trip, from, today));
  const miles = trips.reduce((sum, trip) => sum + trip.miles, 0);
  return {
    kind: 'trips', miles, count: trips.length, days,
    whPerMile: averageEfficiency(trips),
    averageMiles: trips.length ? miles / trips.length : null,
    driveHours: trips.reduce((sum, trip) => sum + trip.minutes, 0) / 60,
    perDay: days.map((day) => ({ day, miles: trips.filter((trip) => trip.day === day).reduce((sum, trip) => sum + trip.miles, 0) })),
  };
}

/* ---------------- format ---------------- */

export const n0 = (value: number) => Math.round(value).toLocaleString('en-US');
export const n1 = (value: number) => (Math.round(value * 10) / 10).toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
export const usd = (value: number) => `$${value.toFixed(2)}`;
export function duration(minutes: number) {
  return minutes >= 60 ? `${Math.floor(minutes / 60)} h ${Math.round(minutes % 60)} min` : `${Math.round(minutes)} min`;
}

/** Projects a route onto a w × h box with padding, for glyphs. */
export function projectRoute(route: RouteCoordinate[] | null, width: number, height: number, pad: number) {
  if (!route || route.length < 2) return null;
  const k = Math.cos((route[0]![1] * Math.PI) / 180);
  const xs = route.map((point) => point[0] * k);
  const ys = route.map((point) => point[1]);
  const minX = Math.min(...xs); const maxX = Math.max(...xs);
  const minY = Math.min(...ys); const maxY = Math.max(...ys);
  const scale = Math.min((width - 2 * pad) / ((maxX - minX) || 1e-6), (height - 2 * pad) / ((maxY - minY) || 1e-6));
  const ox = (width - (maxX - minX) * scale) / 2;
  const oy = (height - (maxY - minY) * scale) / 2;
  return route.map((_, index) => [ox + (xs[index]! - minX) * scale, height - (oy + (ys[index]! - minY) * scale)] as [number, number]);
}

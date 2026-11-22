import type { ChargeCurvePoint, ChargeSession, PhantomDrainPeriod, Place, Trip } from '@panda/types';
import { chargeKind } from '../charging/detail/chargeDetailModel';
import { chargePlace, shortPlace, tripPlace, tripTitleOf } from '../../lib/placeNames';

export { shortPlace };

/* ------------------------------------------------------------------ calendar days */

/** A calendar day in the app timezone, "YYYY-MM-DD". */
export type DayKey = string;

const keyFormatters = new Map<string, Intl.DateTimeFormat>();
function keyFormatter(timezone: string) {
  let formatter = keyFormatters.get(timezone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat('en-CA', { timeZone: timezone, year: 'numeric', month: '2-digit', day: '2-digit' });
    keyFormatters.set(timezone, formatter);
  }
  return formatter;
}

export function dayKeyOf(value: string | number | Date, timezone: string): DayKey {
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  return keyFormatter(timezone).format(date);
}

function keyParts(key: DayKey) {
  const [y, m, d] = key.split('-').map(Number);
  return { y: y ?? 1970, m: m ?? 1, d: d ?? 1 };
}

export function addDays(key: DayKey, days: number): DayKey {
  const { y, m, d } = keyParts(key);
  const date = new Date(Date.UTC(y, m - 1, d + days));
  return date.toISOString().slice(0, 10);
}

/** The Monday of the week that holds the day. */
export function mondayOf(key: DayKey): DayKey {
  const { y, m, d } = keyParts(key);
  const weekday = (new Date(Date.UTC(y, m - 1, d)).getUTCDay() + 6) % 7;
  return addDays(key, -weekday);
}

export function daysBetween(from: DayKey, to: DayKey) {
  const a = keyParts(from); const b = keyParts(to);
  return Math.round((Date.UTC(b.y, b.m - 1, b.d) - Date.UTC(a.y, a.m - 1, a.d)) / 864e5);
}

/** Every day from `from` to `to`, both included. */
export function dayRange(from: DayKey, to: DayKey): DayKey[] {
  const out: DayKey[] = [];
  if (!from || !to || from > to) return out;
  for (let day = from; day <= to; day = addDays(day, 1)) out.push(day);
  return out;
}

/** The Mondays of every week from the first day to today. */
export function weeksFrom(first: DayKey, today: DayKey): DayKey[] {
  const out: DayKey[] = [];
  if (!first || !today || first > today) return out;
  for (let week = mondayOf(first); week <= today; week = addDays(week, 7)) out.push(week);
  return out;
}

export const inWeek = (day: DayKey, week: DayKey) => day >= week && day < addDays(week, 7);

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const LONG_MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** "Oct 6" */
export function monthDay(key: DayKey) {
  const { m, d } = keyParts(key);
  return `${MONTHS[m - 1]} ${d}`;
}

/** "Mon, Oct 6" */
export function weekdayMonthDay(key: DayKey) {
  const { y, m, d } = keyParts(key);
  return `${WEEKDAYS[new Date(Date.UTC(y, m - 1, d)).getUTCDay()]}, ${MONTHS[m - 1]} ${d}`;
}

/** "October 2026" from "2026-10" */
export function monthName(month: string) {
  const [y, m] = month.split('-').map(Number);
  return `${LONG_MONTHS[(m ?? 1) - 1]} ${y}`;
}

/* ------------------------------------------------------------------ numbers */

export const sum = <T,>(items: readonly T[], value: (item: T) => number | null | undefined) =>
  items.reduce((total, item) => total + (value(item) ?? 0), 0);

export const n0 = (v: number) => Math.round(v).toLocaleString('en-US');
export const n1 = (v: number) => (Math.round(v * 10) / 10).toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
export const n2 = (v: number) => (Math.round(v * 100) / 100).toFixed(2);
export const usd = (v: number) => `$${v.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;

/** A padded [low, high] domain around the values. */
export function paddedDomain(values: readonly number[], step?: number): [number, number] {
  if (!values.length) return [0, 1];
  const lo = Math.min(...values); const hi = Math.max(...values);
  const pad = (hi - lo) * 0.35 || Math.abs(hi) * 0.1 || 1;
  const a = lo - pad; const b = hi + pad;
  return step ? [Math.floor(a / step) * step, Math.ceil(b / step) * step] : [a, b];
}

/** Round axis ticks: 1, 2 or 5 times a power of ten, covering the domain. */
export function niceTicks(lo: number, hi: number, count = 3): number[] {
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) return [];
  if (hi <= lo) hi = lo + 1;
  const raw = (hi - lo) / Math.max(1, count - 1);
  const power = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map(f => f * power).find(s => s >= raw) ?? raw;
  const start = Math.floor(lo / step) * step;
  const end = Math.ceil(hi / step) * step;
  const out: number[] = [];
  for (let v = start; v <= end + step / 2; v += step) out.push(Number(v.toFixed(10)));
  return out;
}

/* ------------------------------------------------------------------ places and categories */

export type Category = 'home' | 'office' | 'other';
export const CATEGORIES: readonly Category[] = ['home', 'office', 'other'];
export const CATEGORY_LABEL: Record<Category, string> = { home: 'Home', office: 'Office', other: 'Other' };
export const CATEGORY_COLOR: Record<Category, string> = { home: 'var(--ri-home)', office: 'var(--ri-office)', other: 'var(--ri-other)' };

export function placeCategory(place: Place | null | undefined): Category {
  if (place?.is_home) return 'home';
  if (place?.is_work) return 'office';
  return 'other';
}

function metersBetween(a: { lat: number; lng: number }, b: { lat: number; lng: number }) {
  const rad = Math.PI / 180;
  const dLat = (b.lat - a.lat) * rad;
  const dLng = (b.lng - a.lng) * rad;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * rad) * Math.cos(b.lat * rad) * Math.sin(dLng / 2) ** 2;
  return 2 * 6_371_000 * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** The saved place that holds the charge: the nearest geofence that contains its location, else a place with the same name. */
export function savedPlaceFor(session: Pick<ChargeSession, 'location_lat' | 'location_lng' | 'location_name'>, places: readonly Place[]): Place | null {
  const lat = session.location_lat; const lng = session.location_lng;
  if (lat != null && lng != null) {
    let best: Place | null = null; let bestMeters = Infinity;
    for (const place of places) {
      const meters = metersBetween({ lat, lng }, { lat: place.latitude, lng: place.longitude });
      if (meters <= Math.max(place.radius_m, 50) && meters < bestMeters) { best = place; bestMeters = meters; }
    }
    if (best) return best;
    return null;
  }
  const name = session.location_name?.trim().toLowerCase();
  return name ? places.find(place => place.name.trim().toLowerCase() === name) ?? null : null;
}

/* ------------------------------------------------------------------ rows */

export interface ChargeRow {
  id: string;
  day: DayKey;
  start: number;
  end: number;
  kwh: number;
  /** What the charge cost; null when no price is known yet. */
  cost: number | null;
  dc: boolean;
  category: Category;
  placeKey: string;
  placeName: string;
  place: Place | null;
  socStart: number | null;
  socEnd: number | null;
  session: ChargeSession;
}

export interface TripRow {
  id: string;
  day: DayKey;
  start: number;
  miles: number;
  /** Wh/mi; null when the trip has no energy reading. */
  whMi: number | null;
  tempC: number | null;
  socStart: number | null;
  socEnd: number | null;
  odoStart: number | null;
  odoEnd: number | null;
  title: string;
}

export function chargeRows(sessions: readonly ChargeSession[], places: readonly Place[], timezone: string): ChargeRow[] {
  return sessions
    .filter(session => session.ended_at != null)
    .map(session => {
      // "none" means the owner tagged the place Other: no saved place, whatever the location.
      const place = session.location_override_mode === 'none' ? null : savedPlaceFor(session, places);
      const name = chargePlace(session, place).name;
      // Group by where the charge was, not by its display name: two chargers in one town stay two places,
      // as the place-tag API treats them.
      const where = session.location_name?.trim() || session.rivian_city?.trim() || name;
      const start = new Date(session.started_at).getTime();
      return {
        id: session.id,
        day: session.session_day_local || dayKeyOf(start, timezone),
        start,
        end: new Date(session.ended_at!).getTime(),
        kwh: Math.max(0, session.energy_added_kwh ?? 0),
        cost: session.cost_usd ?? null,
        dc: chargeKind(session) === 'dc',
        category: placeCategory(place),
        placeKey: place ? `place:${place.id}` : `name:${where.toLowerCase()}`,
        placeName: shortPlace(name),
        place,
        socStart: session.soc_start,
        socEnd: session.soc_end,
        session,
      };
    })
    .sort((a, b) => a.start - b.start);
}

/** The trip title with the shared place names: "Home → Ridgefield". */
export function tripTitle(trip: Trip, places: readonly Place[] = []) {
  return tripTitleOf(tripPlace(trip, 'start', places), tripPlace(trip, 'end', places));
}

export function tripRows(trips: readonly Trip[], timezone: string, places: readonly Place[] = []): TripRow[] {
  return trips
    .filter(trip => trip.ended_at != null && Number.isFinite(trip.distance_mi))
    .map(trip => {
      const start = new Date(trip.started_at).getTime();
      const whMi = trip.efficiency_wh_mi != null && trip.efficiency_wh_mi > 0 ? trip.efficiency_wh_mi
        : trip.energy_used_kwh != null && trip.energy_used_kwh > 0 && trip.distance_mi > 0 ? trip.energy_used_kwh * 1000 / trip.distance_mi : null;
      return {
        id: trip.id, day: dayKeyOf(start, timezone), start, miles: Math.max(0, trip.distance_mi), whMi,
        tempC: trip.outside_temp_c ?? null, socStart: trip.soc_start, socEnd: trip.soc_end,
        odoStart: trip.start_odometer_mi ?? null, odoEnd: trip.end_odometer_mi ?? null, title: tripTitle(trip, places),
      };
    })
    .sort((a, b) => a.start - b.start);
}

/** The first day with a trip or a charge. */
export function firstDay(trips: readonly TripRow[], charges: readonly ChargeRow[], today: DayKey): DayKey {
  const days = [trips[0]?.day, charges[0]?.day].filter((d): d is string => !!d);
  return days.length ? days.sort()[0]! : today;
}

/* ------------------------------------------------------------------ cost */

export interface PlaceSummary {
  category: Category;
  kwh: number;
  cost: number;
  unpriced: number;
  /** $/kWh over the priced charges; null without one. */
  perKwh: number | null;
}

export function byCategory(charges: readonly ChargeRow[]): PlaceSummary[] {
  return CATEGORIES.map(category => {
    const items = charges.filter(c => c.category === category);
    const priced = items.filter(c => c.cost != null);
    const pricedKwh = sum(priced, c => c.kwh);
    return {
      category, kwh: sum(items, c => c.kwh), cost: sum(priced, c => c.cost),
      unpriced: items.length - priced.length, perKwh: pricedKwh ? sum(priced, c => c.cost) / pricedKwh : null,
    };
  });
}

export interface PlaceGroup {
  key: string;
  name: string;
  place: Place | null;
  category: Category;
  charges: ChargeRow[];
  kwh: number;
  cost: number;
  unpriced: number;
}

export function placeGroups(charges: readonly ChargeRow[]): PlaceGroup[] {
  const map = new Map<string, ChargeRow[]>();
  for (const charge of charges) map.set(charge.placeKey, [...(map.get(charge.placeKey) ?? []), charge]);
  return [...map.entries()].map(([key, items]) => ({
    key, name: items[0]!.placeName, place: items[0]!.place, category: items[0]!.category, charges: items,
    kwh: sum(items, c => c.kwh), cost: sum(items, c => c.cost), unpriced: items.filter(c => c.cost == null).length,
  })).sort((a, b) => b.kwh - a.kwh || a.name.localeCompare(b.name));
}

export interface GasSettings { mpg: number; price: number }
export const DEFAULT_GAS: GasSettings = { mpg: 25, price: 4.5 };

export function gasCost(miles: number, gas: GasSettings) {
  return gas.mpg > 0 ? miles / gas.mpg * gas.price : 0;
}

/* ------------------------------------------------------------------ charging */

/** Minutes from the first sample to the first time the battery reached its top level. Null without a rise. */
export function activeMinutes(curve: readonly ChargeCurvePoint[] | undefined): number | null {
  const samples = (curve ?? []).filter(p => p.minutes_elapsed != null && Number.isFinite(p.soc_pct));
  if (samples.length < 2) return null;
  let top = 0;
  samples.forEach((point, index) => { if (point.soc_pct > samples[top]!.soc_pct + 0.05) top = index; });
  const minutes = samples[top]!.minutes_elapsed! - samples[0]!.minutes_elapsed!;
  return minutes > 1 ? minutes : null;
}

export interface SpeedGroup { label: string; color: string; kw: number; count: number }

/** Average charging speed per group: energy added ÷ the time until the battery stopped rising. */
export function speedGroups(charges: readonly ChargeRow[], curves: ReadonlyMap<string, readonly ChargeCurvePoint[]>): SpeedGroup[] {
  const groups: Array<[string, ChargeRow[], string]> = [
    ['Home', charges.filter(c => c.category === 'home' && !c.dc), CATEGORY_COLOR.home],
    ['Office', charges.filter(c => c.category === 'office' && !c.dc), CATEGORY_COLOR.office],
    ['Other AC', charges.filter(c => c.category === 'other' && !c.dc), CATEGORY_COLOR.other],
    ['DC fast', charges.filter(c => c.dc), CATEGORY_COLOR.other],
  ];
  return groups.map(([label, items, color]) => {
    const timed = items.map(c => [c, activeMinutes(curves.get(c.id))] as const).filter(([, m]) => m != null);
    const hours = sum(timed, ([, m]) => m) / 60;
    return { label, color, kw: hours ? sum(timed, ([c]) => c.kwh) / hours : 0, count: items.length };
  }).filter(group => group.count > 0);
}

/* ------------------------------------------------------------------ battery */

export { capacityByDay, type CapacityDay } from '../../lib/batteryHealth';

export interface BandDay { index: number; day: DayKey; lo: number; hi: number }

/** The lowest and highest battery level each day, from trip and charge readings. */
export function batteryBand(days: readonly DayKey[], trips: readonly TripRow[], charges: readonly ChargeRow[]): BandDay[] {
  const map = new Map<DayKey, number[]>();
  const add = (day: DayKey, value: number | null) => { if (value != null && Number.isFinite(value)) map.set(day, [...(map.get(day) ?? []), value]); };
  trips.forEach(t => { add(t.day, t.socStart); add(t.day, t.socEnd); });
  charges.forEach(c => { add(c.day, c.socStart); add(c.day, c.socEnd); });
  return days.flatMap((day, index) => {
    const values = map.get(day);
    return values?.length ? [{ index, day, lo: Math.min(...values), hi: Math.max(...values) }] : [];
  });
}

export interface DrainDay { hours: number; loss: number; count: number }

/** Battery lost while parked, grouped by the day parking started. Only validated periods count. */
export function drainByDay(periods: readonly PhantomDrainPeriod[], timezone: string): Map<DayKey, DrainDay> {
  const map = new Map<DayKey, DrainDay>();
  for (const period of periods) {
    if (period.validation_status !== 'validated' || !period.period_start || period.duration_hours == null) continue;
    const loss = period.soc_lost_pct ?? (period.soc_start != null && period.soc_end != null ? period.soc_start - period.soc_end : null);
    if (loss == null) continue;
    const day = dayKeyOf(period.period_start, timezone);
    const current = map.get(day) ?? { hours: 0, loss: 0, count: 0 };
    map.set(day, { hours: current.hours + period.duration_hours, loss: current.loss + Math.max(0, loss), count: current.count + 1 });
  }
  return map;
}

/* ------------------------------------------------------------------ efficiency */

/** Trips under this distance have coarse battery readings and are left out of efficiency. */
export const SHORT_TRIP_MI = 3;

export const measured = (trips: readonly TripRow[]) => trips.filter(t => t.whMi != null && t.miles >= SHORT_TRIP_MI);

/** Distance-weighted Wh/mi over measured trips; null without one. */
export function averageWhMi(trips: readonly TripRow[]): number | null {
  const items = measured(trips);
  const miles = sum(items, t => t.miles);
  return miles ? sum(items, t => t.whMi! * t.miles) / miles : null;
}

export const TRIP_LENGTH_BUCKETS: ReadonlyArray<readonly [string, number, number]> = [
  ['3–10 mi', 3, 10], ['10–20 mi', 10, 20], ['20–50 mi', 20, 50], ['50 mi +', 50, Infinity],
];

export const cToF = (c: number) => c * 9 / 5 + 32;

/* ------------------------------------------------------------------ lifetime */

export interface OdometerGap { start: number; end: number; day: DayKey; miles: number }

/** Odometer miles between two trips: the next trip starts further than the last one ended. */
export function odometerGaps(trips: readonly TripRow[], timezone: string, minMiles = 0.3): OdometerGap[] {
  const gaps: OdometerGap[] = [];
  trips.forEach((trip, index) => {
    const previous = trips[index - 1];
    if (!previous || previous.odoEnd == null || trip.odoStart == null) return;
    const miles = trip.odoStart - previous.odoEnd;
    if (miles > minMiles) gaps.push({ start: previous.start, end: trip.start, day: dayKeyOf(trip.start, timezone), miles });
  });
  return gaps;
}

export interface MonthRow { month: string; trips: number; miles: number; kwh: number }

export function byMonth(trips: readonly TripRow[], charges: readonly ChargeRow[]): MonthRow[] {
  const map = new Map<string, MonthRow>();
  const row = (month: string) => {
    let value = map.get(month);
    if (!value) { value = { month, trips: 0, miles: 0, kwh: 0 }; map.set(month, value); }
    return value;
  };
  trips.forEach(t => { const r = row(t.day.slice(0, 7)); r.trips += 1; r.miles += t.miles; });
  charges.forEach(c => { row(c.day.slice(0, 7)).kwh += c.kwh; });
  return [...map.values()].sort((a, b) => a.month.localeCompare(b.month));
}

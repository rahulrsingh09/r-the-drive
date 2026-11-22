import { isVehicleCharging } from '@panda/types';
import type { ChargeSession, PhantomDrainPeriod, Place, TrackPoint, Trip, Vehicle, VehicleStatus } from '@panda/types';
import {
  formatAppDate, formatAppDateTime, formatAppTime, getAppDateParts, shiftAppCalendarDays, startOfAppDay,
} from '@panda/ui/lib/dateTime';
import { chargeKind } from '../charging/detail/chargeDetailModel';
import { chargePlace } from '../../lib/placeNames';
import { measuredEfficiencyWhMi, SHORT_TRIP_MILES } from '../r-experience/efficiencyUnit';
import { vehicleLockReading } from '../r-experience/vehicleLockReading';

/** What the vehicle does now. Stale data keeps the last known state; the header shows the age. */
export type NowActivity = 'waiting' | 'charging' | 'driving' | 'parked';

export function nowActivity(status: VehicleStatus | null | undefined): NowActivity {
  if (!status) return 'waiting';
  if (isVehicleCharging(status)) return 'charging';
  // The power state stays "go" or "drive" while the car is on in park, so the gear decides.
  const gear = status.gear_status?.toLowerCase() ?? '';
  if (gear.includes('park')) return 'parked';
  if (/drive|reverse|neutral/.test(gear)) return 'driving';
  if ((status.speed_mph ?? 0) > 0) return 'driving';
  return 'parked';
}

function metersBetween(a: { lat: number; lng: number }, b: { lat: number; lng: number }) {
  const rad = Math.PI / 180;
  const dLat = (b.lat - a.lat) * rad;
  const dLng = (b.lng - a.lng) * rad;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * rad) * Math.cos(b.lat * rad) * Math.sin(dLng / 2) ** 2;
  return 2 * 6_371_000 * Math.asin(Math.min(1, Math.sqrt(h)));
}

/** The saved place that contains the point, the nearest one first. */
export function placeAt(lat: number | null | undefined, lng: number | null | undefined, places: readonly Place[] | undefined) {
  if (lat == null || lng == null || !places?.length) return null;
  const hits = places
    .map(place => ({ place, meters: metersBetween({ lat, lng }, { lat: place.latitude, lng: place.longitude }) }))
    .filter(({ place, meters }) => meters <= Math.max(place.radius_m, 50))
    .sort((a, b) => a.meters - b.meters);
  return hits[0]?.place.name ?? null;
}

/**
 * Where the vehicle is: a saved place at the current position, else the end place of the latest
 * finished trip when that trip ended near the current position (or no position is known).
 */
export function nowPlace(status: VehicleStatus | null | undefined, places: readonly Place[] | undefined, latestTrip: Trip | undefined) {
  const saved = placeAt(status?.latitude, status?.longitude, places);
  if (saved) return saved;
  if (!latestTrip?.ended_at) return null;
  const name = latestTrip.end_place || null;
  if (!name) return null;
  if (status?.latitude == null || status.longitude == null || latestTrip.end_lat == null || latestTrip.end_lng == null) {
    return status?.latitude == null ? name : null;
  }
  return metersBetween({ lat: status.latitude, lng: status.longitude }, { lat: latestTrip.end_lat, lng: latestTrip.end_lng }) <= 250 ? name : null;
}

export function lockWord(status: VehicleStatus | null | undefined) {
  const locked = vehicleLockReading(status ?? null);
  return locked === null ? null : locked ? 'Locked' : 'Unlocked';
}

/** "Parked · Home · Locked". Parts that are not known are left out. */
export function parkedChip(status: VehicleStatus | null | undefined, place: string | null) {
  if (!status) return 'Waiting for the car';
  return ['Parked', place ? shortPlace(place) : null, lockWord(status)].filter(Boolean).join(' · ');
}

/** "Updated 7:42 PM" today, else "Updated Nov 6, 7:42 PM". Null without a reading. */
export function updatedLabel(status: VehicleStatus | null | undefined, now: number) {
  const at = status?.last_updated;
  if (!at || Number.isNaN(Date.parse(at))) return null;
  return `Updated ${dayKey(at) === dayKey(now) ? formatAppTime(at) : formatAppDateTime(at, { year: undefined })}`;
}

/** Model, trim and color from the vehicle record. Missing parts are left out. */
export function vehicleConfiguration(vehicle: Pick<Vehicle, 'model' | 'trim' | 'color'>) {
  const model = vehicle.model?.trim() || 'Rivian';
  const trim = vehicle.trim?.trim();
  return [trim && !trim.toLowerCase().startsWith(model.toLowerCase()) ? `${model} ${trim}` : trim || model, vehicle.color?.trim()]
    .filter(Boolean).join(' · ');
}

export function spinLabel(vehicle: Pick<Vehicle, 'model' | 'color'>) {
  const model = vehicle.model?.trim() || 'Rivian';
  const color = vehicle.color?.trim();
  return color ? `Your ${model}, ${color}. Use arrow keys to turn.` : `Your ${model}. Use arrow keys to turn.`;
}

export function vehicleName(vehicle: Pick<Vehicle, 'display_name' | 'model'>) {
  return vehicle.display_name?.trim() || vehicle.model?.trim() || 'Your Rivian';
}

/** Lowest and highest tire pressure in psi; null without a reading. */
export function tireRange(status: VehicleStatus | null | undefined) {
  const values = [status?.tire_fl_psi, status?.tire_fr_psi, status?.tire_rl_psi, status?.tire_rr_psi]
    .filter((value): value is number => value != null && Number.isFinite(value) && value > 0);
  if (!values.length) return status?.tire_min_psi != null ? { min: status.tire_min_psi, max: status.tire_min_psi } : null;
  return { min: Math.min(...values), max: Math.max(...values) };
}

export function tireText(range: { min: number; max: number } | null) {
  if (!range) return null;
  const low = Math.round(range.min);
  const high = Math.round(range.max);
  return low === high ? `${low}` : `${low}–${high}`;
}

// --- dates --------------------------------------------------------------------

/** App-calendar day key, "2026-11-07". */
export function dayKey(value: Date | string | number) {
  const parts = getAppDateParts(value);
  return parts ? `${parts.year}-${String(parts.month).padStart(2, '0')}-${String(parts.day).padStart(2, '0')}` : '';
}

/** Start of the app-calendar Monday on or before `value`. */
export function mondayOf(value: Date | string | number) {
  const start = startOfAppDay(value);
  const parts = getAppDateParts(start);
  if (!parts) return start;
  const weekday = new Date(Date.UTC(parts.year, parts.month - 1, parts.day)).getUTCDay();
  return startOfAppDay(shiftAppCalendarDays(start, -((weekday + 6) % 7)));
}

/** The Now windows: 30 app-calendar days that end today, and the week that starts on Monday. */
export function nowWindows(now: number) {
  const monthFrom = startOfAppDay(shiftAppCalendarDays(now, -29));
  const weekFrom = mondayOf(now);
  const from = weekFrom < monthFrom ? weekFrom : monthFrom;
  return { from: from.toISOString(), to: new Date(now).toISOString(), monthFrom, weekFrom };
}

/** "Sat, Nov 7". */
export function shortDay(value: Date | string | number) {
  return formatAppDate(value, { year: undefined, weekday: 'short' });
}

// --- trips and charges -------------------------------------------------------

export const shortPlace = (name: string) => name.replace(/^Near /, '');

/** "Kirkland → Home", or "Round trip · Home" when it starts and ends at one place. */
export function tripTitle(trip: Pick<Trip, 'start_place' | 'start_address' | 'end_place' | 'end_address'>) {
  const from = shortPlace(trip.start_place || trip.start_address || 'Start');
  const to = shortPlace(trip.end_place || trip.end_address || 'Destination');
  return from === to ? `Round trip · ${from}` : `${from} → ${to}`;
}

export function isShortTrip(trip: Pick<Trip, 'distance_mi'>) {
  return !(trip.distance_mi >= SHORT_TRIP_MILES);
}

export function isDcCharge(charge: ChargeSession) {
  return chargeKind(charge) === 'dc';
}

/** "DC fast · Ridgefield", else the place name. */
export function chargeTitle(charge: ChargeSession) {
  // The shared place-name rule: a coordinate or address label becomes the nearest town, as on Activity and Insights.
  const place = shortPlace(chargePlace(charge, null).name || 'Charge');
  return isDcCharge(charge) ? `DC fast · ${place}` : place;
}

export type ChargeCategory = 'home' | 'office' | 'other';
export const CATEGORY_LABEL: Record<ChargeCategory, string> = { home: 'Home', office: 'Office', other: 'Other' };
export const CATEGORY_ORDER: readonly ChargeCategory[] = ['home', 'office', 'other'];

/** Home and Office come from the saved place of the charge (home or work flag, else its name). Every other charge is Other. */
export function chargeCategory(charge: Pick<ChargeSession, 'location_name'>, places: readonly Place[] | undefined): ChargeCategory {
  const name = charge.location_name?.trim().toLowerCase();
  if (!name) return 'other';
  const place = places?.find(item => item.name.trim().toLowerCase() === name);
  if (place?.is_home) return 'home';
  if (place?.is_work) return 'office';
  if (name === 'home') return 'home';
  if (name === 'office' || name === 'work') return 'office';
  return 'other';
}

export const hasCost = (charge: Pick<ChargeSession, 'cost_usd'>) => charge.cost_usd != null && Number.isFinite(charge.cost_usd);

export const usd = (value: number) => `$${value.toFixed(2)}`;

/** "Add cost", "Free" or "$8.66". */
export function costText(charge: Pick<ChargeSession, 'cost_usd'>) {
  if (!hasCost(charge)) return 'Add cost';
  return charge.cost_usd === 0 ? 'Free' : usd(charge.cost_usd!);
}

export const sumBy = <T>(items: readonly T[], value: (item: T) => number | null | undefined) =>
  items.reduce((total, item) => total + (value(item) ?? 0), 0);

export interface WeekDay { key: string; label: string; title: string; miles: number; today: boolean }
export interface WeekSummary {
  from: Date;
  days: WeekDay[];
  miles: number;
  trips: number;
  efficiencyWhMi: number | null;
  chargedKwh: number;
  cost: number;
  unpriced: number;
}

/** Monday to Sunday of the current week: trip miles per day, efficiency, energy charged and known cost. */
export function weekSummary(trips: readonly Trip[], charges: readonly ChargeSession[], now: number): WeekSummary {
  const from = mondayOf(now);
  const today = dayKey(now);
  const days: WeekDay[] = Array.from({ length: 7 }, (_, index) => {
    const date = shiftAppCalendarDays(from, index);
    const key = dayKey(date);
    return { key, miles: 0, today: key === today, title: shortDay(date),
      label: formatAppDate(date, { year: undefined, month: undefined, day: undefined, weekday: 'narrow' }) };
  });
  const keys = new Set(days.map(day => day.key));
  const weekTrips = trips.filter(trip => keys.has(dayKey(trip.started_at)) && Number.isFinite(trip.distance_mi));
  for (const trip of weekTrips) days.find(day => day.key === dayKey(trip.started_at))!.miles += trip.distance_mi;
  const weekCharges = charges.filter(charge => keys.has(dayKey(charge.started_at)) && charge.ended_at != null);
  return {
    from, days, trips: weekTrips.length,
    miles: sumBy(weekTrips, trip => trip.distance_mi),
    efficiencyWhMi: measuredEfficiencyWhMi(weekTrips),
    chargedKwh: sumBy(weekCharges, charge => charge.energy_added_kwh),
    cost: sumBy(weekCharges, charge => (hasCost(charge) ? charge.cost_usd : 0)),
    unpriced: weekCharges.filter(charge => !hasCost(charge)).length,
  };
}

export interface CategorySpend { key: ChargeCategory; label: string; kwh: number; cost: number; unpriced: number }
export interface MonthSummary {
  cost: number;
  byCategory: CategorySpend[];
  efficiencyWhMi: number | null;
  weeks: Array<{ from: Date; whMi: number }>;
  drainDays: Array<{ key: string; title: string; lossPct: number; hours: number }>;
  /** Battery % lost per day parked, over the parked hours in the window; null without parked periods. */
  drainPerDay: number | null;
}

/** The last 30 days: charging cost by place, efficiency per week and loss while parked per day. */
export function monthSummary(
  trips: readonly Trip[], charges: readonly ChargeSession[], parks: readonly PhantomDrainPeriod[],
  places: readonly Place[] | undefined, now: number,
): MonthSummary {
  const from = startOfAppDay(shiftAppCalendarDays(now, -29));
  const inMonth = (value: string | null | undefined) => Boolean(value) && Date.parse(value!) >= from.getTime() && Date.parse(value!) <= now;
  const monthTrips = trips.filter(trip => inMonth(trip.started_at));
  const monthCharges = charges.filter(charge => inMonth(charge.started_at) && charge.ended_at != null);
  const byCategory = CATEGORY_ORDER.map(key => {
    const items = monthCharges.filter(charge => chargeCategory(charge, places) === key);
    return { key, label: CATEGORY_LABEL[key], kwh: sumBy(items, charge => charge.energy_added_kwh),
      cost: sumBy(items, charge => (hasCost(charge) ? charge.cost_usd : 0)), unpriced: items.filter(charge => !hasCost(charge)).length };
  });
  const weeks: MonthSummary['weeks'] = [];
  for (let week = mondayOf(from); week.getTime() <= now; week = startOfAppDay(shiftAppCalendarDays(week, 7))) {
    const end = startOfAppDay(shiftAppCalendarDays(week, 7)).getTime();
    const whMi = measuredEfficiencyWhMi(monthTrips.filter(trip => Date.parse(trip.started_at) >= week.getTime() && Date.parse(trip.started_at) < end));
    if (whMi != null) weeks.push({ from: week, whMi });
  }
  const validParks = parks.filter(park => park.period_start && inMonth(park.period_start) && park.validation_status === 'validated');
  const drainDays = Array.from({ length: 30 }, (_, index) => {
    const date = shiftAppCalendarDays(from, index);
    const key = dayKey(date);
    const dayParks = validParks.filter(park => dayKey(park.period_start!) === key);
    return { key, title: shortDay(date), lossPct: sumBy(dayParks, park => Math.max(0, park.soc_lost_pct ?? 0)), hours: sumBy(dayParks, park => park.duration_hours) };
  });
  const parkedHours = sumBy(validParks, park => park.duration_hours);
  return {
    cost: sumBy(byCategory, item => item.cost),
    byCategory,
    efficiencyWhMi: measuredEfficiencyWhMi(monthTrips),
    weeks,
    drainDays,
    drainPerDay: parkedHours > 0 ? sumBy(validParks, park => Math.max(0, park.soc_lost_pct ?? 0)) / (parkedHours / 24) : null,
  };
}

// --- numbers ------------------------------------------------------------------

export const n0 = (value: number) => Math.round(value).toLocaleString('en-US');
export const n1 = (value: number) => value.toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
export const n2 = (value: number) => value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** "17 min", "1 h 5 min". */
export function durationText(minutes: number) {
  if (!Number.isFinite(minutes) || minutes < 0) return '–';
  return minutes >= 60 ? `${Math.floor(minutes / 60)} h ${Math.round(minutes % 60)} min` : `${Math.max(0, Math.round(minutes))} min`;
}

/** SVG path for a trip route in a w × h box, keeping the map aspect. */
export function routePoints(points: readonly Pick<TrackPoint, 'lat' | 'lng'>[] | undefined, width: number, height: number, pad = 8) {
  const valid = (points ?? []).filter(point => Number.isFinite(point.lat) && Number.isFinite(point.lng));
  if (valid.length < 2) return null;
  const k = Math.cos(valid[0]!.lat * Math.PI / 180);
  const xs = valid.map(point => point.lng * k);
  const ys = valid.map(point => point.lat);
  const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const scale = Math.min((width - pad * 2) / ((maxX - minX) || 1e-6), (height - pad * 2) / ((maxY - minY) || 1e-6));
  const offsetX = (width - (maxX - minX) * scale) / 2;
  const offsetY = (height - (maxY - minY) * scale) / 2;
  return xs.map((x, index) => [offsetX + (x - minX) * scale, height - (offsetY + (ys[index]! - minY) * scale)] as const);
}

export function routePath(points: readonly Pick<TrackPoint, 'lat' | 'lng'>[] | undefined, width: number, height: number, pad = 8) {
  const projected = routePoints(points, width, height, pad);
  return projected ? projected.map(([x, y], index) => `${index ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`).join('') : null;
}

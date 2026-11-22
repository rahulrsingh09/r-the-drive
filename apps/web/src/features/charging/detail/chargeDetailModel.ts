/**
 * Pure presentation rules for the charge detail page. No React, no requests.
 * Copy follows ASD-STE100 and the glossary in docs/branding.md: "Battery", not
 * "SoC"; "→", not "->".
 */
import type {
  ChargeCostMethod, ChargeCurvePoint, ChargePhase, ChargeSession, ChargeShapeValues, Place,
  PlaceAddress, UpsertPlaceBody,
} from '@panda/types';

const KM_PER_MILE = 1.609344;
/** Lowest rate that only a DC charger gives (the API uses the same rule). */
const DC_MIN_RATE_KW = 20;
const DC_NETWORKS = ['tesla', 'rivian', 'electrify america', 'evgo'];
const EARTH_RADIUS_M = 6_371_000;
const DEFAULT_PLACE_RADIUS_M = 75;

export type ChargeKind = 'ac' | 'dc';

export function chargeKind(session: Pick<ChargeSession, 'charger_type' | 'network_vendor' | 'peak_power_kw' | 'ended_at' | 'live_power_kw'>): ChargeKind | null {
  const type = session.charger_type?.toLowerCase();
  if (type === 'dc' || type === 'dcfc') return 'dc';
  if (type === 'ac') return 'ac';
  const vendor = (session.network_vendor ?? '').trim().toLowerCase();
  if (vendor && DC_NETWORKS.includes(vendor)) return 'dc';
  const power = session.ended_at == null ? session.live_power_kw ?? session.peak_power_kw : session.peak_power_kw;
  if (power != null && Number.isFinite(power)) return power < DC_MIN_RATE_KW ? 'ac' : 'dc';
  return null;
}

export function chargeKindLabel(kind: ChargeKind | null) {
  return kind === 'dc' ? 'DC FAST' : kind === 'ac' ? 'AC' : null;
}

const NETWORK_TITLES: Record<string, string> = {
  tesla: 'Tesla Supercharger',
  rivian: 'Rivian Adventure Network',
  'electrify america': 'Electrify America',
  evgo: 'EVgo',
  chargepoint: 'ChargePoint',
};

export function networkTitle(vendor: string | null | undefined) {
  const trimmed = vendor?.trim();
  if (!trimmed) return null;
  return NETWORK_TITLES[trimmed.toLowerCase()] ?? trimmed;
}

function sameText(left: string | null | undefined, right: string | null | undefined) {
  return !!left && !!right && left.trim().toLocaleLowerCase() === right.trim().toLocaleLowerCase();
}

/**
 * Title = the place. A public charge that is not at a saved place uses the
 * network name, and the place or city moves to the subtitle.
 */
export function chargeTitle(session: ChargeSession, savedPlace: Place | null): { title: string; where: string | null } {
  if (savedPlace) return { title: savedPlace.name, where: null };
  const network = session.is_public !== false ? networkTitle(session.network_vendor) : null;
  const location = session.location_name?.trim() || null;
  const city = session.rivian_city?.trim() || null;
  if (network && !sameText(network, location)) {
    return { title: network, where: location ?? city };
  }
  if (location) return { title: location, where: city && !location.toLocaleLowerCase().includes(city.toLocaleLowerCase()) ? city : null };
  if (city) return { title: city, where: null };
  return { title: 'Charge', where: null };
}

/** "4:08–4:30 PM", or "11:50 PM–12:20 AM" when the halves differ. */
export function timeRangeText(start: string, end: string) {
  const suffix = /\s?([AP]M)$/i;
  const startSuffix = suffix.exec(start)?.[1];
  const endSuffix = suffix.exec(end)?.[1];
  const head = startSuffix && startSuffix === endSuffix ? start.replace(suffix, '') : start;
  return `${head}–${end}`;
}

export function formatMinutes(minutes: number | null | undefined) {
  if (minutes == null || !Number.isFinite(minutes)) return null;
  const rounded = Math.max(0, Math.round(minutes));
  if (rounded < 60) return `${rounded} min`;
  const hours = Math.floor(rounded / 60);
  const rest = rounded % 60;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

export function formatKw(value: number) {
  return value >= 20 ? `${Math.round(value)}` : `${Number(value.toFixed(1))}`;
}

export function formatPct(value: number | null | undefined) {
  return value == null || !Number.isFinite(value) ? null : `${Math.round(value)}%`;
}

export function rangeText(rangeKm: number | null | undefined, distanceUnit: 'miles' | 'kilometers') {
  if (rangeKm == null || !Number.isFinite(rangeKm) || rangeKm <= 0) return null;
  const value = distanceUnit === 'kilometers' ? rangeKm : rangeKm / KM_PER_MILE;
  return `about +${Math.round(value)} ${distanceUnit === 'kilometers' ? 'km' : 'mi'} of range`;
}

/** "40% → 71% in 22 min · about +92 mi of range". */
export function heroLine({ socStart, socEnd, minutes, rangeKm, distanceUnit, live }: {
  socStart: number | null | undefined;
  socEnd: number | null | undefined;
  minutes: number | null | undefined;
  rangeKm: number | null | undefined;
  distanceUnit: 'miles' | 'kilometers';
  live: boolean;
}) {
  const start = formatPct(socStart);
  const end = formatPct(socEnd);
  const duration = formatMinutes(minutes);
  const battery = start && end ? `${start} → ${end}` : null;
  const time = duration ? (live ? `${duration} so far` : `in ${duration}`) : null;
  const head = battery && time ? `${battery} ${time}` : battery ?? (time ? (live ? time : duration) : null);
  return [head, rangeText(rangeKm, distanceUnit)].filter(Boolean).join(' · ') || null;
}

// ---------------------------------------------------------------- cost

export const FREE_METHODS: ReadonlySet<string> = new Set(['user_free', 'network_free', 'rivian_free_session']);

/** The label for each cost method. `profile` depends on whether the place is home. */
export function costMethodLabel(method: ChargeCostMethod | string | null | undefined, atHome: boolean) {
  switch (method) {
    case 'rivian_paid_total': return 'Billed by Rivian';
    case 'profile': return atHome ? 'Your home rate' : 'Place rate';
    case 'user_manual': return 'Your price';
    case 'user_free':
    case 'network_free':
    case 'rivian_free_session': return 'Free';
    case 'unknown': return 'No price';
    case 'profile_pending': return 'Price pending';
    default: return null;
  }
}

/** 0.5 → "50¢"; 0.485 → "48.5¢"; 1.2 → "$1.20". */
export function formatRate(rate: number) {
  if (rate < 1) return `${Number((rate * 100).toFixed(1))}¢`;
  return `$${rate.toFixed(2)}`;
}

export function formatMoney(value: number) {
  return value.toLocaleString('en-US', { style: 'currency', currency: 'USD' });
}

export function roundCents(value: number) {
  return Math.round((value + Number.EPSILON) * 100) / 100;
}

/** One comma as the decimal mark: "14,30" or "0,5". */
const DECIMAL_COMMA = /^\d+,\d{1,2}$/;

function amountBody(input: string) {
  const body = input.trim().replace(/^\$/, '').trim();
  const cents = /¢$/.test(body);
  return { body: body.replace(/¢$/, '').trim(), cents };
}

/** True when the input has a comma that is not one decimal comma, for example "1,430". */
export function hasUnclearComma(input: string) {
  const { body } = amountBody(input);
  return body.includes(',') && !DECIMAL_COMMA.test(body);
}

/**
 * Parse a typed amount: "14.30", "$14.30", "14,30", "50¢" (rate input only).
 * Null when not a non-negative number. A comma is only a decimal mark
 * ("14,30" = 14.30); other commas ("1,430") are not accepted.
 */
export function parseAmount(input: string, unit: 'dollars' | 'rate'): number | null {
  const { body, cents } = amountBody(input);
  if (!body) return null;
  if (body.includes(',') && !DECIMAL_COMMA.test(body)) return null;
  const number = Number(body.replace(',', '.'));
  if (!Number.isFinite(number) || number < 0) return null;
  if (cents && unit === 'rate') return number / 100;
  if (cents) return null;
  return number;
}

export type CostEntryMode = 'total' | 'rate';

export interface CostEntry {
  /** cost_usd to save; null when the input is not valid. */
  total: number | null;
  /** $/kWh shown in the preview and saved to a Place; null when energy is not known. */
  rate: number | null;
  preview: string | null;
  error: string | null;
}

/**
 * Turn the box input into the value to save. A rate is saved as a total:
 * rate × the energy that the session row shows, rounded to cents.
 */
export function costEntry(mode: CostEntryMode, input: string, kwh: number | null | undefined): CostEntry {
  const energy = kwh != null && Number.isFinite(kwh) && kwh > 0 ? kwh : null;
  if (!input.trim()) return { total: null, rate: null, preview: null, error: null };
  const amount = parseAmount(input, mode === 'rate' ? 'rate' : 'dollars');
  if (amount == null) {
    const example = mode === 'rate' ? '0.50' : '14.30';
    if (hasUnclearComma(input)) {
      return { total: null, rate: null, preview: null, error: `Do not use a comma for thousands. Use a period or one comma for the decimal, for example ${example}.` };
    }
    return { total: null, rate: null, preview: null, error: mode === 'rate' ? `Enter a rate of 0 or more, for example ${example}.` : `Enter an amount of 0 or more, for example ${example}.` };
  }
  if (mode === 'rate') {
    if (energy == null) return { total: null, rate: amount, preview: null, error: 'The energy added is not known, so the app cannot compute a total. Enter the total paid.' };
    const total = roundCents(amount * energy);
    return { total, rate: amount, preview: `= ${formatMoney(total)}`, error: null };
  }
  const total = roundCents(amount);
  if (energy == null) return { total, rate: null, preview: null, error: null };
  const rate = total / energy;
  return { total, rate, preview: `= ${formatRate(rate)} per kWh for ${Number(energy.toFixed(1))} kWh`, error: null };
}

export interface CostDisplay {
  value: string;
  context: string | null;
  attention: boolean;
}

/** The Cost tile and the current-price line of the cost box. */
export function costDisplay(session: ChargeSession, atHome: boolean): CostDisplay {
  const method = session.cost_method ?? null;
  const open = session.ended_at == null;
  if (open) {
    return {
      value: 'Pending',
      context: session.live_current_price != null ? `About ${formatMoney(session.live_current_price)} so far` : 'Shows when the charge ends',
      attention: false,
    };
  }
  if (method && FREE_METHODS.has(method)) return { value: formatMoney(session.cost_usd ?? 0), context: 'Free', attention: false };
  if (method === 'user_manual' && session.cost_usd != null) {
    const rate = session.cost_rate_per_kwh;
    return {
      value: formatMoney(session.cost_usd),
      context: rate != null && Number.isFinite(rate) ? `Your price · ${formatRate(rate)}/kWh` : 'Your price',
      attention: false,
    };
  }
  if (method === 'profile_pending') return { value: 'Price pending', context: 'Waits for the charge data', attention: false };
  if (session.cost_usd != null) {
    return { value: formatMoney(session.cost_usd), context: costMethodLabel(method, atHome), attention: false };
  }
  return { value: 'No price', context: method === 'unknown' || method == null ? 'Enter the cost of this charge' : null, attention: true };
}

/** "Your price $14.30 · 50¢/kWh", for the cost box. */
export function currentPriceLine(session: ChargeSession, atHome: boolean) {
  const display = costDisplay(session, atHome);
  if (session.cost_method === 'user_manual' && session.cost_usd != null) {
    const rate = session.cost_rate_per_kwh;
    return `Your price ${formatMoney(session.cost_usd)}${rate != null && Number.isFinite(rate) ? ` · ${formatRate(rate)}/kWh` : ''}`;
  }
  const label = costMethodLabel(session.cost_method, atHome);
  if (display.value === label || !label) return display.value;
  return `${label} ${display.value}`;
}

// --------------------------------------------------------------- places

function distanceMeters(latA: number, lngA: number, latB: number, lngB: number) {
  const toRad = (deg: number) => (deg * Math.PI) / 180;
  const dLat = toRad(latB - latA);
  const dLng = toRad(lngB - lngA);
  const a = Math.sin(dLat / 2) ** 2 + Math.cos(toRad(latA)) * Math.cos(toRad(latB)) * Math.sin(dLng / 2) ** 2;
  return 2 * EARTH_RADIUS_M * Math.asin(Math.min(1, Math.sqrt(a)));
}

export function sessionCoordinates(session: Pick<ChargeSession, 'location_lat' | 'location_lng'>) {
  const { location_lat: lat, location_lng: lng } = session;
  return lat != null && lng != null && Number.isFinite(lat) && Number.isFinite(lng) ? { lat, lng } : null;
}

/** The saved Place whose area holds the charge (nearest first), else a Place with the same name. */
export function matchSavedPlace(session: ChargeSession, places: Place[]): Place | null {
  const point = sessionCoordinates(session);
  if (point) {
    let best: { place: Place; distance: number } | null = null;
    for (const place of places) {
      const distance = distanceMeters(point.lat, point.lng, place.latitude, place.longitude);
      if (distance <= Math.max(place.radius_m, 1) && (!best || distance < best.distance)) best = { place, distance };
    }
    if (best) return best.place;
  }
  return places.find((place) => sameText(place.name, session.location_name)) ?? null;
}

export interface PlaceRatePlan {
  placeId: string | null;
  body: UpsertPlaceBody;
  /** Copy that says what Save does to the saved Place. */
  description: string;
  /**
   * The confirm step before Save, for an existing Place. The server then prices
   * the past automatic charges at that Place again (routes/places.rs update_place).
   * Null for a new Place: create_place prices no charge again at once.
   */
  confirm: string | null;
}

/** Where to change a price that the cost box must not change. */
export const PLACES_SETTINGS = 'Settings → Places';

/**
 * Why the cost box must not save a rate to the Place, or null when it can.
 * The Home price and a time-of-use schedule change only in Settings → Places:
 * a per-kWh rate from one receipt would replace them and price their history again.
 */
export function placeRateBlock(place: Place | null, atHome: boolean): string | null {
  if (place?.is_home || (!place && atHome)) return `Change the Home price in ${PLACES_SETTINGS}.`;
  if (place?.charging?.billing_type === 'tou') {
    return `${place.name} has time-of-use rates. Change its price in ${PLACES_SETTINGS}.`;
  }
  return null;
}

/**
 * Create or update the saved Place at the charge location with a per-kWh rate.
 * Null when the charge has no location to save. Call `placeRateBlock` first:
 * the Home place and a time-of-use place are not changed here.
 */
export function placeRatePlan(session: ChargeSession, place: Place | null, rate: number, timezone: string, defaultName: string): PlaceRatePlan | null {
  const rateText = `${formatRate(rate)} per kWh`;
  if (place) {
    const address: PlaceAddress = place.address ?? {
      display_name: place.name, osm_id: null, latitude: place.latitude, longitude: place.longitude,
      road: null, city: null, state: null, postcode: null, country: null, raw: null,
    };
    const confirm = `This sets ${place.name} to ${formatRate(rate)}/kWh. Past automatic charges at ${place.name} are priced again with this rate.`;
    return {
      placeId: place.id,
      body: {
        name: place.name,
        radius_m: place.radius_m,
        is_home: place.is_home,
        is_work: place.is_work,
        address,
        charging: {
          ...(place.charging?.name ? { name: place.charging.name } : {}),
          billing_type: 'per_kwh',
          rate,
          session_fee: place.charging?.session_fee ?? 0,
          currency: place.charging?.currency ?? 'USD',
          timezone: place.charging?.timezone ?? timezone,
        },
      },
      description: confirm,
      confirm,
    };
  }
  const point = sessionCoordinates(session);
  if (!point) return null;
  const name = defaultName.trim() || 'Charger';
  return {
    placeId: null,
    body: {
      name,
      radius_m: DEFAULT_PLACE_RADIUS_M,
      is_home: false,
      is_work: false,
      address: {
        display_name: [session.location_name?.trim(), session.rivian_city?.trim()].filter(Boolean).join(', ') || name,
        osm_id: null, latitude: point.lat, longitude: point.lng,
        road: null, city: session.rivian_city ?? null, state: null, postcode: null, country: null, raw: null,
      },
      charging: { billing_type: 'per_kwh', rate, session_fee: 0, currency: 'USD', timezone },
    },
    description: `Adds the saved place “${name}” at this charger with ${rateText}. Automatic charges at this place use this rate.`,
    confirm: null,
  };
}

// ---------------------------------------------------------------- curve

export interface CurvePoint {
  /** Minutes from the session start, the same origin as the API charge_shape. Can be below 0. */
  minute: number;
  /** Battery %, interpolated by time when Rivian sent none. Null when no battery % is known. */
  soc: number | null;
  kw: number;
}

export interface ChargeCurveModel {
  points: CurvePoint[];
  /** True when every point is an estimate from battery % changes. */
  estimated: boolean;
  /** True when every point has a battery % and the battery % rises. */
  socAxis: boolean;
  /** First minute of the time axis: 0, or the first reading when it is before the session start. */
  startMin: number;
  /** Last minute of the time axis, from the session start. */
  durationMin: number;
  socMin: number;
  socMax: number;
  peakIndex: number;
  lowIndex: number;
}

function positiveSoc(value: number | null | undefined) {
  return value != null && Number.isFinite(value) && value > 0 && value <= 100 ? value : null;
}

/**
 * Prepare the curve for the chart. Measured points win over estimates. A
 * point with no battery % gets one interpolated by time between known
 * points, or between the session start and end battery %. Minutes stay
 * counted from the session start (`minutes_elapsed`), so the API phase and
 * peak minutes line up with the points.
 */
export function buildChargeCurve(raw: ChargeCurvePoint[], session: Pick<ChargeSession, 'soc_start' | 'soc_end' | 'duration_min'>): ChargeCurveModel | null {
  const valid = raw.filter((point) => Number.isFinite(point.power_kw) && point.power_kw >= 0);
  const recorded = valid.filter((point) => point.power_method == null || point.power_method === 'recorded');
  const source = recorded.length >= 2 ? recorded : valid;
  if (source.length < 2) return null;
  const sorted = source
    .map((point, index) => ({ point, minute: point.minutes_elapsed ?? index }))
    .filter(({ minute }) => Number.isFinite(minute))
    .sort((left, right) => left.minute - right.minute);
  if (sorted.length < 2) return null;

  const anchors: Array<{ minute: number; soc: number }> = [];
  sorted.forEach(({ point, minute }) => {
    const soc = positiveSoc(point.soc_pct);
    if (soc != null) anchors.push({ minute, soc });
  });
  const first = sorted[0]!.minute;
  const last = sorted[sorted.length - 1]!.minute;
  const startSoc = positiveSoc(session.soc_start);
  const endSoc = positiveSoc(session.soc_end);
  if ((anchors.length === 0 || anchors[0]!.minute > first) && startSoc != null) anchors.unshift({ minute: first, soc: startSoc });
  if ((anchors.length === 0 || anchors[anchors.length - 1]!.minute < last) && endSoc != null) anchors.push({ minute: last, soc: endSoc });

  const socAt = (minute: number): number | null => {
    if (anchors.length === 0) return null;
    if (anchors.length === 1) return anchors[0]!.soc;
    if (minute <= anchors[0]!.minute) return anchors[0]!.soc;
    for (let index = 1; index < anchors.length; index += 1) {
      const right = anchors[index]!;
      if (minute <= right.minute) {
        const left = anchors[index - 1]!;
        const span = right.minute - left.minute;
        return span <= 0 ? right.soc : left.soc + ((minute - left.minute) / span) * (right.soc - left.soc);
      }
    }
    return anchors[anchors.length - 1]!.soc;
  };

  const points: CurvePoint[] = sorted.map(({ point, minute }) => ({
    minute,
    soc: positiveSoc(point.soc_pct) ?? socAt(minute),
    kw: point.power_kw,
  }));
  const socs = points.map((point) => point.soc).filter((soc): soc is number => soc != null);
  const socMin = socs.length ? Math.min(...socs) : 0;
  const socMax = socs.length ? Math.max(...socs) : 0;
  const socAxis = socs.length === points.length && socMax - socMin >= 2;

  let peakIndex = 0;
  points.forEach((point, index) => { if (point.kw > points[peakIndex]!.kw) peakIndex = index; });
  let lowIndex = peakIndex;
  for (let index = peakIndex; index < points.length; index += 1) {
    if (points[index]!.kw <= points[lowIndex]!.kw) lowIndex = index;
  }

  return {
    points,
    estimated: recorded.length < 2,
    socAxis,
    startMin: Math.min(0, first),
    durationMin: Math.max(last, session.duration_min ?? 0, 1),
    socMin,
    socMax,
    peakIndex,
    lowIndex,
  };
}

/** Battery % at a minute, by linear interpolation along the curve. */
export function socAtMinute(points: CurvePoint[], minute: number): number | null {
  const known = points.filter((point) => point.soc != null);
  if (known.length === 0) return null;
  if (minute <= known[0]!.minute) return known[0]!.soc;
  for (let index = 1; index < known.length; index += 1) {
    const right = known[index]!;
    if (minute <= right.minute) {
      const left = known[index - 1]!;
      const span = right.minute - left.minute;
      return span <= 0 ? right.soc : left.soc! + ((minute - left.minute) / span) * (right.soc! - left.soc!);
    }
  }
  return known[known.length - 1]!.soc;
}

export function nearestIndexByMinute(points: CurvePoint[], minute: number) {
  let best = 0;
  points.forEach((point, index) => {
    if (Math.abs(point.minute - minute) < Math.abs(points[best]!.minute - minute)) best = index;
  });
  return best;
}

export const PHASE_LABELS: Record<ChargePhase['name'], string> = { climb: 'Climb', peak: 'Peak', taper: 'Taper' };

/** "0 → 2 min". */
export function phaseSpan(phase: ChargePhase) {
  return `${Math.round(phase.start_min)} → ${Math.round(phase.end_min)} min`;
}

/** The kW line of a phase card: "to 108 kW", "105–108 kW", "108 → 52 kW". */
export function phaseKw(phase: ChargePhase) {
  if (phase.name === 'climb') return `to ${formatKw(phase.to_kw)} kW`;
  if (phase.name === 'peak') {
    const low = Math.min(phase.from_kw, phase.to_kw);
    const high = Math.max(phase.from_kw, phase.to_kw);
    return Math.round(low) === Math.round(high) ? `${formatKw(high)} kW` : `${formatKw(low)}–${formatKw(high)} kW`;
  }
  return `${formatKw(phase.from_kw)} → ${formatKw(phase.to_kw)} kW`;
}

/**
 * One sentence for each phase. `chargerLimited` comes from the curve shape
 * (power fell and then recovered); then the taper sentence names the charger.
 */
export function phaseSentence(phase: ChargePhase, shape: ChargeShapeValues, chargerLimited: boolean) {
  const minutes = Math.max(0, Math.round(phase.end_min - phase.start_min));
  if (phase.name === 'climb') {
    return `Power climbed from ${formatKw(phase.from_kw)} to ${formatKw(phase.to_kw)} kW in the first ${Math.max(1, Math.round(phase.end_min))} min.`;
  }
  if (phase.name === 'peak') {
    const at = formatPct(shape.peak_soc);
    const floor = formatKw(Math.min(phase.from_kw, phase.to_kw, shape.peak_kw));
    return `Peak ${formatKw(shape.peak_kw)} kW${at ? ` at ${at}` : ''}. Power stayed above ${floor} kW for ${minutes} min.`;
  }
  const lowAt = formatPct(shape.low_soc);
  if (chargerLimited) {
    return `Power went down and up between ${formatKw(shape.low_kw)} and ${formatKw(shape.peak_kw)} kW. The charger probably limited the rate.`;
  }
  return `Then a steady taper to ${formatKw(phase.to_kw)} kW${lowAt ? ` at ${lowAt}` : ''} as the battery filled.`;
}

/** Context line of the peak and lowest power tiles: "At 44%, 2 min in" or "At 71%, at the end". */
export function pointContext(soc: number | null | undefined, minute: number | null | undefined, atEnd: boolean) {
  const at = formatPct(soc);
  const when = atEnd ? 'at the end' : minute != null && Number.isFinite(minute) ? `${Math.round(minute)} min in` : null;
  return [at ? `At ${at}` : null, when].filter(Boolean).join(', ') || null;
}

/** Average power = energy added / charging time. */
export function averagePowerKw(kwh: number | null | undefined, minutes: number | null | undefined) {
  if (kwh == null || minutes == null || !Number.isFinite(kwh) || !Number.isFinite(minutes) || minutes <= 0 || kwh <= 0) return null;
  return kwh / (minutes / 60);
}

/** Usual DC band caption. */
export function bandCaption(sessions: number, withinPct: number | null) {
  const charges = `${sessions} earlier ${sessions === 1 ? 'charge' : 'charges'}`;
  const within = withinPct == null ? '' : withinPct >= 70
    ? ' This charge was close to it.'
    : withinPct <= 30 ? ' This charge was outside it for most of the battery range.' : '';
  return `Gray band: your usual DC curve (${charges}).${within}`;
}

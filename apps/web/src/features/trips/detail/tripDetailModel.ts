import type { Insight, Place, RouteComparisonValues, Trip, TripDetailSamples } from '@panda/types';
import type { SpeedHistogramBin } from '@panda/ui/charts';
import type { UnitPreferences } from '@panda/ui/lib/utils';

/** One trip sample, aligned to the trip start. */
export interface TimelinePoint {
  elapsed_s: number;
  ts: string;
  speed_mph: number | null;
  power_kw: number | null;
  /** Regenerative braking power as a negative kW value. */
  regen_kw: number | null;
  estimated_net_power_kw: number | null;
  battery_level: number | null;
  outside_temp_c: number | null;
  cabin_temp_c: number | null;
  driver_temp_c: number | null;
  hvac_active: boolean | null;
  tire_fl_psi: number | null;
  tire_fr_psi: number | null;
  tire_rl_psi: number | null;
  tire_rr_psi: number | null;
  altitude_m: number | null;
  lat: number | null;
  lng: number | null;
}

/** A display value split into its number and unit, so the unit can be smaller. */
export interface Measure {
  value: string;
  unit: string;
}

const MILES_TO_KM = 1.609344;
const METERS_TO_FEET = 3.28084;
const PSI_TO_KPA = 6.894757293168361;
/** Elevation chart threshold from the design: a change of more than 100 ft. */
export const ELEVATION_CHART_MIN_RANGE_M = 100 / METERS_TO_FEET;

const finite = (value: number | null | undefined): value is number => value != null && Number.isFinite(value);
const number = (value: number, decimals: number) => value.toLocaleString('en-US', {
  minimumFractionDigits: decimals,
  maximumFractionDigits: decimals,
});

export function buildTimeline(tripStartIso: string | undefined, samples: TripDetailSamples | undefined): TimelinePoint[] {
  if (!tripStartIso || !samples) return [];
  const tripStartMs = Date.parse(tripStartIso);
  if (!Number.isFinite(tripStartMs)) return [];
  return samples.elapsed_s.map((elapsed, index) => ({
    elapsed_s: elapsed,
    ts: new Date(tripStartMs + elapsed * 1000).toISOString(),
    speed_mph: samples.speed_mph[index] ?? null,
    power_kw: samples.power_kw[index] ?? null,
    regen_kw: samples.regen_power_kw[index] != null ? -Math.abs(samples.regen_power_kw[index]!) : null,
    estimated_net_power_kw: samples.estimated_net_power_kw?.[index] ?? null,
    battery_level: samples.battery_level[index] ?? null,
    outside_temp_c: samples.outside_temp_c[index] ?? null,
    cabin_temp_c: samples.cabin_temp_c[index] ?? null,
    driver_temp_c: samples.driver_temp_c[index] ?? null,
    hvac_active: samples.hvac_active[index] ?? null,
    tire_fl_psi: samples.tire_fl_psi[index] ?? null,
    tire_fr_psi: samples.tire_fr_psi[index] ?? null,
    tire_rl_psi: samples.tire_rl_psi[index] ?? null,
    tire_rr_psi: samples.tire_rr_psi[index] ?? null,
    altitude_m: samples.altitude_m[index] ?? null,
    lat: samples.lat[index] ?? null,
    lng: samples.lng[index] ?? null,
  }));
}

/** "Home → Office". Null when neither end has a label. */
export function routeTitle(start: { label: string; source: string }, end: { label: string; source: string }) {
  if (start.source === 'unavailable' && end.source === 'unavailable') return null;
  return `${start.label} → ${end.label}`;
}

/** Elapsed time as whole minutes: "0 min", "12 min". */
export function formatElapsedMinutes(seconds: number) {
  return `${Math.max(0, Math.round(seconds / 60))} min`;
}

/** X-axis ticks every 5, 10, 15 or 30 minutes, so a trip has about 3 to 6 labels. */
export function minuteSplits(durationSeconds: number) {
  if (!Number.isFinite(durationSeconds) || durationSeconds <= 0) return undefined;
  const minutes = durationSeconds / 60;
  const step = [1, 2, 5, 10, 15, 30, 60, 120].find(candidate => minutes / candidate <= 5) ?? 240;
  const splits: number[] = [];
  for (let minute = 0; minute * 60 <= durationSeconds + 1; minute += step) splits.push(minute * 60);
  return splits;
}

export function distanceMeasure(miles: number | null | undefined, prefs: UnitPreferences, decimals = 1): Measure | null {
  if (!finite(miles)) return null;
  return prefs.distance_unit === 'kilometers'
    ? { value: number(miles * MILES_TO_KM, decimals), unit: 'km' }
    : { value: number(miles, decimals), unit: 'mi' };
}

export function odometerMeasure(miles: number | null | undefined, prefs: UnitPreferences): Measure | null {
  return distanceMeasure(miles, prefs, 0);
}

export function speedMeasure(mph: number | null | undefined, prefs: UnitPreferences): Measure | null {
  if (!finite(mph)) return null;
  return prefs.speed_unit === 'kmh'
    ? { value: number(mph * MILES_TO_KM, 0), unit: 'km/h' }
    : { value: number(mph, 0), unit: 'mph' };
}

/** Efficiency in the owner's unit. The API value is Wh/mi. */
export function efficiencyMeasure(whPerMile: number | null | undefined, prefs: UnitPreferences): Measure | null {
  if (!finite(whPerMile) || whPerMile <= 0) return null;
  if (prefs.efficiency_display === 'energy_per_distance') {
    return prefs.distance_unit === 'kilometers'
      ? { value: number(whPerMile / MILES_TO_KM, 0), unit: 'Wh/km' }
      : { value: number(whPerMile, 0), unit: 'Wh/mi' };
  }
  const miPerKwh = 1000 / whPerMile;
  return prefs.distance_unit === 'kilometers'
    ? { value: number(miPerKwh * MILES_TO_KM, 1), unit: 'km/kWh' }
    : { value: number(miPerKwh, 1), unit: 'mi/kWh' };
}

export function efficiencyFromMiPerKwh(miPerKwh: number, prefs: UnitPreferences) {
  return miPerKwh > 0 ? efficiencyMeasure(1000 / miPerKwh, prefs) : null;
}

export function temperatureText(celsius: number, prefs: UnitPreferences) {
  return prefs.temperature_unit === 'celsius'
    ? `${Math.round(celsius)} °C`
    : `${Math.round(celsius * 9 / 5 + 32)} °F`;
}

export function joinMeasure(measure: Measure | null) {
  return measure ? `${measure.value} ${measure.unit}` : null;
}

/** Average speed from the trip distance and the trip duration. */
export function averageSpeedMph(distanceMiles: number | null | undefined, durationSeconds: number) {
  if (!finite(distanceMiles) || durationSeconds <= 0) return null;
  return distanceMiles / (durationSeconds / 3600);
}

/**
 * Energy that regenerative braking returned, in kWh. It uses the same rule as
 * the trip detector: each regen reading times the time since the previous
 * regen reading. A gap longer than `maxGapSeconds` adds nothing.
 */
export function regenEnergyKwh(timeline: TimelinePoint[], maxGapSeconds = 300) {
  let previous: number | null = null;
  let kwh = 0;
  let readings = 0;
  for (const point of timeline) {
    if (!finite(point.regen_kw) || !Number.isFinite(point.elapsed_s)) continue;
    readings += 1;
    if (previous != null) {
      const seconds = point.elapsed_s - previous;
      if (seconds > 0 && seconds <= maxGapSeconds) kwh += Math.abs(point.regen_kw) * seconds / 3600;
    }
    if (previous == null || point.elapsed_s >= previous) previous = point.elapsed_s;
  }
  return readings > 1 ? kwh : null;
}

/** The kW value for the replay readout: drive power, or regen when the car is braking. */
export function readoutPowerKw(point: TimelinePoint | null | undefined, estimated: boolean) {
  if (!point) return null;
  if (estimated) return point.estimated_net_power_kw;
  if (finite(point.power_kw) && point.power_kw > 0) return point.power_kw;
  if (finite(point.regen_kw) && point.regen_kw < 0) return point.regen_kw;
  return point.power_kw ?? point.regen_kw;
}

export function peakPowerKw(timeline: TimelinePoint[], estimated: boolean) {
  let peak: number | null = null;
  for (const point of timeline) {
    const value = estimated ? point.estimated_net_power_kw : point.power_kw;
    if (finite(value) && (peak == null || value > peak)) peak = value;
  }
  return peak;
}

function validLocation(point: TimelinePoint) {
  return finite(point.lat) && finite(point.lng) && Math.abs(point.lat) <= 90 && Math.abs(point.lng) <= 180
    && (point.lat !== 0 || point.lng !== 0);
}

function haversineMiles(a: TimelinePoint, b: TimelinePoint) {
  const radians = Math.PI / 180;
  const h = Math.sin((b.lat! - a.lat!) * radians / 2) ** 2
    + Math.cos(a.lat! * radians) * Math.cos(b.lat! * radians) * Math.sin((b.lng! - a.lng!) * radians / 2) ** 2;
  return 3958.8 * 2 * Math.asin(Math.sqrt(Math.min(1, Math.max(0, h))));
}

/**
 * Distance driven at each sample, from the GPS track, scaled so the last
 * sample equals the trip distance. Null when the track has fewer than two fixes.
 */
export function cumulativeDistanceMiles(timeline: TimelinePoint[], tripDistanceMiles: number | null | undefined) {
  let total = 0;
  let previous: TimelinePoint | null = null;
  const raw = timeline.map((point) => {
    if (validLocation(point)) {
      if (previous) total += haversineMiles(previous, point);
      previous = point;
    }
    return total;
  });
  if (total <= 0) return null;
  const scale = finite(tripDistanceMiles) && tripDistanceMiles > 0 ? tripDistanceMiles / total : 1;
  return raw.map(value => value * scale);
}

export interface ElevationSummary {
  gainM: number;
  lossM: number;
  rangeM: number;
}

/** Climb and descent with a 1 m dead band, so sensor noise does not add up. */
export function summarizeElevation(timeline: TimelinePoint[], deadBandM = 1): ElevationSummary | null {
  const values = timeline.map(point => point.altitude_m).filter(finite);
  if (values.length < 2) return null;
  let anchor = values[0]!;
  let gain = 0;
  let loss = 0;
  for (const value of values.slice(1)) {
    const delta = value - anchor;
    if (Math.abs(delta) < deadBandM) continue;
    if (delta > 0) gain += delta; else loss -= delta;
    anchor = value;
  }
  return { gainM: gain, lossM: loss, rangeM: Math.max(...values) - Math.min(...values) };
}

export function elevationText(summary: ElevationSummary, prefs: UnitPreferences) {
  const feet = prefs.altitude_unit !== 'meters';
  const format = (meters: number) => feet ? `${Math.round(meters * METERS_TO_FEET)} ft` : `${Math.round(meters)} m`;
  const rounded = (meters: number) => Math.round(feet ? meters * METERS_TO_FEET : meters);
  if (rounded(summary.gainM) === 0 && rounded(summary.lossM) === 0) return 'No change';
  return `+${format(summary.gainM)}, −${format(summary.lossM)}`;
}

function median(values: number[]) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

export interface ConditionRow {
  key: 'outside' | 'cabin' | 'elevation' | 'tires' | 'drive_mode';
  label: string;
  value: string;
}

export interface ConditionInputs {
  timeline: TimelinePoint[];
  /** Outside readings from the detail response, vehicle or weather source. */
  outsideSamples?: Array<{ temperature_c: number }> | undefined;
  outsideEstimated?: boolean | undefined;
  tripOutsideC?: number | null | undefined;
  driveModeLabel?: string | null | undefined;
  elevation: ElevationSummary | null;
  prefs: UnitPreferences;
}

/** Plausible tire pressures. Readings outside this band are sensor faults. */
const TIRE_MIN_PSI = 5;
const TIRE_MAX_PSI = 100;

export function buildConditions(input: ConditionInputs): ConditionRow[] {
  const rows: ConditionRow[] = [];
  const { prefs, timeline } = input;
  const outsideValues = (input.outsideSamples?.length
    ? input.outsideSamples.map(sample => sample.temperature_c)
    : timeline.map(point => point.outside_temp_c)).filter(finite);
  const outside = median(outsideValues) ?? (finite(input.tripOutsideC) ? input.tripOutsideC : null);
  if (outside != null) {
    rows.push({ key: 'outside', label: 'Outside',
      value: `${temperatureText(outside, prefs)}${input.outsideEstimated ? ' · estimated' : ''}` });
  }
  const cabin = median(timeline.map(point => point.cabin_temp_c).filter(finite));
  if (cabin != null) {
    const climate = timeline.some(point => point.hvac_active === true);
    rows.push({ key: 'cabin', label: 'Cabin', value: `${temperatureText(cabin, prefs)}${climate ? ' · climate on' : ''}` });
  }
  if (input.elevation) rows.push({ key: 'elevation', label: 'Elevation', value: elevationText(input.elevation, prefs) });
  const tires = timeline.flatMap(point => [point.tire_fl_psi, point.tire_fr_psi, point.tire_rl_psi, point.tire_rr_psi])
    .filter((value): value is number => finite(value) && value >= TIRE_MIN_PSI && value <= TIRE_MAX_PSI);
  if (tires.length) {
    const kpa = prefs.pressure_unit === 'kpa';
    const unit = kpa ? 'kPa' : 'psi';
    const low = Math.round(Math.min(...tires) * (kpa ? PSI_TO_KPA : 1));
    const high = Math.round(Math.max(...tires) * (kpa ? PSI_TO_KPA : 1));
    rows.push({ key: 'tires', label: 'Tires', value: low === high ? `${low} ${unit}` : `${low}–${high} ${unit}` });
  }
  if (input.driveModeLabel) rows.push({ key: 'drive_mode', label: 'Drive mode', value: input.driveModeLabel });
  return rows;
}

export interface SpeedBand {
  label: string;
  min: number;
  max: number;
  seconds: number;
}

/** Joins the 5 mph profile bins into 10 mph bands; empty bands at the ends are dropped. */
export function buildSpeedBands(bins: SpeedHistogramBin[], prefs: UnitPreferences): SpeedBand[] {
  const kmh = prefs.speed_unit === 'kmh';
  const bands = new Map<number, SpeedBand>();
  for (const bin of bins) {
    const min = Math.floor(bin.min / 10) * 10;
    const band = bands.get(min) ?? {
      min, max: min + 10, seconds: 0,
      label: kmh ? `${Math.round(min * MILES_TO_KM)}–${Math.round((min + 10) * MILES_TO_KM)}` : `${min}–${min + 10}`,
    };
    band.seconds += bin.duration_seconds;
    bands.set(min, band);
  }
  const ordered = [...bands.values()].sort((a, b) => a.min - b.min);
  const first = ordered.findIndex(band => band.seconds > 0);
  if (first < 0) return [];
  let last = ordered.length - 1;
  while (last > first && ordered[last]!.seconds <= 0) last -= 1;
  return ordered.slice(first, last + 1);
}

export interface SpeedCoverageInput {
  durationSeconds: number;
  recordedSeconds: number;
  estimatedSeconds: number;
}

/** One coverage line, always inside the trip duration. */
export function speedCoverageText({ durationSeconds, recordedSeconds, estimatedSeconds }: SpeedCoverageInput) {
  const total = Math.max(0, Math.round(durationSeconds / 60));
  if (total === 0) return null;
  const clamp = (seconds: number) => Math.min(total, Math.max(0, Math.round(seconds / 60)));
  const recorded = clamp(recordedSeconds);
  const estimated = Math.min(total - recorded, clamp(estimatedSeconds));
  if (recorded > 0 && estimated > 0) return `Speed recorded for ${recorded} of ${total} min. GPS estimate for ${estimated} min.`;
  if (estimated > 0) return `Speed estimated from GPS for ${estimated} of ${total} min.`;
  return `Speed recorded for ${recorded} of ${total} min.`;
}

/** The route comparison for this trip only. Insights for another trip are ignored. */
export function findRouteComparison(insights: Insight[] | undefined, tripId: string): RouteComparisonValues | null {
  const insight = insights?.find(item => item.kind === 'route_comparison'
    && (item.id === `route_comparison:trip:${tripId}` || item.evidence.some(evidence => evidence.type === 'trip' && evidence.id === tripId)));
  return insight?.kind === 'route_comparison' ? insight.values : null;
}

export function ordinal(value: number) {
  const mod100 = value % 100;
  if (mod100 >= 11 && mod100 <= 13) return `${value}th`;
  return `${value}${({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[value % 10] ?? 'th'}`;
}

/** "3rd best of 13 drives". */
export function rankText(values: RouteComparisonValues) {
  const total = values.count + 1;
  return `${values.rank === 1 ? 'Best' : `${ordinal(values.rank)} best`} of ${total} drives`;
}

/** Efficiency compared with the usual drive on the route. */
export function routeDifferenceText(values: RouteComparisonValues) {
  const percent = Math.round(Math.abs(values.pct_diff));
  if (!Number.isFinite(percent) || percent === 0) return 'Same as your usual on this route';
  return `${percent}% ${values.pct_diff > 0 ? 'better' : 'worse'} than your usual on this route`;
}

export interface HomePrice {
  rate: number;
  currency: string;
}

/** The home place's price per kWh. Time-of-use and other prices give no single rate. */
export function findHomePrice(places: Place[] | undefined): HomePrice | null {
  const home = places?.find(place => place.is_home && place.charging?.billing_type === 'per_kwh');
  const rate = home?.charging?.rate;
  if (!home?.charging || !finite(rate) || rate <= 0) return null;
  return { rate, currency: home.charging.currency || 'USD' };
}

export function formatMoney(value: number, currency: string, decimals = 2) {
  try {
    return value.toLocaleString('en-US', { style: 'currency', currency, minimumFractionDigits: decimals, maximumFractionDigits: decimals });
  } catch {
    return `${number(value, decimals)} ${currency}`;
  }
}

export function tripCost(trip: Pick<Trip, 'energy_used_kwh'>, price: HomePrice | null) {
  if (!price || !finite(trip.energy_used_kwh) || trip.energy_used_kwh <= 0) return null;
  return trip.energy_used_kwh * price.rate;
}

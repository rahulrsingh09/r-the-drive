import type {
  Vehicle,
  Trip,
  TripPowerPoint,
  TripDetailSeriesPoint,
  TripDetailResponse,
  ChargeSession,
  ChargeCurvePoint,
  ChargeCurveAnalysisPoint,
  EfficiencyByMode,
  EfficiencySummary,
  ChargingSummary,
  ChargingChartSeries,
  PaginatedResponse,
  Place,
  BatteryMileagePoint,
  MetricCatalogEntry,
  VehicleMember,
  VehicleInvite,
  AdminUserRecord,
  AdminVehicleOption,
  AdminUserMembership,
  AdminUserInvite,
  AccountInvitation,
  TripTag,
} from '@panda/types';
import type { LiveSession } from './apiTypes';
import { liveFields } from './chargingSessionFields';
import { finiteValue, numeric, optionalText, project, textual } from './rowProjection';

export function normalizeTrip(raw: unknown): Trip {
  const row = isRecord(raw) ? raw : {};
  const distance = firstNumber(row, 'distance_mi', 'distance_miles') ?? 0;
  const efficiency = firstNumber(row, 'efficiency_wh_mi', 'efficiency_wh_per_mile');
  const energyWh = finiteNumber(row.energy_wh);
  const durationSeconds = finiteNumber(row.duration_seconds);
  const start = normalizeCoordinateValue(row.start_lat, row.start_lng);
  const end = normalizeCoordinateValue(row.end_lat, row.end_lng);
  return {
    ...recordIdentity(row),
    ...numberFields(row, 'max_speed_mph', 'soc_start', 'soc_end'),
    ...stringFields(row, 'start_address', 'end_address'),
    distance_mi: distance,
    duration_min:
      finiteNumber(row.duration_min) ?? (durationSeconds === undefined ? 0 : durationSeconds / 60),
    efficiency_wh_mi: efficiency ?? null,
    energy_used_kwh:
      finiteNumber(row.energy_used_kwh) ??
      (energyWh === undefined ? undefined : energyWh / 1000) ??
      (efficiency !== undefined && distance > 0 ? (efficiency * distance) / 1000 : null),
    drive_mode: typeof row.drive_mode === 'string' ? (row.drive_mode as Trip['drive_mode']) : null,
    start_lat: start?.lat ?? null,
    start_lng: start?.lng ?? null,
    end_lat: end?.lat ?? null,
    end_lng: end?.lng ?? null,
    start_place: firstString(row, 'start_place', 'start_place_name'),
    end_place: firstString(row, 'end_place', 'end_place_name'),
    tags: decodeTripTags(row.tags, String(row.vehicle_id ?? '')),
  };
}

export function normalizeChargeSession(raw: unknown): ChargeSession {
  const row = isRecord(raw) ? raw : {};
  const identity = recordIdentity(row);
  if (!identity.id) throw new Error('normalizeChargeSession: missing id in response');
  const energyWh = finiteNumber(row.energy_added_wh);
  const location =
    normalizeCoordinateLabel(row.location_name) ??
    (row.is_home === true
      ? 'Home'
      : formatCoordinateLabel(normalizeCoordinateValue(row.location_lat, row.location_lng)));
  return {
    ...identity,
    ...numberFields(
      row,
      'soc_start',
      'soc_end',
      'cost_usd',
      'range_added_km',
      'rivian_paid_total',
      'live_current_price',
      'live_total_charged_kwh',
      'live_range_added_km',
      'live_power_kw',
      'live_charge_rate_kph',
      'location_lat',
      'location_lng',
      'source_location_lat',
      'source_location_lng',
      'cost_override_usd'
    ),
    ...stringFields(
      row,
      'session_day_local',
      'cost_method',
      'source',
      'data_confidence',
      'network_vendor',
      'rivian_charger_type',
      'currency_code',
      'rivian_city',
      'charger_id',
      'live_current_currency'
    ),
    ...booleanFields(row, 'is_free_session', 'is_rivian_network', 'is_public'),
    ...liveFields(row),
    location_name: location,
    charger_type: oneOf(row.charger_type, ['AC', 'DC', 'DCFC']) as ChargeSession['charger_type'],
    energy_added_kwh:
      firstNumber(row, 'energy_added_kwh', 'kwh_added') ??
      (energyWh === undefined ? null : energyWh / 1000),
    peak_power_kw:
      firstNumber(row, 'peak_power_kw', 'max_charge_rate_kw', 'avg_charge_rate_kw') ?? null,
    duration_min: firstNumber(row, 'duration_min', 'duration_minutes') ?? null,
    api_started_at: nullableIdentity(row.api_started_at),
    api_ended_at: nullableIdentity(row.api_ended_at),
    telemetry_sample_count: finiteNumber(row.telemetry_sample_count) ?? 0,
    location_override_mode: oneOf(row.location_override_mode, ['automatic', 'saved_place', 'none']),
    cost_override_mode: oneOf(row.cost_override_mode, ['automatic', 'free', 'manual']),
  };
}

function recordIdentity(row: Record<string, unknown>) {
  return {
    id: String(row.id ?? ''),
    vehicle_id: String(row.vehicle_id ?? ''),
    started_at: String(row.started_at ?? ''),
    ended_at: nullableIdentity(row.ended_at),
  };
}

function nullableIdentity(value: unknown): string | null {
  return value == null ? null : String(value);
}

function firstNumber(row: Record<string, unknown>, ...keys: string[]): number | undefined {
  for (const key of keys) {
    const value = finiteNumber(row[key]);
    if (value !== undefined) return value;
  }
  return undefined;
}

function firstString(row: Record<string, unknown>, ...keys: string[]): string | null {
  for (const key of keys) if (typeof row[key] === 'string') return row[key];
  return null;
}

function numberFields<const Key extends string>(row: Record<string, unknown>, ...keys: Key[]) {
  return Object.fromEntries(keys.map((key) => [key, finiteNumber(row[key]) ?? null])) as Record<
    Key,
    number | null
  >;
}

function stringFields<const Key extends string>(row: Record<string, unknown>, ...keys: Key[]) {
  return Object.fromEntries(keys.map((key) => [key, firstString(row, key)])) as Record<
    Key,
    string | null
  >;
}

function booleanFields<const Key extends string>(row: Record<string, unknown>, ...keys: Key[]) {
  return Object.fromEntries(
    keys.map((key) => [key, typeof row[key] === 'boolean' ? row[key] : null])
  ) as Record<Key, boolean | null>;
}

function oneOf<const Value extends string>(
  value: unknown,
  allowed: readonly Value[]
): Value | null {
  return typeof value === 'string' && allowed.includes(value as Value) ? (value as Value) : null;
}

function decodeTripTags(value: unknown, vehicleId: string): TripTag[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((tag) => {
    if (!isRecord(tag) || typeof tag.id !== 'string' || typeof tag.name !== 'string') return [];
    return [
      {
        id: tag.id,
        name: tag.name,
        vehicle_id: typeof tag.vehicle_id === 'string' ? tag.vehicle_id : vehicleId,
        color_token:
          oneOf(tag.color_token, ['neutral', 'info', 'success', 'warning', 'danger']) ?? 'accent',
        created_by: firstString(tag, 'created_by') ?? '',
        created_at: firstString(tag, 'created_at') ?? '',
        updated_at: firstString(tag, 'updated_at') ?? '',
      },
    ];
  });
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function normalizeCoordinateValue(latRaw: unknown, lngRaw: unknown) {
  const values = [finiteValue(latRaw), finiteValue(lngRaw)];
  if (values.some((value) => value === undefined) || values.every((value) => value === 0)) return null;
  return { lat: values[0]!, lng: values[1]! };
}

function formatCoordinateLabel(value: { lat: number; lng: number } | null) {
  return value && [value.lat, value.lng].map((coordinate) => coordinate.toFixed(4)).join(', ');
}

function normalizeCoordinateLabel(value: unknown) {
  const label = typeof value === 'string' ? value.trim() : '';
  return label.length && !/^0+(?:\.0+)?\s*,\s*0+(?:\.0+)?$/.test(label) ? label : null;
}

export function finiteNumber(value: unknown): number | undefined { return finiteValue(value); }

export function decodeListVehicles(res: { vehicles: Vehicle[] }) {
  return listEnvelope(res, 'vehicles');
}

async function downloadBody(response: Response, fallbackName: string) {
  const attachment = /filename="([^"]+)"/i.exec(response.headers.get('content-disposition') || '');
  const fileName = attachment === null ? fallbackName : attachment[1]!;
  return { blob: await response.blob(), fileName };
}

export async function decodeDownloadVehicleIngestionCapture(res: Response) {
  return downloadBody(res, 'panda-capture.jsonl');
}

export function decodeListVehicleMembers(res: { members: VehicleMember[] }) {
  return listEnvelope(res, 'members');
}

export function decodeListVehicleInvites(res: { invites: VehicleInvite[] }) {
  return listEnvelope(res, 'invites');
}

export function decodeListUsers(res: { users: AdminUserRecord[] }) {
  return listEnvelope(res, 'users');
}

export function decodeListAdminVehicleOptions(res: { vehicles: AdminVehicleOption[] }) {
  return listEnvelope(res, 'vehicles');
}

export function decodeListAccountInvitations(res: { invitations: AccountInvitation[] }) {
  return listEnvelope(res, 'invitations');
}

export function decodeListUserVehicleMemberships(res: { memberships: AdminUserMembership[] }) {
  return listEnvelope(res, 'memberships');
}

export function decodeListUserInvites(res: { invites: AdminUserInvite[] }) {
  return listEnvelope(res, 'invites');
}

export function decodeListPlaces(response: { places: Place[] }) {
  return listEnvelope(response, 'places');
}

export async function decodeDownloadBackupArtifact(res: Response, artifactId: string) {
  return downloadBody(res, `backup-${artifactId}`);
}

export function decodeGetBatteryMileage(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map(project({
    ts: textual(() => new Date().toISOString(), 'ts'),
    odometer_mi: numeric(null, 'odometer_mi'), usable_kwh: numeric(null, 'usable_kwh'),
    range_mi: numeric(null, 'range_mi'), projected_max_range_mi: numeric(null, 'projected_max_range_mi'),
    degradation_pct: numeric(null, 'degradation_pct'),
  })) satisfies BatteryMileagePoint[];
}

export function decodeListTrips(
  response: PaginatedResponse<unknown> & { data?: unknown[]; limit?: number; offset?: number },
  page: number,
  perPage: number
) {
  const normalized = normalizePaginated(response, page, perPage);
  return {
    ...normalized,
    items: normalized.items.map(normalizeTrip),
  } satisfies PaginatedResponse<Trip>;
}

export function decodeGetTrip(trip: unknown) {
  return normalizeTrip(trip);
}

export function decodeGetTripDetailData(response: TripDetailResponse) {
  return {
    ...response,
    trip: normalizeTrip(response.trip),
  } satisfies TripDetailResponse;
}

export function decodeGetSpeedProfile(rows: Array<{ elapsed_s?: number; speed_mph?: number; ts?: string; value?: number | null }>) {
  const decode = project({ elapsed_s: (row, index) => finiteValue(row.elapsed_s) ?? index * 60, speed_mph: numeric(0, 'speed_mph', 'value') });
  return requireRows(rows).map(decode);
}

export function decodeGetTripPowerProfile(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map(project({
    ts: textual(() => new Date().toISOString(), 'ts'), power_kw: numeric(null, 'power_kw'),
    regen_power_kw: numeric(null, 'regen_power_kw'), speed_mph: numeric(null, 'speed_mph'),
    battery_level: numeric(null, 'battery_level'), estimated_net_power_kw: numeric(null, 'estimated_net_power_kw'),
    power_source: optionalText('power_source', ['direct', 'estimated_soc', 'unavailable']),
  })) as TripPowerPoint[];
}

export function decodeGetTripDetailSeries(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map(project({
    ts: textual(() => new Date().toISOString(), 'ts'), speed_mph: numeric(null, 'speed_mph'),
    power_kw: numeric(null, 'power_kw'), regen_power_kw: numeric(null, 'regen_power_kw'), battery_level: numeric(null, 'battery_level'),
    outside_temp_c: numeric(null, 'outside_temp_c'), cabin_temp_c: numeric(null, 'cabin_temp_c'), driver_temp_c: numeric(null, 'driver_temp_c'),
    hvac_active: (row) => typeof row.hvac_active === 'boolean' ? row.hvac_active : null,
    tire_fl_psi: numeric(null, 'tire_fl_psi'), tire_fr_psi: numeric(null, 'tire_fr_psi'), tire_rl_psi: numeric(null, 'tire_rl_psi'), tire_rr_psi: numeric(null, 'tire_rr_psi'),
    estimated_net_power_kw: numeric(null, 'estimated_net_power_kw'), power_source: optionalText('power_source', ['direct', 'estimated_soc', 'unavailable']),
  })) as TripDetailSeriesPoint[];
}

export function decodeListChargeSessions(
  response: PaginatedResponse<unknown> & { data?: unknown[]; limit?: number; offset?: number },
  page: number,
  perPage: number
) {
  const normalized = normalizePaginated(response, page, perPage);
  return {
    ...normalized,
    items: normalized.items.map(normalizeChargeSession),
  } satisfies PaginatedResponse<ChargeSession>;
}

export function decodeGetChargeSession(response: unknown) {
  return normalizeChargeSession(
    isRecord(response) && 'session' in response ? response.session : response
  );
}

export function decodeUpdateChargeSession(response: unknown) {
  return normalizeChargeSession(
    isRecord(response) && 'session' in response ? response.session : response
  );
}

export function decodeGetChargeCurve(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map(project({
    minutes_elapsed: numeric(null, 'minutes_elapsed'), soc_pct: numeric(0, 'soc_pct', 'soc'), power_kw: numeric(0, 'power_kw', 'charge_rate_kw'),
    sample_source: optionalText('sample_source'), power_method: optionalText('power_method'),
  })) satisfies ChargeCurvePoint[];
}

export function decodeGetChargeCurveAnalysis(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map(project({
    session_id: textual('', 'session_id'), minutes_elapsed: numeric(null, 'minutes_elapsed'), soc_pct: numeric(null, 'soc_pct', 'soc'),
    charge_rate_kw: numeric(0, 'charge_rate_kw', 'power_kw'), charger_type: textual(null, 'charger_type'),
    sample_source: textual('telemetry', 'sample_source'), power_method: optionalText('power_method'),
  })) as ChargeCurveAnalysisPoint[];
}

export function decodeGetChargingSummary(summary: {
  total_energy_kwh?: number;
  total_kwh?: number;
  total_cost_usd?: number;
  session_count?: number;
  home_kwh?: number;
  away_kwh?: number;
  unknown_location_kwh?: number;
  ac_kwh?: number;
  ac_l2_kwh?: number;
  dc_kwh?: number;
  by_type?: { ac_kwh?: number; ac_l2_kwh?: number; dc_kwh?: number };
  charging_cycles?: number | null;
  charging_efficiency_pct?: number | null;
  total_energy_used_kwh?: number | null;
  max_charge_limit_pct?: number | null;
  max_charge_rate_kw?: number | null;
  typed_session_count?: number;
  known_cost_session_count?: number;
  unknown_cost_session_count?: number;
  free_session_count?: number;
  total_range_added_km?: number | null;
  rivian_paid_total_usd?: number | null;
  network_breakdown?: Array<{
    network_vendor: string | null;
    session_count: number;
    energy_kwh: number | null;
    cost_usd: number | null;
    free_sessions: number;
  }>;
  weekly?: Array<{ week_start: string; kwh?: number; energy_kwh?: number; sessions: number }>;
}) {
  const defaults = {
    total_cost_usd: null, session_count: 0, home_kwh: 0, away_kwh: 0, unknown_location_kwh: 0,
    charging_cycles: null, charging_efficiency_pct: null, total_energy_used_kwh: null, max_charge_limit_pct: null,
    max_charge_rate_kw: null, typed_session_count: 0, known_cost_session_count: 0, unknown_cost_session_count: 0,
    free_session_count: 0, total_range_added_km: null, rivian_paid_total_usd: null, network_breakdown: [],
  };
  const direct = Object.fromEntries(Object.entries(defaults).map(([key, fallback]) => [key, summary[key as keyof typeof defaults] ?? fallback]));
  const energy = (name: 'ac_kwh' | 'ac_l2_kwh' | 'dc_kwh') => summary[name] ?? summary.by_type?.[name] ?? 0;
  return {
    ...direct,
    total_energy_kwh: summary.total_energy_kwh ?? summary.total_kwh ?? 0,
    ac_kwh: energy('ac_kwh') + energy('ac_l2_kwh'), dc_kwh: energy('dc_kwh'),
    weekly: (summary.weekly ?? []).map(project({ week_start: (row) => row.week_start as string, energy_kwh: (row) => (row.energy_kwh ?? row.kwh ?? 0) as number, sessions: (row) => row.sessions as number })),
  } as ChargingSummary;
}

export function decodeGetChargingChartSeries(response: {
  daily?: Array<{ day_local?: string; day_start?: string; total_energy_kwh?: number | null; session_count?: number | null }>;
  daily_sessions?: Array<Record<string, unknown>>;
}) {
  const timestamp = () => new Date().toISOString();
  return {
    daily: (response.daily ?? []).map(project({ day_local: textual('', 'day_local'), day_start: textual(timestamp, 'day_start'), total_energy_kwh: numeric(0, 'total_energy_kwh'), session_count: numeric(0, 'session_count') })),
    daily_sessions: (response.daily_sessions ?? []).map(project({
      session_id: textual('', 'session_id'), day_local: textual('', 'day_local'), day_start: textual(timestamp, 'day_start'), started_at: textual(timestamp, 'started_at'),
      energy_added_kwh: numeric(null, 'energy_added_kwh'), cost_usd: numeric(null, 'cost_usd'), charger_type: textual(null, 'charger_type'), location_name: textual(null, 'location_name'),
    })),
  } as ChargingChartSeries;
}

export function decodeGetLiveSession(result: LiveSession | undefined) {
  return result ?? null;
}

export function decodeGetEfficiencySummary(summary: {
  avg_wh_per_mi: number;
  p10_wh_per_mi: number;
  p90_wh_per_mi: number;
  total_miles: number;
  efficiency_miles: number;
  coverage_percent: number;
}) {
  return {
    avg: summary.avg_wh_per_mi,
    p10: summary.p10_wh_per_mi,
    p90: summary.p90_wh_per_mi,
    total_miles: summary.total_miles,
    efficiency_miles: summary.efficiency_miles,
    coverage_percent: summary.coverage_percent,
  } satisfies EfficiencySummary;
}

export function decodeGetEfficiencyByMode(rows: Array<{ drive_mode: string; avg_wh_per_mi: number; trip_count: number }>) {
  return requireRows(rows).map(project({
    drive_mode: (row) => row.drive_mode as string, avg_efficiency: (row) => row.avg_wh_per_mi as number,
    trip_count: (row) => row.trip_count as number, p10_efficiency: () => 0, p90_efficiency: () => 0,
  })) satisfies EfficiencyByMode[];
}

export function decodeGetMetricCatalog(response: { metrics: MetricCatalogEntry[] }) {
  return listEnvelope(response, 'metrics');
}
function normalizePaginated<T>(
  response: PaginatedResponse<T> & { data?: T[]; limit?: number; offset?: number },
  requestedPage: number,
  requestedPerPage: number,
): PaginatedResponse<T> {
  const { items, data, total = 0, page, offset } = response;
  const perPage = [response.per_page, response.limit, requestedPerPage].find((value) => value != null)!;
  const resolvedPage = page ?? (offset === undefined ? requestedPage : 1 + Math.floor(offset / perPage));
  return { items: items ?? data ?? [], total: total ?? 0, page: resolvedPage, per_page: perPage };
}

export function buildTimeframeParams(from: string | null, to: string | null, lifetime = false): Record<string, string> {
  const range = Object.fromEntries(Object.entries({ from, to }).filter((entry): entry is [string, string] => Boolean(entry[1])));
  return lifetime || Object.keys(range).length === 0 ? { lifetime: 'true' } : range;
}

export function buildTripTagFilterParams(filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }): Record<string, string | number> {
  const ordered = (filters?.tagIds ?? []).slice().sort();
  const ids = ordered.filter((id, index) => index === 0 || id !== ordered[index - 1]);
  const tagged = ids.length ? { tag_ids: ids.join(','), tag_match: filters?.tagMatch === 'any' ? 'any' : 'all' } : null;
  return tagged ?? (filters?.untagged ? { untagged: 1 } : {});
}

function listEnvelope<T, Key extends string>(response: Record<Key, T[]>, key: Key): T[] {
  if (!isRecord(response)) throw new Error('Expected a response object');
  const list = response[key];
  if (list == null) return [];
  if (!Array.isArray(list)) throw new Error('Expected an array in response');
  return list;
}

function requireRows<T extends object>(rows: T[]): T[] {
  if (!Array.isArray(rows) || !rows.every(isRecord)) throw new Error('Expected response rows');
  return rows;
}

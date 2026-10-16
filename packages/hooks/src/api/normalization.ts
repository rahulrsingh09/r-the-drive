import type {
  Vehicle,
  Trip,
  TripPowerPoint,
  TripPowerSource,
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
import type { LiveSession } from './apiClientTransport';
import { liveFields } from './chargingSessionFields';

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
  const lat = finiteNumber(latRaw);
  const lng = finiteNumber(lngRaw);
  if (lat === undefined || lng === undefined) return null;
  if (lat === 0 && lng === 0) return null;
  return { lat, lng };
}

function formatCoordinateLabel(value: { lat: number; lng: number } | null) {
  return value ? `${value.lat.toFixed(4)}, ${value.lng.toFixed(4)}` : null;
}

function normalizeCoordinateLabel(value: unknown) {
  if (typeof value !== 'string') return null;
  const label = value.trim();
  if (!label) return null;
  if (/^0+(?:\.0+)?\s*,\s*0+(?:\.0+)?$/.test(label)) return null;
  return label;
}

export function finiteNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

export function decodeListVehicles(res: { vehicles: Vehicle[] }) {
  return listEnvelope(res, 'vehicles');
}

export async function decodeDownloadVehicleIngestionCapture(res: Response) {
  const disposition = res.headers.get('content-disposition') ?? '';
  const fileNameMatch = disposition.match(/filename="([^"]+)"/i);
  return {
    blob: await res.blob(),
    fileName: fileNameMatch?.[1] ?? 'panda-capture.jsonl',
  };
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
  const disposition = res.headers.get('content-disposition') ?? '';
  const fileNameMatch = disposition.match(/filename="([^"]+)"/i);
  return {
    blob: await res.blob(),
    fileName: fileNameMatch?.[1] ?? `backup-${artifactId}`,
  };
}

export function decodeGetBatteryMileage(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map((row) => ({
    ts: typeof row.ts === 'string' ? row.ts : new Date().toISOString(),
    odometer_mi: finiteNumber(row.odometer_mi) ?? null,
    usable_kwh: finiteNumber(row.usable_kwh) ?? null,
    range_mi: finiteNumber(row.range_mi) ?? null,
    projected_max_range_mi: finiteNumber(row.projected_max_range_mi) ?? null,
    degradation_pct: finiteNumber(row.degradation_pct) ?? null,
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

export function decodeGetSpeedProfile(
  rows: Array<{ elapsed_s?: number; speed_mph?: number; ts?: string; value?: number | null }>
) {
  return requireRows(rows)
    .map((row, index) => ({
      elapsed_s: finiteNumber(row.elapsed_s) ?? index * 60,
      speed_mph: finiteNumber(row.speed_mph) ?? finiteNumber(row.value) ?? 0,
    }))
    .filter((row) => Number.isFinite(row.elapsed_s) && Number.isFinite(row.speed_mph));
}

export function decodeGetTripPowerProfile(rows: Array<Record<string, unknown>>) {
  return requireRows(rows)
    .map((row) => {
      const powerSource = tripPowerSource(row.power_source);
      return {
        ts: typeof row.ts === 'string' ? row.ts : new Date().toISOString(),
        power_kw: finiteNumber(row.power_kw) ?? null,
        regen_power_kw: finiteNumber(row.regen_power_kw) ?? null,
        speed_mph: finiteNumber(row.speed_mph) ?? null,
        battery_level: finiteNumber(row.battery_level) ?? null,
        estimated_net_power_kw: finiteNumber(row.estimated_net_power_kw) ?? null,
        ...(powerSource ? { power_source: powerSource } : {}),
      };
    })
    .filter((row) => typeof row.ts === 'string') satisfies TripPowerPoint[];
}

export function decodeGetTripDetailSeries(rows: Array<Record<string, unknown>>) {
  return requireRows(rows)
    .map((row) => {
      const powerSource = tripPowerSource(row.power_source);
      return {
        ts: typeof row.ts === 'string' ? row.ts : new Date().toISOString(),
        speed_mph: finiteNumber(row.speed_mph) ?? null,
        power_kw: finiteNumber(row.power_kw) ?? null,
        regen_power_kw: finiteNumber(row.regen_power_kw) ?? null,
        battery_level: finiteNumber(row.battery_level) ?? null,
        outside_temp_c: finiteNumber(row.outside_temp_c) ?? null,
        cabin_temp_c: finiteNumber(row.cabin_temp_c) ?? null,
        driver_temp_c: finiteNumber(row.driver_temp_c) ?? null,
        hvac_active: typeof row.hvac_active === 'boolean' ? row.hvac_active : null,
        tire_fl_psi: finiteNumber(row.tire_fl_psi) ?? null,
        tire_fr_psi: finiteNumber(row.tire_fr_psi) ?? null,
        tire_rl_psi: finiteNumber(row.tire_rl_psi) ?? null,
        tire_rr_psi: finiteNumber(row.tire_rr_psi) ?? null,
        estimated_net_power_kw: finiteNumber(row.estimated_net_power_kw) ?? null,
        ...(powerSource ? { power_source: powerSource } : {}),
      };
    })
    .filter((row) => typeof row.ts === 'string') satisfies TripDetailSeriesPoint[];
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
  return requireRows(rows).map((row) => ({
    minutes_elapsed: finiteNumber(row.minutes_elapsed) ?? null,
    soc_pct: finiteNumber(row.soc_pct) ?? finiteNumber(row.soc) ?? 0,
    power_kw: finiteNumber(row.power_kw) ?? finiteNumber(row.charge_rate_kw) ?? 0,
    ...(typeof row.sample_source === 'string' ? { sample_source: row.sample_source } : {}),
    ...(typeof row.power_method === 'string' ? { power_method: row.power_method } : {}),
  })) satisfies ChargeCurvePoint[];
}

export function decodeGetChargeCurveAnalysis(rows: Array<Record<string, unknown>>) {
  return requireRows(rows).map((row) => ({
    session_id: typeof row.session_id === 'string' ? row.session_id : '',
    minutes_elapsed: finiteNumber(row.minutes_elapsed) ?? null,
    soc_pct: finiteNumber(row.soc_pct) ?? finiteNumber(row.soc) ?? null,
    charge_rate_kw: finiteNumber(row.charge_rate_kw) ?? finiteNumber(row.power_kw) ?? 0,
    charger_type:
      typeof row.charger_type === 'string'
        ? (row.charger_type as ChargeCurveAnalysisPoint['charger_type'])
        : null,
    sample_source: typeof row.sample_source === 'string' ? row.sample_source : 'telemetry',
    ...(typeof row.power_method === 'string' ? { power_method: row.power_method } : {}),
  })) satisfies ChargeCurveAnalysisPoint[];
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
  const acKwh =
    (summary.ac_kwh ?? summary.by_type?.ac_kwh ?? 0) +
    (summary.ac_l2_kwh ?? summary.by_type?.ac_l2_kwh ?? 0);
  const dcKwh = summary.dc_kwh ?? summary.by_type?.dc_kwh ?? 0;
  return {
    total_energy_kwh: summary.total_energy_kwh ?? summary.total_kwh ?? 0,
    total_cost_usd: summary.total_cost_usd ?? null,
    session_count: summary.session_count ?? 0,
    home_kwh: summary.home_kwh ?? 0,
    away_kwh: summary.away_kwh ?? 0,
    unknown_location_kwh: summary.unknown_location_kwh ?? 0,
    ac_kwh: acKwh,
    dc_kwh: dcKwh,
    charging_cycles: summary.charging_cycles ?? null,
    charging_efficiency_pct: summary.charging_efficiency_pct ?? null,
    total_energy_used_kwh: summary.total_energy_used_kwh ?? null,
    max_charge_limit_pct: summary.max_charge_limit_pct ?? null,
    max_charge_rate_kw: summary.max_charge_rate_kw ?? null,
    typed_session_count: summary.typed_session_count ?? 0,
    known_cost_session_count: summary.known_cost_session_count ?? 0,
    unknown_cost_session_count: summary.unknown_cost_session_count ?? 0,
    free_session_count: summary.free_session_count ?? 0,
    total_range_added_km: summary.total_range_added_km ?? null,
    rivian_paid_total_usd: summary.rivian_paid_total_usd ?? null,
    network_breakdown: summary.network_breakdown ?? [],
    weekly: (summary.weekly ?? []).map((week) => ({
      week_start: week.week_start,
      energy_kwh: week.energy_kwh ?? week.kwh ?? 0,
      sessions: week.sessions,
    })),
  } satisfies ChargingSummary;
}

export function decodeGetChargingChartSeries(response: {
  daily?: Array<{
    day_local?: string;
    day_start?: string;
    total_energy_kwh?: number | null;
    session_count?: number | null;
  }>;
  daily_sessions?: Array<Record<string, unknown>>;
}) {
  return {
    daily: (response.daily ?? []).map((point) => ({
      day_local: typeof point.day_local === 'string' ? point.day_local : '',
      day_start: typeof point.day_start === 'string' ? point.day_start : new Date().toISOString(),
      total_energy_kwh: finiteNumber(point.total_energy_kwh) ?? 0,
      session_count: finiteNumber(point.session_count) ?? 0,
    })),
    daily_sessions: (response.daily_sessions ?? []).map((row) => ({
      session_id: typeof row.session_id === 'string' ? row.session_id : '',
      day_local: typeof row.day_local === 'string' ? row.day_local : '',
      day_start: typeof row.day_start === 'string' ? row.day_start : new Date().toISOString(),
      started_at: typeof row.started_at === 'string' ? row.started_at : new Date().toISOString(),
      energy_added_kwh: finiteNumber(row.energy_added_kwh) ?? null,
      cost_usd: finiteNumber(row.cost_usd) ?? null,
      charger_type:
        typeof row.charger_type === 'string'
          ? (row.charger_type as ChargingChartSeries['daily_sessions'][number]['charger_type'])
          : null,
      location_name: typeof row.location_name === 'string' ? row.location_name : null,
    })),
  } satisfies ChargingChartSeries;
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

export function decodeGetEfficiencyByMode(
  rows: Array<{
    drive_mode: string;
    avg_wh_per_mi: number;
    trip_count: number;
  }>
) {
  return requireRows(rows).map((row) => ({
    drive_mode: row.drive_mode,
    avg_efficiency: row.avg_wh_per_mi,
    p10_efficiency: 0,
    p90_efficiency: 0,
    trip_count: row.trip_count,
  })) satisfies EfficiencyByMode[];
}

export function decodeGetMetricCatalog(response: { metrics: MetricCatalogEntry[] }) {
  return listEnvelope(response, 'metrics');
}
function normalizePaginated<T>(
  response: PaginatedResponse<T> & { data?: T[]; limit?: number; offset?: number },
  requestedPage: number,
  requestedPerPage: number
): PaginatedResponse<T> {
  const perPage = response.per_page ?? response.limit ?? requestedPerPage;
  const page =
    response.page ??
    (response.offset !== undefined ? Math.floor(response.offset / perPage) + 1 : requestedPage);
  return {
    items: response.items ?? response.data ?? [],
    total: response.total ?? 0,
    page,
    per_page: perPage,
  };
}

export function buildTimeframeParams(from: string | null, to: string | null, lifetime = false) {
  if (lifetime || (!from && !to)) {
    return { lifetime: 'true' };
  }

  return {
    ...(from ? { from } : {}),
    ...(to ? { to } : {}),
  };
}

export function buildTripTagFilterParams(filters?: {
  tagIds?: string[];
  tagMatch?: 'all' | 'any';
  untagged?: boolean;
}) {
  const tagIds = [...new Set(filters?.tagIds ?? [])].sort();
  if (tagIds.length > 0) {
    return {
      tag_ids: tagIds.join(','),
      tag_match: filters?.tagMatch === 'any' ? 'any' : 'all',
    };
  }
  return filters?.untagged ? { untagged: 1 } : {};
}

function tripPowerSource(value: unknown): TripPowerSource | undefined {
  return value === 'direct' || value === 'estimated_soc' || value === 'unavailable'
    ? value
    : undefined;
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

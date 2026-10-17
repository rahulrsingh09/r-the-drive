import type { ChargeSessionRow, TripRow } from '@panda/ui/tables';
import { commonTripTags } from './tagPickerModel';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import { formatDriveMode, getDriveModeBadgeClass } from '@panda/ui/lib/driveMode';
import { formatEfficiency, formatPercent } from '@panda/ui/lib/utils';

export function tripPresentation(trip: TripRow) {
  const endpoints = {
    start: trip.start_place ?? trip.start_address,
    end: trip.end_place ?? trip.end_address,
  };
  const readings: { efficiency?: string; soc?: [string, string] } = {};
  if (trip.efficiency_wh_mi != null) readings.efficiency = formatEfficiency(trip.efficiency_wh_mi);
  if (trip.soc_start != null && trip.soc_end != null)
    readings.soc = [formatPercent(trip.soc_start), formatPercent(trip.soc_end)];
  return {
    ...endpoints,
    ...readings,
    title: `${endpoints.start ?? 'Start'} → ${endpoints.end ?? 'Destination'}`,
    date: formatAppDateTime(trip.started_at),
    mode: trip.drive_mode
      ? { label: formatDriveMode(trip.drive_mode), className: getDriveModeBadgeClass(trip.drive_mode) }
      : null,
  };
}

export function routeKeyRows(
  routes: Array<{ id: string; index: number }>,
  trips: Record<string, TripRow>,
  highlighted: string | null
) {
  if (routes.length < 2) return [];
  const labels = new Map<string, ReturnType<typeof tripPresentation>>();
  for (const route of routes) {
    const trip = trips[route.id];
    if (trip && !labels.has(route.id)) labels.set(route.id, tripPresentation(trip));
  }
  return routes.map(({ id, index }) => ({
    id,
    index,
    title: labels.get(id)?.title ?? `Route ${index + 1}`,
    date: labels.get(id)?.date,
    selected: highlighted === id,
    nextHighlight: highlighted === id ? null : id,
  }));
}

export function chargingSessionValues(session: ChargeSessionRow) {
  const active = session.ended_at == null;
  const energy = active
    ? (session.live_total_charged_kwh ?? session.energy_added_kwh)
    : session.energy_added_kwh;
  const duration =
    active && session.live_time_elapsed_seconds != null
      ? session.live_time_elapsed_seconds / 60
      : session.duration_min;
  const power = active ? (session.live_power_kw ?? session.peak_power_kw) : session.peak_power_kw;
  const socEnd = active ? (session.live_soc_pct ?? session.soc_end) : session.soc_end;
  const range = active
    ? (session.live_range_added_km ?? session.range_added_km)
    : session.range_added_km;
  const explicit = { ac: 'ac', ac_l2: 'ac', dc: 'dc', dcfc: 'dc' } as const;
  const normalized = session.charger_type?.toLowerCase() as keyof typeof explicit;
  let type: 'ac' | 'dc' | null = explicit[normalized] ?? null;
  if (!type) {
    const network = (session.network_vendor ?? '').trim().toLowerCase();
    const dcNetwork = new Set(['tesla', 'rivian', 'electrify america', 'evgo']);
    type = dcNetwork.has(network)
      ? 'dc'
      : power != null && Number.isFinite(power)
        ? power < 20
          ? 'ac'
          : 'dc'
        : null;
  }
  return { active, energy, duration, power, socEnd, range, type };
}

export function selectedTripRows(
  ids: string[],
  registry: Record<string, TripRow>,
  page: TripRow[] = []
) {
  const pageIndex = new Map<string, TripRow>();
  page.forEach((trip) => {
    if (!pageIndex.has(trip.id)) pageIndex.set(trip.id, trip);
  });
  const selected: TripRow[] = [];
  for (const id of ids) {
    const trip = registry[id] ?? pageIndex.get(id);
    if (trip) selected.push(trip);
  }
  return selected;
}
export function tripTagBatch(ids: string[], registry: Record<string, TripRow>, page: TripRow[]) {
  const selectedTrips = selectedTripRows(ids, registry, page);
  const allSelectedTripsLoaded = selectedTrips.length === ids.length;
  const commonTagIds = allSelectedTripsLoaded ? commonTripTags(selectedTrips) : [];
  const common = new Set(commonTagIds);
  const hasMixedTags =
    allSelectedTripsLoaded &&
    selectedTrips.length > 1 &&
    selectedTrips.some((trip) => {
      const tags = new Set(trip.tags?.map((tag) => tag.id));
      return tags.size !== common.size || [...common].some((id) => !tags.has(id));
    });
  return { selectedTrips, allSelectedTripsLoaded, commonTagIds, hasMixedTags };
}

type MapRoute = { trip_id: string; coordinates: Array<[number, number]> };
export function projectMapRoutes(source: MapRoute[], selection: string[]) {
  const ids = new Set<string>(),
    selected = new Set(selection);
  const key: Array<{ id: string; index: number }> = [];
  const routes = source.map((route, index) => {
    ids.add(route.trip_id);
    if (selected.has(route.trip_id)) key.push({ id: route.trip_id, index });
    return { id: route.trip_id, track: route.coordinates.map(([lng, lat]) => ({ lat, lng })) };
  });
  return { routes, selectedRouteIds: selection.filter((id) => ids.has(id)), key };
}

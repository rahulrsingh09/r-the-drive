import type { TripRow } from '@panda/ui/tables';
import { formatDuration, formatEfficiency, formatMiles } from '@panda/ui/lib/utils';

export type TripStatKind = 'miles' | 'count' | 'efficiency' | 'duration';

const TRIP_STAT_BY_METRIC: Record<string, TripStatKind> = {
  trip_miles: 'miles',
  total_trips: 'count',
  avg_efficiency: 'efficiency',
  avg_trip_duration: 'duration',
};

export function getTripStatKind(metric: string | null | undefined): TripStatKind | null {
  if (!metric) return null;
  return TRIP_STAT_BY_METRIC[metric] ?? null;
}

export function computeTripStat(stat: TripStatKind, trips: TripRow[]): number | null {
  if (!trips.length) return null;
  const totals = { miles: 0, minutes: 0, weightedEnergy: 0, weightedMiles: 0 };
  for (const trip of trips) {
    totals.miles += trip.distance_mi;
    totals.minutes += trip.duration_min;
    if (trip.distance_mi > 0 && trip.efficiency_wh_mi != null) {
      totals.weightedMiles += trip.distance_mi;
      totals.weightedEnergy += trip.distance_mi * trip.efficiency_wh_mi;
    }
  }
  switch (stat) {
    case 'count': return trips.length;
    case 'miles': return totals.miles;
    case 'duration': return totals.minutes / trips.length;
    default: return totals.weightedMiles > 0 ? totals.weightedEnergy / totals.weightedMiles : null;
  }
}

export function formatTripStat(stat: TripStatKind, value: number | null): string {
  const formatters = { miles: formatMiles, duration: formatDuration, efficiency: formatEfficiency, count: (count: number) => count.toFixed(0) };
  return value !== null && Number.isFinite(value) ? formatters[stat](value) : '-';
}

export function formatTripSelectionLabel(count: number) {
  return `${count} trip${count === 1 ? '' : 's'} selected`;
}

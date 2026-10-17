import { formatCurrency, formatDuration, formatEfficiency, formatKwh, formatMiles } from '@panda/ui/lib/utils';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import type { useTrips, useChargeSessions } from '@panda/hooks';

type Trips = ReturnType<typeof useTrips>;
type Charges = ReturnType<typeof useChargeSessions>;
function activityPhase(query: { isPlaceholderData: boolean; isLoading: boolean; isError: boolean }, errorsFirst: boolean) {
  const pending = query.isLoading || query.isPlaceholderData;
  const states = errorsFirst ? ['error', 'loading'] as const : ['loading', 'error'] as const;
  for (const state of states) {
    if (state === 'error' ? query.isError : pending) return state;
  }
  return 'ready';
}

export function activityPresentation(trips: Trips, charges: Charges) {
  const tripPhase = activityPhase(trips, false);
  const chargePhase = activityPhase(charges, true);
  const rows = (tripPhase === 'ready' ? trips.data?.items ?? [] : []).map(trip => ({
    id: trip.id,
    start: trip.start_place || trip.start_address || 'Start',
    end: trip.end_place || trip.end_address || 'Destination',
    time: formatAppDateTime(trip.started_at), duration: formatDuration(trip.duration_min),
    distance: formatMiles(trip.distance_mi), efficiency: formatEfficiency(trip.efficiency_wh_mi),
  }));
  const session = chargePhase === 'ready' ? charges.data?.items?.[0] : undefined;
  const charge = session ? {
    id: session.id, name: session.location_name || 'Charging session',
    time: formatAppDateTime(session.started_at), energy: formatKwh(session.energy_added_kwh), cost: formatCurrency(session.cost_usd),
  } : undefined;
  return { trips: { phase: tripPhase, rows }, charges: { phase: chargePhase, charge } };
}

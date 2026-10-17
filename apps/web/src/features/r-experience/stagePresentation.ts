import type { Vehicle, VehicleStatus } from '@panda/types';
import { resolveVehicleArtwork } from '@panda/hooks';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import { formatEfficiencyValue, getEfficiencyUnitLabel, formatMiles } from '@panda/ui/lib/utils';
import { isConfiguredR2, vehicleReading, vehicleState } from './readings';

type PeriodTotals = { distance: number | null; trips: number | null; efficiency: number | null };

export function stagePresentation(vehicle: Vehicle, status: VehicleStatus | null | undefined, totals: PeriodTotals, dark: boolean) {
  const artwork = resolveVehicleArtwork(vehicle.images, vehicle.model, 'vehicle-card');
  const distance = formatMiles(totals.distance);
  const separator = distance.indexOf(' ');
  const configured = isConfiguredR2(vehicle);
  const identity = [vehicle.model, vehicle.trim, vehicle.color].reduce<string[]>((parts, part) => {
    if (part) parts.push(part);
    return parts;
  }, []);
  return {
    configured,
    title: vehicle.display_name || vehicle.model,
    subtitle: configured ? 'R2 Performance · Catalina Cove' : identity.join(' · '),
    state: vehicleState(status),
    artwork: { source: dark && artwork.dark != null ? artwork.dark : artwork.light, fallback: artwork.fallback },
    distance: { value: separator < 0 ? distance : distance.slice(0, separator), unit: separator < 0 ? '—' : distance.slice(separator + 1) || '—' },
    trips: totals.trips == null ? '—' : totals.trips.toLocaleString(),
    efficiency: { value: formatEfficiencyValue(totals.efficiency), unit: getEfficiencyUnitLabel() },
    odometer: formatMiles(vehicleReading(status, 'odometer_miles')),
    snapshot: status?.last_updated ? `Snapshot ${formatAppDateTime(status.last_updated)}` : 'Waiting for a vehicle snapshot',
  };
}

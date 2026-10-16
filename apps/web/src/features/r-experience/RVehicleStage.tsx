import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight } from 'lucide-react';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { formatEfficiencyValue, getEfficiencyUnitLabel, formatMiles } from '@panda/ui/lib/utils';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import { isConfiguredR2, vehicleReading, vehicleState } from './readings';
import { AuthenticatedVehicleArtwork, resolveVehicleArtwork } from '@panda/hooks';
import { useDocumentTheme } from '@panda/ui/hooks';
import { RVehicleOrbit } from './RVehicleOrbit';
import { RVehicleDetails } from './RVehicleDetails';
import { RVehicleLockStatus } from './RVehicleLockStatus';
import './r-overview.css';

export function RVehicleStage({ vehicle, status, distance, trips, efficiency, headingLevel = 'h1' }: {
  vehicle: Vehicle; status: VehicleStatus | null | undefined;
  distance: number | null; trips: number | null; efficiency: number | null;
  headingLevel?: 'h1' | 'h2';
}) {
  const Heading = headingLevel;
  const configured = isConfiguredR2(vehicle);
  const dark = useDocumentTheme();
  const artwork = resolveVehicleArtwork(vehicle.images, vehicle.model, 'vehicle-card');
  const [distanceValue, ...distanceUnit] = formatMiles(distance).split(' ');
  return (
    <section className="r-vehicle-stage" aria-label="Vehicle snapshot">
      <header className="r-scene-heading">
        <div><Heading>{vehicle.display_name || vehicle.model}</Heading><p>{configured ? 'R2 Performance · Catalina Cove' : [vehicle.model, vehicle.trim, vehicle.color].filter(Boolean).join(' · ')}</p>
          {vehicle.is_demo && <small>Demo vehicle · Illustrative data</small>}</div>
        <div className="r-car-status"><span>{vehicleState(status)}</span>
          <RVehicleLockStatus status={status} /></div>
      </header>
      {configured ? <RVehicleOrbit key={vehicle.id} vehicleId={vehicle.id} /> : <div className="r-static-car">
        <AuthenticatedVehicleArtwork source={dark ? artwork.dark ?? artwork.light : artwork.light} fallbackSource={artwork.fallback}
          alt={`${vehicle.model} vehicle artwork`} />
      </div>}
      <div className="r-trip-instruments" aria-label="Trip totals for selected period">
        <Link className="r-trip-instrument" to="/trips">
          <span className="r-card-top">Trip distance <ArrowUpRight /></span>
          <strong>{distanceValue}</strong><span className="r-metric-unit">{distanceUnit.join(' ') || '—'}</span>
        </Link>
        <Link className="r-trip-instrument" to="/trips">
          <span className="r-card-top">Total trips <ArrowUpRight /></span>
          <strong>{trips == null ? '—' : trips.toLocaleString()}</strong><span className="r-metric-unit">in this period</span>
        </Link>
        <Link className="r-efficiency-card" to="/efficiency">
          <span className="r-card-top">Efficiency <ArrowUpRight /></span>
          <strong>{formatEfficiencyValue(efficiency)}</strong><span className="r-metric-unit">{getEfficiencyUnitLabel()}</span>
        </Link>
      </div>
      <RVehicleDetails status={status} vehicle={vehicle} />
      <footer className="r-snapshot-foot">
        <span>Odometer {formatMiles(vehicleReading(status, 'odometer_miles'))}</span>
        <span>{status?.last_updated ? `Snapshot ${formatAppDateTime(status.last_updated)}` : 'Waiting for a vehicle snapshot'}</span>
      </footer>
    </section>
  );
}

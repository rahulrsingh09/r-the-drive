import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight } from 'lucide-react';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { formatEfficiencyValue, getEfficiencyUnitLabel, formatMiles } from '@panda/ui/lib/utils';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import { isConfiguredR2, vehicleReading, vehicleState } from './readings';
import { AuthenticatedVehicleArtwork, resolveVehicleArtwork } from '@panda/hooks';
import { useDocumentTheme } from '@panda/ui/hooks';
import { RVehicleAngles } from './RVehicleOrbit';
import { RVehicleDetails } from './RVehicleDetails';
import { RVehicleLockStatus } from './RVehicleLockStatus';
import { spinLabel, vehicleConfiguration } from '../now/nowModel';
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
  // Only the owner's own Rivian images turn. The bundled orbit frames show an R1S body.
  const angles = (dark ? artwork.angles?.dark : artwork.angles?.light) ?? [];
  const turn = configured && angles.length >= 2;
  const [distanceValue, ...distanceUnit] = formatMiles(distance).split(' ');
  return (
    <section className="r-vehicle-stage" aria-label="Vehicle snapshot">
      <header className="r-scene-heading">
        <div><Heading>{vehicle.display_name || vehicle.model}</Heading><p>{vehicleConfiguration(vehicle)}</p>
          {vehicle.is_demo && <small>Demo vehicle · Illustrative data</small>}</div>
        <div className="r-car-status"><span>{vehicleState(status)}</span>
          <RVehicleLockStatus status={status} /></div>
      </header>
      {turn ? <RVehicleAngles key={vehicle.id} label={spinLabel(vehicle)} angles={angles} fallbackSource={artwork.fallback} /> : <div className="r-static-car">
        <AuthenticatedVehicleArtwork source={(dark ? artwork.dark ?? artwork.light : artwork.light) ?? angles[0]?.url} fallbackSource={artwork.fallback}
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

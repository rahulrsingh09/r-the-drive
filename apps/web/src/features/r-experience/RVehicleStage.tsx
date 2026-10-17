import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight } from 'lucide-react';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { AuthenticatedVehicleArtwork } from '@panda/hooks';
import { useDocumentTheme } from '@panda/ui/hooks';
import { RVehicleOrbit } from './RVehicleOrbit';
import { RVehicleDetails } from './RVehicleDetails';
import { RVehicleLockStatus } from './RVehicleLockStatus';
import { stagePresentation } from './stagePresentation';
import './r-overview.css';

export function RVehicleStage({ vehicle, status, distance, trips, efficiency, headingLevel = 'h1' }: {
  vehicle: Vehicle; status: VehicleStatus | null | undefined;
  distance: number | null; trips: number | null; efficiency: number | null;
  headingLevel?: 'h1' | 'h2';
}) {
  const Heading = headingLevel;
  const dark = useDocumentTheme();
  const view = stagePresentation(vehicle, status, { distance, trips, efficiency }, dark);
  return (
    <section className="r-vehicle-stage" aria-label="Vehicle snapshot">
      <header className="r-scene-heading">
        <div><Heading>{view.title}</Heading><p>{view.subtitle}</p>
          {vehicle.is_demo && <small>Demo vehicle · Illustrative data</small>}</div>
        <div className="r-car-status"><span>{view.state}</span>
          <RVehicleLockStatus status={status} /></div>
      </header>
      {view.configured ? <RVehicleOrbit key={vehicle.id} vehicleId={vehicle.id} /> : <div className="r-static-car">
        <AuthenticatedVehicleArtwork source={view.artwork.source} fallbackSource={view.artwork.fallback}
          alt={`${vehicle.model} vehicle artwork`} />
      </div>}
      <div className="r-trip-instruments" aria-label="Trip totals for selected period">
        <Link className="r-trip-instrument" to="/trips">
          <span className="r-card-top">Trip distance <ArrowUpRight /></span>
          <strong>{view.distance.value}</strong><span className="r-metric-unit">{view.distance.unit}</span>
        </Link>
        <Link className="r-trip-instrument" to="/trips">
          <span className="r-card-top">Total trips <ArrowUpRight /></span>
          <strong>{view.trips}</strong><span className="r-metric-unit">in this period</span>
        </Link>
        <Link className="r-efficiency-card" to="/efficiency">
          <span className="r-card-top">Efficiency <ArrowUpRight /></span>
          <strong>{view.efficiency.value}</strong><span className="r-metric-unit">{view.efficiency.unit}</span>
        </Link>
      </div>
      <RVehicleDetails status={status} vehicle={vehicle} />
      <footer className="r-snapshot-foot">
        <span>Odometer {view.odometer}</span>
        <span>{view.snapshot}</span>
      </footer>
    </section>
  );
}

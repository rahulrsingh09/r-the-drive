import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight, CircleGauge, Thermometer, CircleDot, Download } from 'lucide-react';
import { type Vehicle, type VehicleStatus } from '@panda/types';
import { useVehicleHealth } from '@panda/hooks';
import { formatAltitude, formatMph, formatMiles, formatPercent, formatTemp } from '@panda/ui/lib/utils';
import { formatDriveMode } from '@panda/ui/lib/driveMode';
import { vehicleDetails } from './vehiclePresentation';
import { currentVehicleRecord } from './telemetryModel';
import { vehicleReading } from './readings';
import './r-full-dashboard.css';

export function RVehicleDetails({ status, vehicle }: { status: VehicleStatus | null | undefined; vehicle: Vehicle }) {
  const healthQuery = useVehicleHealth(status?.ota_current_version ? null : vehicle.id);
  const health = currentVehicleRecord(healthQuery, vehicle.id);
  const { soc, limit, charging, chargeLabel, chargeNote, tireRange, tireWarning, tireNote,
    currentVersion, hasUpdate, softwareNote } = vehicleDetails(status, vehicle, health);
  return <div className="r-vehicle-details" aria-label="Vehicle details">
    <div className="r-energy-summary" aria-label="Energy and charging details" data-charging={charging || undefined}>
      <Link to="/battery" className="r-soc-summary"><span>State of charge <ArrowUpRight /></span><strong>{soc == null ? '—' : formatPercent(soc)}</strong>
        <span className="r-soc-track" aria-hidden="true"><span style={{ width: `${soc == null ? 0 : Math.min(100, Math.max(0, soc))}%` }} /></span></Link>
      <Link to="/battery" aria-label="Estimated remaining range"><span>Est. range <ArrowUpRight /></span><strong>{formatMiles(vehicleReading(status, 'range_miles'))}</strong><small>Remaining estimate</small></Link>
      <Link to="/charging"><span>Charging <ArrowUpRight /></span><strong>{chargeLabel}</strong>
        <small>{chargeNote} · Limit {limit == null ? '—' : formatPercent(limit)}</small></Link>
    </div>
    <div className="r-status-glances">
      <Link to="/vehicle-health" className="r-status-glance"><span><CircleGauge />Drive mode<ArrowUpRight /></span>
        <strong>{status?.drive_mode ? formatDriveMode(status.drive_mode) : 'Unknown mode'}</strong>
        <small>Speed {formatMph(vehicleReading(status, 'speed_mph'))} · Alt. {formatAltitude(vehicleReading(status, 'altitude_m'))}</small></Link>
      <Link to="/vehicle-health" className="r-status-glance"><span><Thermometer />Cabin<ArrowUpRight /></span>
        <strong>{formatTemp(vehicleReading(status, 'cabin_temp_c'))}</strong>
        <small>Outside {formatTemp(vehicleReading(status, 'outside_temp_c'))}</small></Link>
      <Link to="/vehicle-health" className="r-status-glance" data-attention={tireWarning || undefined}><span><CircleDot />Tires<ArrowUpRight /></span>
        <strong>{tireRange}</strong><small>{tireNote}</small></Link>
      <Link to="/vehicle-health" className="r-status-glance" data-update={Boolean(hasUpdate) || undefined}><span><Download />Software<ArrowUpRight /></span>
        <strong>{currentVersion || 'Version unknown'}</strong>
        <small>{softwareNote}</small></Link>
    </div>
  </div>;
}

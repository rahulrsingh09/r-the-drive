import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight, CircleGauge, Thermometer, CircleDot, Download } from 'lucide-react';
import { isVehicleCharging, type Vehicle, type VehicleStatus } from '@panda/types';
import { useVehicleHealth } from '@panda/hooks';
import { isVehiclePluggedIn } from '@panda/dashboards';
import { formatAltitude, formatDuration, formatMph, formatMiles, formatPercent, formatPressure, formatTemp } from '@panda/ui/lib/utils';
import { formatDriveMode } from '@panda/ui/lib/driveMode';
import { getTireHealthTone } from '@panda/ui/lib/vehicleTires';
import { vehicleReading } from './readings';
import './r-full-dashboard.css';

export function RVehicleDetails({ status, vehicle }: { status: VehicleStatus | null | undefined; vehicle: Vehicle }) {
  const healthQuery = useVehicleHealth(status?.ota_current_version ? null : vehicle.id);
  const health = !healthQuery.isPlaceholderData && healthQuery.data?.vehicle_id === vehicle.id ? healthQuery.data : null;
  const currentVersion = status?.ota_current_version ?? health?.current_software_version;
  const availableVersion = status?.ota_available_version;
  const hasUpdate = availableVersion && availableVersion !== '0.0.0' && availableVersion !== currentVersion;
  const updateStatus = status?.ota_status ?? status?.software_update_status ?? status?.ota_current_status;
  const activeUpdate = updateStatus && !['idle', 'unknown'].includes(updateStatus.toLowerCase());
  const soc = vehicleReading(status, 'battery_level');
  const limit = vehicleReading(status, 'battery_limit');
  const charging = isVehicleCharging(status);
  const connected = isVehiclePluggedIn(status);
  const connectionKnown = connected || status?.charger_state?.toLowerCase() === 'disconnected' || status?.charger_status === 'chrgr_sts_not_connected';
  const tires = (['fl', 'fr', 'rl', 'rr'] as const).map(corner => {
    const psi = vehicleReading(status, `tire_${corner}_psi`);
    return { psi, tone: getTireHealthTone({ psi, status: status?.[`tire_${corner}_status`], targetPsi: vehicle.target_tire_pressure_psi }) };
  });
  const tireWarning = tires.some(tire => ['warning', 'danger'].includes(tire.tone));
  const pressures = tires.map(tire => tire.psi);
  const tireRange = pressures.every((psi): psi is number => psi !== null)
    ? Math.min(...pressures) === Math.max(...pressures) ? formatPressure(pressures[0]!) : `${formatPressure(Math.min(...pressures)).split(' ')[0]}–${formatPressure(Math.max(...pressures))}`
    : 'Readings incomplete';
  return <div className="r-vehicle-details" aria-label="Vehicle details">
    <div className="r-energy-summary" aria-label="Energy and charging details" data-charging={charging || undefined}>
      <Link to="/battery" className="r-soc-summary"><span>State of charge <ArrowUpRight /></span><strong>{soc == null ? '—' : formatPercent(soc)}</strong>
        <span className="r-soc-track" aria-hidden="true"><span style={{ width: `${soc == null ? 0 : Math.min(100, Math.max(0, soc))}%` }} /></span></Link>
      <Link to="/battery" aria-label="Estimated remaining range"><span>Est. range <ArrowUpRight /></span><strong>{formatMiles(vehicleReading(status, 'range_miles'))}</strong><small>Remaining estimate</small></Link>
      <Link to="/charging"><span>Charging <ArrowUpRight /></span><strong>{charging ? 'Charging' : !connectionKnown ? 'Unknown' : connected ? 'Connected' : 'Not charging'}</strong>
        <small>{charging ? `To limit ${formatDuration(vehicleReading(status, 'time_to_end_of_charge_min'))}` : !connectionKnown ? 'Connection unknown' : connected ? 'Plugged in' : 'Not connected'} · Limit {limit == null ? '—' : formatPercent(limit)}</small></Link>
    </div>
    <div className="r-status-glances">
      <Link to="/vehicle-health" className="r-status-glance"><span><CircleGauge />Drive mode<ArrowUpRight /></span>
        <strong>{status?.drive_mode ? formatDriveMode(status.drive_mode) : 'Unknown mode'}</strong>
        <small>Speed {formatMph(vehicleReading(status, 'speed_mph'))} · Alt. {formatAltitude(vehicleReading(status, 'altitude_m'))}</small></Link>
      <Link to="/vehicle-health" className="r-status-glance"><span><Thermometer />Cabin<ArrowUpRight /></span>
        <strong>{formatTemp(vehicleReading(status, 'cabin_temp_c'))}</strong>
        <small>Outside {formatTemp(vehicleReading(status, 'outside_temp_c'))}</small></Link>
      <Link to="/vehicle-health" className="r-status-glance" data-attention={tireWarning || undefined}><span><CircleDot />Tires<ArrowUpRight /></span>
        <strong>{tireRange}</strong><small>{tireWarning ? 'Check tire pressure' : tires.every(tire => tire.tone === 'success') ? 'All four in range' : 'Check readings'}</small></Link>
      <Link to="/vehicle-health" className="r-status-glance" data-update={Boolean(hasUpdate) || undefined}><span><Download />Software<ArrowUpRight /></span>
        <strong>{currentVersion || 'Version unknown'}</strong>
        <small>{hasUpdate ? `Update ${availableVersion} available` : activeUpdate ? updateStatus.replaceAll('_', ' ') : availableVersion != null || updateStatus?.toLowerCase() === 'idle' ? 'No update reported' : 'Update status unknown'}</small></Link>
    </div>
  </div>;
}

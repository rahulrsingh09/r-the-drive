import { isVehicleCharging, type Vehicle, type VehicleStatus, type VehicleHealth } from '@panda/types';
import { isVehiclePluggedIn } from '@panda/dashboards';
import { formatDuration, formatPressure } from '@panda/ui/lib/utils';
import { getTireHealthTone } from '@panda/ui/lib/vehicleTires';
import { vehicleReading } from './readings';

export function vehicleDetails(status: VehicleStatus | null | undefined, vehicle: Vehicle, health: VehicleHealth | null) {
  const tires = { complete: true, allHealthy: true, warning: false, low: Infinity, high: -Infinity };
  for (const corner of ['fl', 'fr', 'rl', 'rr'] as const) {
    const psi = vehicleReading(status, `tire_${corner}_psi`);
    const tone = getTireHealthTone({ psi, status: status?.[`tire_${corner}_status`], targetPsi: vehicle.target_tire_pressure_psi });
    tires.complete &&= psi !== null;
    tires.allHealthy &&= tone === 'success';
    tires.warning ||= tone === 'warning' || tone === 'danger';
    if (psi !== null) {
      tires.low = Math.min(tires.low, psi);
      tires.high = Math.max(tires.high, psi);
    }
  }
  let tireRange = 'Readings incomplete';
  if (tires.complete) {
    tireRange = formatPressure(tires.high);
    if (tires.low !== tires.high) tireRange = `${formatPressure(tires.low).split(' ')[0]}–${tireRange}`;
  }
  const currentVersion = status?.ota_current_version ?? health?.current_software_version;
  const available = status?.ota_available_version;
  const update = status?.ota_status ?? status?.software_update_status ?? status?.ota_current_status;
  const updateName = update?.toLowerCase();
  const hasUpdate = Boolean(available && available !== '0.0.0' && available !== currentVersion);
  let softwareNote = 'Update status unknown';
  if (available != null || updateName === 'idle') softwareNote = 'No update reported';
  if (update && updateName !== 'idle' && updateName !== 'unknown') softwareNote = update.replaceAll('_', ' ');
  if (hasUpdate) softwareNote = `Update ${available} available`;

  const charging = isVehicleCharging(status);
  const connected = isVehiclePluggedIn(status);
  const disconnected = status?.charger_state?.toLowerCase() === 'disconnected' || status?.charger_status === 'chrgr_sts_not_connected';
  const phase = charging ? 'charging' : connected ? 'connected' : disconnected ? 'disconnected' : 'unknown';
  const labels = {
    charging: ['Charging', `To limit ${formatDuration(vehicleReading(status, 'time_to_end_of_charge_min'))}`],
    connected: ['Connected', 'Plugged in'],
    disconnected: ['Not charging', 'Not connected'],
    unknown: ['Unknown', 'Connection unknown'],
  } as const;
  return {
    soc: vehicleReading(status, 'battery_level'), limit: vehicleReading(status, 'battery_limit'), charging,
    chargeLabel: labels[phase][0], chargeNote: labels[phase][1],
    tireRange, tireWarning: tires.warning,
    tireNote: tires.warning ? 'Check tire pressure' : tires.allHealthy ? 'All four in range' : 'Check readings',
    currentVersion, hasUpdate, softwareNote,
  };
}

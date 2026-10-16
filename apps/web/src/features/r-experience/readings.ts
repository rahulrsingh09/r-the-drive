import { isVehicleCharging, type Vehicle, type VehicleStatus } from '@panda/types';

export function finiteReading(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function vehicleReading(status: VehicleStatus | null | undefined, field: keyof VehicleStatus) {
  const availability = status?.field_availability?.[field];
  if (availability?.availability === 'never_seen' || availability?.reason_code === 'invalid_sensor') return null;
  if (/^tire_.._psi$/.test(field) && status?.[field.replace('_psi', '_valid') as keyof VehicleStatus] === false) return null;
  return finiteReading(status?.[field]);
}

export function isConfiguredR2(vehicle: Pick<Vehicle, 'model' | 'trim' | 'color'> | undefined) {
  const normalize = (value: string | null | undefined) => (value ?? '').replace(/[^a-z0-9]/gi, '').toLowerCase();
  return normalize(vehicle?.model) === 'r2'
    && normalize(vehicle?.trim).includes('performance');
}

export function vehicleState(status: VehicleStatus | null | undefined) {
  if (!status) return 'Waiting for telemetry';
  if (status.telemetry_stale) return 'Last known readings';
  if (isVehicleCharging(status)) return 'Charging';
  if (/drive|go/.test(status.power_state?.toLowerCase() ?? '')) return 'Driving';
  if (status.power_state?.toLowerCase().includes('sleep')) return 'Asleep';
  return status.power_state === 'ready' ? 'Parked' : 'Vehicle snapshot';
}

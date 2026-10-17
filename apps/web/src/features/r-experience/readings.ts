import { isVehicleCharging, type Vehicle, type VehicleStatus } from '@panda/types';
import { admittedTelemetry } from './telemetryModel';

export function finiteReading(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function vehicleReading(status: VehicleStatus | null | undefined, field: keyof VehicleStatus) {
  const reading = admittedTelemetry(status, field);
  const tire = field.match(/^tire_(..)_psi$/);
  const valid = tire ? status?.[`tire_${tire[1]}_valid` as keyof VehicleStatus] : undefined;
  return valid === false ? null : finiteReading(reading);
}

export function isConfiguredR2(vehicle: Pick<Vehicle, 'model' | 'trim' | 'color'> | undefined) {
  const identity = [vehicle?.model, vehicle?.trim].map(value =>
    value?.match(/[a-z0-9]+/gi)?.join('').toLowerCase() ?? '');
  return identity[0] === 'r2' && identity[1]!.includes('performance');
}

export function vehicleState(status: VehicleStatus | null | undefined) {
  if (status == null) return 'Waiting for telemetry';
  const power = status.power_state?.toLowerCase() ?? '';
  const labels = [
    status.telemetry_stale && 'Last known readings',
    isVehicleCharging(status) && 'Charging',
    (power.includes('drive') || power.includes('go')) && 'Driving',
    power.includes('sleep') && 'Asleep',
    status.power_state === 'ready' && 'Parked',
    'Vehicle snapshot',
  ];
  return labels.find((label): label is string => typeof label === 'string')!;
}

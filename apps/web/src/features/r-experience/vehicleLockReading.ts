import type { VehicleStatus } from '@panda/types';

export function vehicleLockReading(status: VehicleStatus | null | undefined): boolean | null {
  const availability = status?.field_availability?.doors_locked;
  if (availability?.availability === 'never_seen' || availability?.reason_code === 'invalid_sensor') return null;
  return typeof status?.doors_locked === 'boolean' ? status.doors_locked : null;
}

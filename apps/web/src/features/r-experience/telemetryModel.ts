import type { VehicleStatus } from '@panda/types';

export function admittedTelemetry(status: VehicleStatus | null | undefined, field: keyof VehicleStatus): unknown {
  if (status == null) return undefined;
  const { availability, reason_code } = status.field_availability?.[field] ?? {};
  const blocked = availability === 'never_seen' || reason_code === 'invalid_sensor';
  return blocked ? undefined : status[field];
}

export function currentVehicleRecord<T extends { vehicle_id: string }>(
  query: { isPlaceholderData: boolean; data?: T | null | undefined },
  vehicleId: string | null | undefined,
): T | null {
  if (query.isPlaceholderData || query.data == null) return null;
  return query.data.vehicle_id === vehicleId ? query.data : null;
}

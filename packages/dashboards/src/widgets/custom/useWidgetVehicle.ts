import { useAuth, useCurrentVehicleStatus, useVehicles } from '@panda/hooks';
import type { Vehicle } from '@panda/types';

function vehicleIdentity(vehicle: Vehicle | undefined) {
  const explicitDemo = vehicle?.is_demo;
  const demo = explicitDemo == null
    ? (vehicle?.rivian_vehicle_id?.startsWith('demo-') ?? false)
    : explicitDemo;
  return {
    images: vehicle?.images,
    vehicleName: vehicle?.display_name,
    vehicleModel: vehicle?.model,
    isDemoVehicle: demo,
    targetTirePressurePsi: vehicle?.target_tire_pressure_psi,
  };
}

export function useWidgetVehicle(requestedId: string | null) {
  const auth = useAuth();
  const id = requestedId ?? auth.defaultVehicleId;
  const status = useCurrentVehicleStatus(id);
  const catalog = useVehicles();
  const identities = new Map<string, ReturnType<typeof vehicleIdentity>>();
  for (const vehicle of catalog.data ?? []) {
    if (!identities.has(vehicle.id)) identities.set(vehicle.id, vehicleIdentity(vehicle));
  }
  const identity = id == null ? undefined : identities.get(id);
  return { ...identity ?? vehicleIdentity(undefined), status: status.data };
}

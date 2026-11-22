import { useEffect } from 'react';
import type { Vehicle } from '@panda/types';
import { useAuth } from './useAuth';
import { useAuthReady } from './useAuthState';
import { useVehicles } from './useVehicles';

const noVehicles: Vehicle[] = [];

export function useResolvedVehicleSelection() {
  const authReady = useAuthReady();
  const activeVehicleId = useAuth((state) => state.activeVehicleId);
  const defaultVehicleId = useAuth((state) => state.defaultVehicleId);
  const vehiclesQuery = useVehicles();
  const allVehicles = vehiclesQuery.data ?? noVehicles;
  const vehicles = allVehicles.some((vehicle) => !vehicle.is_demo)
    ? allVehicles.filter((vehicle) => !vehicle.is_demo)
    : allVehicles;
  const validateShown = (id: string | null) => id && vehicles.some((vehicle) => vehicle.id === id) ? id : null;
  const validateOwned = (id: string | null) => id && allVehicles.some((vehicle) => vehicle.id === id) ? id : null;
  const validatedActiveVehicleId = validateShown(activeVehicleId);
  const validatedDefaultVehicleId = validateShown(defaultVehicleId);
  const fallbackId = vehicles[0]?.id ?? null;
  useEffect(() => {
    if (!authReady || !vehiclesQuery.isFetched) return;
    const auth = useAuth.getState();
    if (activeVehicleId && !validateOwned(activeVehicleId)) auth.setActiveVehicleId(null);
    if (defaultVehicleId && !validateOwned(defaultVehicleId)) auth.setDefaultVehicleId(fallbackId);
  }, [authReady, vehiclesQuery.isFetched, activeVehicleId, defaultVehicleId, allVehicles, fallbackId]);
  return {
    authReady,
    effectiveVehicleId: validatedActiveVehicleId ?? validatedDefaultVehicleId ?? fallbackId,
    validatedActiveVehicleId,
    validatedDefaultVehicleId,
    vehicles,
    vehicleSelectionReady: !authReady || vehiclesQuery.isFetched,
    vehiclesQuery,
  };
}

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
  const vehicles = vehiclesQuery.data ?? noVehicles;
  const validate = (id: string | null) => id && vehicles.some((vehicle) => vehicle.id === id) ? id : null;
  const validatedActiveVehicleId = validate(activeVehicleId);
  const validatedDefaultVehicleId = validate(defaultVehicleId);
  const fallbackId = vehicles[0]?.id ?? null;
  useEffect(() => {
    if (!authReady || !vehiclesQuery.isFetched) return;
    const auth = useAuth.getState();
    if (activeVehicleId && !validatedActiveVehicleId) auth.setActiveVehicleId(null);
    if (defaultVehicleId && !validatedDefaultVehicleId) auth.setDefaultVehicleId(fallbackId);
  }, [authReady, vehiclesQuery.isFetched, activeVehicleId, defaultVehicleId, validatedActiveVehicleId, validatedDefaultVehicleId, fallbackId]);
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

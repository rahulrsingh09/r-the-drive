import { useApiQuery, queryPolicies } from './queryRuntime';

export function useSocHistory(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['battery', 'soc', vehicleId, from, to, lifetime], 'getSoc', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 2 * 60 * 1000
  });
}

export function useRangeHistory(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['battery', 'range', vehicleId, from, to, lifetime], 'getRange', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 2 * 60 * 1000
  });
}

export function usePhantomDrain(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['battery', 'phantom', vehicleId, from, to, lifetime], 'getPhantomDrain', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000
  });
}

export function usePhantomDrainPeriods(
  vehicleId: string | null,
  from: string | null,
  to: string | null,
  limit = 250,
  minDurationHours = 6,
) {
  const lifetime = !from && !to;
  return useApiQuery(['battery', 'phantom-periods', vehicleId, from, to, lifetime, limit, minDurationHours], 'getIdleDrainPeriods', [vehicleId!, from, to, limit, minDurationHours, lifetime], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 5 * 60 * 1000
  });
}

export function useParkedEnergy(
  vehicleId: string | null,
  from: string | null,
  to: string | null,
) {
  return useApiQuery(['battery', 'parked-energy', vehicleId, from, to], 'getParkedEnergy', [vehicleId!, from, to], [vehicleId], {
    staleTime: 60 * 1000,
    refetchInterval: 2 * 60 * 1000
  });
}

export function useDegradation(vehicleId: string | null, from: string | null = null, to: string | null = null) {
  const lifetime = !from && !to;
  return useApiQuery(['battery', 'degradation', vehicleId, from, to, lifetime], 'getDegradation', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 60 * 60 * 1000
  });
}

export function useBatteryHealth(vehicleId: string | null) {
  return useApiQuery(['battery', 'health', vehicleId], 'getBatteryHealth', [vehicleId!], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 60 * 60 * 1000
  });
}

export function useBatteryMileage(vehicleId: string | null, from: string | null, to: string | null) {
  const lifetime = !from && !to;
  return useApiQuery(['battery', 'mileage', vehicleId, from, to, lifetime], 'getBatteryMileage', [vehicleId!, from, to, lifetime], [vehicleId], {
    ...queryPolicies.history,
    staleTime: 60 * 60 * 1000
  });
}

import { useCallback, useMemo } from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { useResolvedVehicleSelection } from '@panda/hooks';
import { createTripTagFilterAdapter, type TripTagSearch } from '../../features/trip-tags/tripTagFilter';

export function useTripPageContext(route: '/trips' | '/efficiency') {
  const search = useSearch({ from: route });
  const navigate = useNavigate();
  const selection = useResolvedVehicleSelection();
  const replaceSearch = useCallback((next: TripTagSearch) => {
    navigate({ to: route, search: next, replace: true });
  }, [navigate, route]);
  const tripTagFilter = useMemo(() => createTripTagFilterAdapter(search, replaceSearch), [replaceSearch, search]);
  const permissions = new Map<string, boolean>();
  for (const vehicle of selection.vehicles) {
    if (!permissions.has(vehicle.id)) {
      permissions.set(vehicle.id, vehicle.membership_role === 'owner' || vehicle.membership_role === 'manager');
    }
  }
  const canManageTripTags = selection.effectiveVehicleId !== null && permissions.get(selection.effectiveVehicleId) === true;
  return { tripTagFilter, canManageTripTags };
}

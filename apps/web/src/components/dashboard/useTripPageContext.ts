import { useMemo } from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { useResolvedVehicleSelection } from '@panda/hooks';
import { createTripTagFilterAdapter } from '../../features/trip-tags/tripTagFilter';

export function useTripPageContext(route: '/trips' | '/efficiency') {
  const search = useSearch({ from: route });
  const navigate = useNavigate();
  const selection = useResolvedVehicleSelection();
  const tripTagFilter = useMemo(() => createTripTagFilterAdapter(search, next => {
    navigate({ to: route, search: next, replace: true });
  }), [navigate, route, search]);
  const membership = selection.vehicles.find(vehicle => vehicle.id === selection.effectiveVehicleId)?.membership_role;
  return { tripTagFilter, canManageTripTags: membership === 'owner' || membership === 'manager' };
}

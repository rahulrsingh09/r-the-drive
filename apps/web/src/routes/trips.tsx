import { useTripPageContext } from '../components/dashboard/useTripPageContext';
import React from 'react';
import { createRoute } from '@tanstack/react-router';
import { rootRoute } from './__root';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { DashboardPageShell } from '../components/dashboard/DashboardPageShell';
import { tripTagSearchSchema } from '../features/trip-tags/tripTagFilter';

export function TripsContent() {
  return <ProtectedRoute><TripsDashboardPage /></ProtectedRoute>;
}

function TripsDashboardPage() {
  const widgetCtx = useTripPageContext('/trips');

  return (
    <DashboardPageShell
      navKey="trips"
      slug="trips"
      title="Trips"
      showEfficiencyDisplayToggle
      widgetCtx={widgetCtx}
    />
  );
}

export const tripsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/trips',
  validateSearch: tripTagSearchSchema,
  component: TripsContent,
});

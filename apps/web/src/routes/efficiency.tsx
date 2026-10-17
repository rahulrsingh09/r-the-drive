import { useTripPageContext } from '../components/dashboard/useTripPageContext';
import React from 'react';
import { createRoute } from '@tanstack/react-router';
import { rootRoute } from './__root';
import { EfficiencyDashboardPage } from '../components/dashboard/EfficiencyDashboardPage';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { tripTagSearchSchema } from '../features/trip-tags/tripTagFilter';

function EfficiencyContent() {
  const widgetCtx = useTripPageContext('/efficiency');

  return (
    <ProtectedRoute>
      <EfficiencyDashboardPage
        navKey="efficiency"
        slug="efficiency"
        title="Efficiency"
        widgetCtx={widgetCtx}
      />
    </ProtectedRoute>
  );
}

export const efficiencyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/efficiency',
  validateSearch: tripTagSearchSchema,
  component: EfficiencyContent,
});

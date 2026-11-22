import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** R has no chart editor; Insights replaces it. Keep old links working. */
export const settingsChartsNewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/charts/new',
  beforeLoad: () => { throw redirect({ to: '/insights', replace: true }); },
});

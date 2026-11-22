import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** R has no chart editor; Insights replaces it. Keep old links working. */
export const settingsChartsEditRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/charts/$chartId',
  beforeLoad: () => { throw redirect({ to: '/insights', replace: true }); },
});

import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** R has no custom dashboards; Insights replaces them. Keep old links working. */
export const adminDashboardsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/dashboards',
  beforeLoad: () => { throw redirect({ to: '/insights', replace: true }); },
});

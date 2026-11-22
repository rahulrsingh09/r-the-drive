import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** R has no custom dashboards; Insights replaces them. Keep old links working. */
export const userDashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/d/$slug',
  beforeLoad: () => { throw redirect({ to: '/insights', replace: true }); },
});

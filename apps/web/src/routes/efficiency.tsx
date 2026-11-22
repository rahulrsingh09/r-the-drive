import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** Efficiency moved to the Efficiency tab of Insights. Keep old links working. */
export const efficiencyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/efficiency',
  beforeLoad: () => { throw redirect({ to: '/insights/$tab', params: { tab: 'efficiency' }, replace: true }); },
});

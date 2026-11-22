import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** Battery moved to the Battery tab of Insights. Keep old links working. */
export const batteryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/battery',
  beforeLoad: () => { throw redirect({ to: '/insights/$tab', params: { tab: 'battery' }, replace: true }); },
});

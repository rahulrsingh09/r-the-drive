import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** Explore was replaced by the Insights tab. Keep old links working. */
export const exploreRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/explore',
  beforeLoad: () => { throw redirect({ to: '/insights', replace: true }); },
});

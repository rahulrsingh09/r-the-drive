import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** The charging list lives in Activity now: /charging opens Activity with the Charges filter. */
export const chargingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/charging',
  beforeLoad: () => {
    throw redirect({ to: '/trips', search: { f: 'charges' }, replace: true });
  },
});

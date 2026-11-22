import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** Parked loss is a card on the Battery tab of Insights. Keep old links working. */
export const batteryPhantomDrainRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/battery/phantom-drain',
  beforeLoad: () => { throw redirect({ to: '/insights/$tab', params: { tab: 'battery' }, replace: true }); },
});

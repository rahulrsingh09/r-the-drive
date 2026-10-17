import { createRoute } from '@tanstack/react-router';
import { rootRoute } from './__root';
import { ActivatePage } from '../components/auth/ActivatePage';

export const activateRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/activate',
  component: ActivatePage,
});

export { ActivatePage };

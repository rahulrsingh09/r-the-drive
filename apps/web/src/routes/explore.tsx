import { createRoute } from '@tanstack/react-router';
import { rootRoute } from './__root';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { RExplore } from '../features/r-experience/RExplore';

export const exploreRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/explore',
  component: () => <ProtectedRoute><RExplore /></ProtectedRoute>,
});

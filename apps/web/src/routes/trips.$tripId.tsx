import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { createRoute } from '@tanstack/react-router';
import { TripDetailContent } from '../features/trips/TripDetailPage';
import { validateActivitySearch } from '../features/trips/activity/activitySearch';
import { rootRoute } from './__root';

export const tripDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/trips/$tripId',
  validateSearch: validateActivitySearch,
  component: TripDetailPage,
});

export { TripDetailContent } from '../features/trips/TripDetailPage';

function TripDetailPage() {
  return <ProtectedRoute><TripDetailContent /></ProtectedRoute>;
}

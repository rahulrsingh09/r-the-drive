import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { createRoute } from '@tanstack/react-router';
import { rootRoute } from './__root';
import { validateActivitySearch } from '../features/trips/activity/activitySearch';
import { TripsContent } from '../features/trips/TripDetailPage';

export { TripsContent } from '../features/trips/TripDetailPage';

export const tripsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/trips',
  validateSearch: validateActivitySearch,
  component: TripsPage,
});

function TripsPage() {
  return <ProtectedRoute><TripsContent /></ProtectedRoute>;
}

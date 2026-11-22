import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { createRoute } from '@tanstack/react-router';
import { ChargeSessionContent } from '../features/charging/ChargeSessionDetailPage';
import { validateActivitySearch } from '../features/trips/activity/activitySearch';
import { rootRoute } from './__root';

export const chargingDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/charging/$sessionId',
  validateSearch: validateActivitySearch,
  component: ChargeSessionDetailPage,
});

export { ChargeSessionContent } from '../features/charging/ChargeSessionDetailPage';

function ChargeSessionDetailPage() {
  return <ProtectedRoute><ChargeSessionContent /></ProtectedRoute>;
}

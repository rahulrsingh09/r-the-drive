import { useParams, useSearch } from '@tanstack/react-router';
import { ActivityContent } from './activity/ActivityPage';

/** Trips list route content: the Activity page with no selection (desktop opens the newest item). */
export function TripsContent() {
  const search = useSearch({ from: '/trips' });
  return <ActivityContent selection={null} search={search} />;
}

/** Trip detail route content: Activity with this trip selected. */
export function TripDetailContent() {
  const { tripId } = useParams({ from: '/trips/$tripId' });
  const search = useSearch({ from: '/trips/$tripId' });
  return <ActivityContent selection={{ kind: 'trip', id: tripId }} search={search} />;
}

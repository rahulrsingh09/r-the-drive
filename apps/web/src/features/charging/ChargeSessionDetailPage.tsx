import { useParams, useSearch } from '@tanstack/react-router';
import { ActivityContent } from '../trips/activity/ActivityPage';

/** Charge detail route content: Activity with this charge selected. */
export function ChargeSessionContent() {
  const { sessionId } = useParams({ from: '/charging/$sessionId' });
  const search = useSearch({ from: '/charging/$sessionId' });
  return <ActivityContent selection={{ kind: 'charge', id: sessionId }} search={search} />;
}

import React from 'react';
import { Link } from '@tanstack/react-router';
import type { ActivityEvent, ActivityFilter, ActivityPeriod } from './activityModel';

/**
 * Activity URL search. Defaults stay out of the URL:
 * f = all, p = 30. `place` opens the charges at one place (from Insights → Places).
 * `rt` opens an inferred road trip detail pane.
 */
export interface ActivitySearch {
  f?: Exclude<ActivityFilter, 'all'>;
  p?: Exclude<ActivityPeriod, '30'>;
  place?: string;
  rt?: string;
}

export function validateActivitySearch(raw: Record<string, unknown>): ActivitySearch {
  const search: ActivitySearch = {};
  if (raw.f === 'trips' || raw.f === 'charges') search.f = raw.f;
  if (raw.p === '7' || raw.p === 7) search.p = '7';
  else if (raw.p === 'all') search.p = 'all';
  if (typeof raw.place === 'string' && raw.place.trim()) search.place = raw.place.trim().slice(0, 200);
  if (typeof raw.rt === 'string' && raw.rt.trim()) search.rt = raw.rt.trim().slice(0, 120);
  return search;
}

export function searchFilter(search: ActivitySearch): ActivityFilter { return search.f ?? 'all'; }
export function searchPeriod(search: ActivitySearch): ActivityPeriod { return search.p ?? '30'; }

export function withFilter(search: ActivitySearch, filter: ActivityFilter): ActivitySearch {
  const rest: ActivitySearch = search.p ? { p: search.p } : {};
  return filter === 'all' ? rest : { ...rest, f: filter };
}

export function withPeriod(search: ActivitySearch, period: ActivityPeriod): ActivitySearch {
  const rest: ActivitySearch = { ...search };
  delete rest.p;
  return period === '30' ? rest : { ...rest, p: period };
}

export function withRoadTrip(search: ActivitySearch, roadTripId: string): ActivitySearch {
  return { ...search, rt: roadTripId };
}

export function withoutRoadTrip(search: ActivitySearch): ActivitySearch {
  const rest: ActivitySearch = { ...search };
  delete rest.rt;
  return rest;
}

/** Link to the detail of a trip, road trip or charge, with the list filters kept. */
export const ActivityItemLink = React.forwardRef<HTMLAnchorElement, {
  item: Pick<ActivityEvent, 'kind' | 'id'>;
  search: ActivitySearch;
  children: React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
  onClick?: () => void;
  'aria-label'?: string;
  'aria-current'?: 'true' | undefined;
}>(function ActivityItemLink({ item, search, children, ...rest }, ref) {
  if (item.kind === 'road_trip') {
    return <Link ref={ref} to="/trips" search={withRoadTrip(search, item.id)} {...rest}>{children}</Link>;
  }
  const cleanSearch = withoutRoadTrip(search);
  return item.kind === 'trip'
    ? <Link ref={ref} to="/trips/$tripId" params={{ tripId: item.id }} search={cleanSearch} {...rest}>{children}</Link>
    : <Link ref={ref} to="/charging/$sessionId" params={{ sessionId: item.id }} search={cleanSearch} {...rest}>{children}</Link>;
});

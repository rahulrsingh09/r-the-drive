import React from 'react';
import { Link } from '@tanstack/react-router';
import type { ActivityEvent, ActivityFilter, ActivityPeriod } from './activityModel';

/**
 * Activity URL search. Defaults stay out of the URL:
 * f = all, p = 30. `place` opens the charges at one place (from Insights → Places).
 */
export interface ActivitySearch {
  f?: Exclude<ActivityFilter, 'all'>;
  p?: Exclude<ActivityPeriod, '30'>;
  place?: string;
}

export function validateActivitySearch(raw: Record<string, unknown>): ActivitySearch {
  const search: ActivitySearch = {};
  if (raw.f === 'trips' || raw.f === 'charges') search.f = raw.f;
  if (raw.p === '7' || raw.p === 7) search.p = '7';
  else if (raw.p === 'all') search.p = 'all';
  if (typeof raw.place === 'string' && raw.place.trim()) search.place = raw.place.trim().slice(0, 200);
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

/** Link to the detail of a trip or a charge, with the list filters kept. */
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
  return item.kind === 'trip'
    ? <Link ref={ref} to="/trips/$tripId" params={{ tripId: item.id }} search={search} {...rest}>{children}</Link>
    : <Link ref={ref} to="/charging/$sessionId" params={{ sessionId: item.id }} search={search} {...rest}>{children}</Link>;
});

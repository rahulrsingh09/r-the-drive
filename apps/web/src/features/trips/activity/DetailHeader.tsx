import { ChevronDown, ChevronLeft, ChevronUp } from 'lucide-react';
import type { ActivityEvent } from './activityModel';
import { ActivityItemLink, type ActivitySearch } from './activitySearch';

export interface DetailPosition {
  index: number;
  total: number;
  newer: ActivityEvent | null;
  older: ActivityEvent | null;
}

/** Detail header: back, "Trip · N of M", and newer/older buttons. */
export function DetailHeader({ kind, position, search, onBack }: {
  kind: 'trip' | 'charge';
  position: DetailPosition | null;
  search: ActivitySearch;
  onBack: () => void;
}) {
  const nav = (item: ActivityEvent | null, label: string, Icon: typeof ChevronUp) => (item ? (
    <ActivityItemLink item={item} search={search} className="r-act-iconbtn" aria-label={label} data-detail-nav={label.toLowerCase()}>
      <Icon aria-hidden="true" />
    </ActivityItemLink>
  ) : (
    <span className="r-act-iconbtn off" aria-hidden="true"><Icon /></span>
  ));
  return (
    <div className="r-act-dhead">
      <button type="button" className="r-act-iconbtn r-act-back" aria-label="Back to activity" onClick={onBack}>
        <ChevronLeft aria-hidden="true" />
      </button>
      <h2 className="r-act-dhead-t" data-detail-position>
        {kind === 'trip' ? 'Trip' : 'Charge'}
        {position ? <span className="muted"> · {position.index + 1} of {position.total}</span> : null}
      </h2>
      <div className="r-act-dnav">
        {nav(position?.newer ?? null, 'Newer', ChevronUp)}
        {nav(position?.older ?? null, 'Older', ChevronDown)}
      </div>
    </div>
  );
}

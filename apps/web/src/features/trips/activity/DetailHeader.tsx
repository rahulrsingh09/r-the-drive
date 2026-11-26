import { ChevronDown, ChevronLeft, ChevronUp } from 'lucide-react';
import type { ActivityEvent, ActivityRoadTrip } from './activityModel';
import { ActivityItemLink, type ActivitySearch } from './activitySearch';

export interface DetailPosition {
  index: number;
  total: number;
  newer: ActivityEvent | null;
  older: ActivityEvent | null;
}

/** Detail header: back, optional parent Road Trip shortcut, "Trip · N of M", and newer/older buttons. */
export function DetailHeader({ kind, position, search, parentRoadTrip = null, onBack }: {
  kind: 'trip' | 'charge' | 'road_trip';
  position: DetailPosition | null;
  search: ActivitySearch;
  parentRoadTrip?: ActivityRoadTrip | null;
  onBack: () => void;
}) {
  const label = kind === 'road_trip' ? 'Road trip' : kind === 'trip' ? 'Trip' : 'Charge';
  const nav = (item: ActivityEvent | null, navLabel: string, Icon: typeof ChevronUp) => (item ? (
    <ActivityItemLink item={item} search={search} className="r-act-iconbtn" aria-label={navLabel} data-detail-nav={navLabel.toLowerCase()}>
      <Icon aria-hidden="true" />
    </ActivityItemLink>
  ) : (
    <span className="r-act-iconbtn off" aria-hidden="true"><Icon /></span>
  ));
  return (
    <div className="r-act-dhead-wrap">
      <div className="r-act-dhead">
        <button type="button" className="r-act-iconbtn r-act-back" aria-label="Back to activity" onClick={onBack}>
          <ChevronLeft aria-hidden="true" />
        </button>
        <h2 className="r-act-dhead-t" data-detail-position>
          {label}
          {position ? <span className="muted"> · {position.index + 1} of {position.total}</span> : null}
        </h2>
        <div className="r-act-dnav">
          {nav(position?.newer ?? null, 'Newer', ChevronUp)}
          {nav(position?.older ?? null, 'Older', ChevronDown)}
        </div>
      </div>
      {parentRoadTrip ? (
        <div className="r-act-rt-backbar">
          <ActivityItemLink item={parentRoadTrip} search={search} className="r-act-rt-backpill" data-road-trip-back>
            <ChevronLeft aria-hidden="true" />
            <span className="r-act-rt-backtitle">Back to {parentRoadTrip.name}</span>
            <span className="muted num">{parentRoadTrip.trips.length} drives · {parentRoadTrip.charges.length} charges</span>
          </ActivityItemLink>
        </div>
      ) : null}
    </div>
  );
}

import React from 'react';
import { Zap } from 'lucide-react';
import { formatAppTime } from '@panda/ui/lib/dateTime';
import { useEfficiencyUnit } from '../../r-experience/efficiencyUnit';
import { CATEGORY_COLOR } from '../../charging/detail/ChargeDetailPane';
import {
  CATEGORY_LABEL, chargeTitle, duration, n0, n1, SHORT_TRIP_MI, tripTitle, usd,
  type ActivityItem, type ActivityRoadTrip,
} from './activityModel';
import { RouteGlyph } from './ActivityCharts';
import { ActivityItemLink, type ActivitySearch } from './activitySearch';

export type ActivityItemSelection = { kind: 'trip' | 'charge' | 'road_trip'; id: string } | null;

function roadTripMeta(item: ActivityRoadTrip) {
  const drivesLabel = `${item.trips.length} ${item.trips.length === 1 ? 'drive' : 'drives'}`;
  const chargesLabel = item.charges.length > 0 ? ` · ${item.charges.length} ${item.charges.length === 1 ? 'charge' : 'charges'}` : '';
  return `${formatAppTime(item.start)} · ${drivesLabel}${chargesLabel}`;
}

function roadTripCostNode(item: ActivityRoadTrip) {
  if (item.cost > 0 && item.unpricedCount > 0) {
    return <>{usd(item.cost)} · <span className="r-act-pill">+{item.unpricedCount}</span></>;
  }
  if (item.cost > 0) return usd(item.cost);
  if (item.unpricedCount > 0) return <span className="r-act-pill">Add cost</span>;
  if (item.charges.length > 0) return 'Free';
  return duration(item.minutes);
}

/** Single row in the Activity feed (road trip with expandable legs, drive, charge, parked, or gap). */
export function ActivityListItem({
  efficiency, item, index, animate, search, selected, expanded, onToggleExpand, chargeRoadTrip, onSelect,
}: {
  efficiency: ReturnType<typeof useEfficiencyUnit>;
  item: ActivityItem;
  index: number;
  animate: boolean;
  search: ActivitySearch;
  selected: ActivityItemSelection;
  expanded: boolean;
  onToggleExpand: (id: string) => void;
  chargeRoadTrip: ActivityRoadTrip | null;
  onSelect: () => void;
}) {
  const style = { '--i': index } as React.CSSProperties;
  const rise = animate ? ' rise' : '';
  const current = selected && selected.kind === item.kind && selected.id === item.id ? 'true' : undefined;
  if (item.kind === 'road_trip') {
    return (
      <div className={`r-act-rt-wrap${rise}`} style={style} data-activity-item="road_trip">
        <ActivityItemLink item={item} search={search} className="r-act-item r-act-rt-main" aria-current={current} onClick={onSelect}>
          <span className="r-act-ig"><RouteGlyph route={item.route} /></span>
          <span className="r-act-main">
            <span className="ttl r-act-rt-ttl">
              <span>{item.name}</span>
              <span className="r-act-rt-pill">Road trip</span>
            </span>
            <span className="meta">{roadTripMeta(item)}</span>
          </span>
          <span className="val num">
            {n1(item.miles)} mi
            <span className="muted">{roadTripCostNode(item)}</span>
          </span>
        </ActivityItemLink>
        <button
          type="button"
          className="r-act-rt-toggle"
          aria-expanded={expanded}
          onClick={() => onToggleExpand(item.id)}
          data-road-trip-legs-toggle={item.id}
        >
          {expanded ? '▴ Hide legs' : `▾ Show legs (${item.events.length})`}
        </button>
        {expanded ? (
          <div className="r-act-rt-legs" data-road-trip-legs={item.id}>
            {item.events.map((ev) => {
              const legCurrent = selected && selected.kind === ev.kind && selected.id === ev.id ? 'true' : undefined;
              if (ev.kind === 'trip') {
                const eff = ev.whPerMile != null && ev.miles >= SHORT_TRIP_MI ? efficiency.format(ev.whPerMile) : null;
                return (
                  <ActivityItemLink key={`trip-${ev.id}`} item={ev} search={search} className="r-act-item r-act-rt-leg" aria-current={legCurrent} onClick={onSelect}>
                    <span className="r-act-ig r-act-rt-leg-ig" aria-hidden="true" />
                    <span className="r-act-main">
                      <span className="ttl">{tripTitle(ev)}</span>
                      <span className="meta">{formatAppTime(ev.start)} · {duration(ev.minutes)}</span>
                    </span>
                    <span className="val num">{n1(ev.miles)} mi<span className="muted">{eff ? `${eff} ${efficiency.label}` : 'Short trip'}</span></span>
                  </ActivityItemLink>
                );
              }
              if (ev.kind === 'charge') {
                return (
                  <ActivityItemLink key={`charge-${ev.id}`} item={ev} search={search} className="r-act-item r-act-rt-leg" aria-current={legCurrent} onClick={onSelect}>
                    <span className="r-act-ig bolt r-act-rt-leg-ig" style={{ color: CATEGORY_COLOR[ev.category] }}>
                      <Zap aria-hidden="true" fill="currentColor" />
                    </span>
                    <span className="r-act-main">
                      <span className="ttl">{chargeTitle(ev)}</span>
                      <span className="meta">
                        {CATEGORY_LABEL[ev.category]} · {formatAppTime(ev.start)}
                        {ev.socStart != null && ev.socEnd != null ? ` · ${Math.round(ev.socStart)}% → ${Math.round(ev.socEnd)}%` : ''}
                      </span>
                    </span>
                    <span className="val num">
                      +{n1(ev.kwh)} kWh
                      <span className="muted">{ev.active ? 'Charging' : ev.cost == null ? <span className="r-act-pill">Add cost</span> : ev.cost === 0 ? 'Free' : usd(ev.cost)}</span>
                    </span>
                  </ActivityItemLink>
                );
              }
              return null;
            })}
          </div>
        ) : null}
      </div>
    );
  }
  if (item.kind === 'trip') {
    const measured = item.whPerMile != null && item.miles >= SHORT_TRIP_MI ? efficiency.format(item.whPerMile) : null;
    return (
      <ActivityItemLink item={item} search={search} className={`r-act-item${rise}`} style={style} aria-current={current} onClick={onSelect} data-activity-item="trip">
        <span className="r-act-ig"><RouteGlyph route={item.route} /></span>
        <span className="r-act-main">
          <span className="ttl">{tripTitle(item)}</span>
          <span className="meta">{formatAppTime(item.start)} · {duration(item.minutes)}</span>
        </span>
        <span className="val num">{n1(item.miles)} mi<span className="muted">{measured ? `${measured} ${efficiency.label}` : 'Short trip'}</span></span>
      </ActivityItemLink>
    );
  }
  if (item.kind === 'charge') {
    return (
      <ActivityItemLink item={item} search={search} className={`r-act-item${rise}`} style={style} aria-current={current} onClick={onSelect} data-activity-item="charge">
        <span className="r-act-ig bolt" style={{ color: CATEGORY_COLOR[item.category] }}><Zap aria-hidden="true" fill="currentColor" /></span>
        <span className="r-act-main">
          <span className="ttl">{chargeTitle(item)}</span>
          <span className="meta">
            {CATEGORY_LABEL[item.category]} · {formatAppTime(item.start)}
            {item.socStart != null && item.socEnd != null ? ` · ${Math.round(item.socStart)}% → ${Math.round(item.socEnd)}%` : ''}
            {chargeRoadTrip ? ` · Road trip: ${chargeRoadTrip.name}` : ''}
          </span>
        </span>
        <span className="val num">
          +{n1(item.kwh)} kWh
          <span className="muted">{item.active ? 'Charging' : item.cost == null ? <span className="r-act-pill">Add cost</span> : item.cost === 0 ? 'Free' : usd(item.cost)}</span>
        </span>
      </ActivityItemLink>
    );
  }
  if (item.kind === 'gap') {
    return (
      <div className={`r-act-item parked gap${rise}`} style={style} data-activity-item="gap">
        <span className="r-act-ig p" aria-hidden="true" />
        <span className="r-act-main"><span className="ttl">Odometer moved {n1(item.miles)} mi with no trip recorded</span></span>
        <span className="val muted num">{formatAppTime(item.end)}</span>
      </div>
    );
  }
  return (
    <div className={`r-act-item parked${rise}`} style={style} data-activity-item="parked">
      <span className="r-act-ig p" aria-hidden="true" />
      <span className="r-act-main"><span className="ttl">Parked {n0(item.hours)} h</span></span>
      <span className="val muted num">{item.loss == null ? '' : item.loss > 0 ? `−${n1(item.loss)}%` : 'No loss'}</span>
    </div>
  );
}

import React from 'react';
import { ChevronDown, ChevronUp, Route, X, Zap } from 'lucide-react';
import { formatAppTime } from '@panda/ui/lib/dateTime';
import { useEfficiencyUnit } from '../../r-experience/efficiencyUnit';
import { CATEGORY_COLOR } from '../../charging/detail/ChargeDetailPane';
import {
  CATEGORY_LABEL, chargeTitle, duration, n1, SHORT_TRIP_MI, tripTitle, usd,
  type ActivityRoadTrip,
} from './activityModel';
import { ActivityItemLink, type ActivitySearch } from './activitySearch';
import type { ActivityItemSelection } from './ActivityListItem';

export type DrawerMode = 'open' | 'collapsed' | 'closed';

/**
 * Mobile-only bottom drawer when viewing a Road Trip or any leg inside one.
 * Picking a leg swaps the map/view above immediately and slides the drawer down
 * so the new route map stays unobstructed; users can reopen, swipe down, or close.
 */
export function RoadTripDrawer({
  roadTrip,
  selected,
  search,
  mode,
  onModeChange,
}: {
  roadTrip: ActivityRoadTrip;
  selected: ActivityItemSelection;
  search: ActivitySearch;
  mode: DrawerMode;
  onModeChange: (next: DrawerMode) => void;
}) {
  const efficiency = useEfficiencyUnit();
  const touchStartY = React.useRef<number | null>(null);

  if (mode === 'closed') {
    return (
      <button
        type="button"
        className="r-act-drawer-fab"
        onClick={() => onModeChange('open')}
        data-road-trip-drawer-reopen
      >
        <ChevronUp aria-hidden="true" />
        <span>Legs ({roadTrip.events.length})</span>
      </button>
    );
  }

  const onTouchStart: React.TouchEventHandler<HTMLDivElement> = (event) => {
    touchStartY.current = event.touches[0]?.clientY ?? null;
  };
  const onTouchEnd: React.TouchEventHandler<HTMLDivElement> = (event) => {
    if (touchStartY.current == null) return;
    const endY = event.changedTouches[0]?.clientY ?? touchStartY.current;
    const delta = endY - touchStartY.current;
    touchStartY.current = null;
    if (delta > 44) {
      onModeChange(mode === 'open' ? 'collapsed' : 'closed');
    } else if (delta < -36 && mode === 'collapsed') {
      onModeChange('open');
    }
  };

  return (
    <section
      className={`r-act-drawer ${mode}`}
      aria-label={`${roadTrip.name} legs`}
      data-road-trip-drawer={mode}
    >
      <div
        className="r-act-drawer-bar"
        onTouchStart={onTouchStart}
        onTouchEnd={onTouchEnd}
      >
        <button
          type="button"
          className="r-act-drawer-toggle"
          aria-expanded={mode === 'open'}
          onClick={() => onModeChange(mode === 'open' ? 'collapsed' : 'open')}
          data-road-trip-drawer-toggle
        >
          {mode === 'open' ? <ChevronDown aria-hidden="true" /> : <ChevronUp aria-hidden="true" />}
          <span className="r-act-drawer-title">
            <b>{roadTrip.name}</b>
            <span className="muted num">
              {roadTrip.trips.length} drives · {roadTrip.charges.length} charges
            </span>
          </span>
        </button>
        <button
          type="button"
          className="r-act-drawer-close"
          aria-label="Close legs drawer"
          onClick={() => onModeChange('closed')}
          data-road-trip-drawer-close
        >
          <X aria-hidden="true" />
        </button>
      </div>

      {mode === 'open' ? (
        <div className="r-act-drawer-list" data-road-trip-drawer-list>
          <ActivityItemLink
            item={roadTrip}
            search={search}
            className="r-act-drawer-row"
            aria-current={selected?.kind === 'road_trip' && selected.id === roadTrip.id ? 'true' : undefined}
            onClick={() => onModeChange('collapsed')}
          >
            <span className="r-act-ig r-act-rt-leg-ig"><Route aria-hidden="true" /></span>
            <span className="r-act-main">
              <span className="ttl">Road trip overview</span>
              <span className="meta">{roadTrip.events.length} legs combined</span>
            </span>
            <span className="val num">
              {n1(roadTrip.miles)} mi
              <span className="muted">{roadTrip.cost > 0 ? usd(roadTrip.cost) : 'Overview'}</span>
            </span>
          </ActivityItemLink>

          {roadTrip.events.map((ev) => {
            const isCurrent = selected?.kind === ev.kind && selected.id === ev.id ? 'true' : undefined;
            if (ev.kind === 'trip') {
              const eff = ev.whPerMile != null && ev.miles >= SHORT_TRIP_MI ? efficiency.format(ev.whPerMile) : null;
              return (
                <ActivityItemLink
                  key={`trip-${ev.id}`}
                  item={ev}
                  search={search}
                  className="r-act-drawer-row"
                  aria-current={isCurrent}
                  onClick={() => onModeChange('collapsed')}
                >
                  <span className="r-act-ig r-act-rt-leg-ig" aria-hidden="true" />
                  <span className="r-act-main">
                    <span className="ttl">{tripTitle(ev)}</span>
                    <span className="meta">{formatAppTime(ev.start)} · {duration(ev.minutes)}</span>
                  </span>
                  <span className="val num">
                    {n1(ev.miles)} mi
                    <span className="muted">{eff ? `${eff} ${efficiency.label}` : 'Short trip'}</span>
                  </span>
                </ActivityItemLink>
              );
            }
            if (ev.kind === 'charge') {
              return (
                <ActivityItemLink
                  key={`charge-${ev.id}`}
                  item={ev}
                  search={search}
                  className="r-act-drawer-row"
                  aria-current={isCurrent}
                  onClick={() => onModeChange('collapsed')}
                >
                  <span className="r-act-ig bolt r-act-rt-leg-ig" style={{ color: CATEGORY_COLOR[ev.category] }}>
                    <Zap aria-hidden="true" fill="currentColor" />
                  </span>
                  <span className="r-act-main">
                    <span className="ttl">{chargeTitle(ev)}</span>
                    <span className="meta">{CATEGORY_LABEL[ev.category]} · {formatAppTime(ev.start)}</span>
                  </span>
                  <span className="val num">
                    +{n1(ev.kwh)} kWh
                    <span className="muted">{ev.cost == null ? 'Add cost' : ev.cost === 0 ? 'Free' : usd(ev.cost)}</span>
                  </span>
                </ActivityItemLink>
              );
            }
            return null;
          })}
        </div>
      ) : null}
    </section>
  );
}

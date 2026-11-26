import React from 'react';
import { ChevronRight, Pencil, Split, TriangleAlert, Zap } from 'lucide-react';
import { useAuth, useBasemapConfig } from '@panda/hooks';
import { RouteReplayMap } from '@panda/ui/charts';
import { useDocumentTheme } from '@panda/ui/hooks';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import { useEfficiencyUnit } from '../../r-experience/efficiencyUnit';
import { CATEGORY_COLOR } from '../../charging/detail/ChargeDetailPane';
import {
  CATEGORY_LABEL, chargeTitle, duration, n1, shiftDay, tripTitle, usd, SHORT_TRIP_MI,
} from '../activity/activityModel';
import { ActivityItemLink, type ActivitySearch } from '../activity/activitySearch';
import type { ActivityRoadTrip } from '../activity/roadTripModel';

export interface RoadTripDetailPaneProps {
  roadTrip: ActivityRoadTrip;
  today: string;
  search: ActivitySearch;
  header: React.ReactNode;
  onRename: (id: string, name: string) => void;
  onSplit: (id: string) => void;
}

function dayLabel(day: string, today: string) {
  if (day === today) return 'Today';
  if (day === shiftDay(today, -1)) return 'Yesterday';
  return formatAppDate(new Date(`${day}T12:00:00Z`), { weekday: 'short', month: 'short', day: 'numeric', year: undefined });
}

/** Road trip detail: combined route map, 4 totals, Rename/Split actions, and day-by-day leg links. */
export function RoadTripDetailPane({ roadTrip, today, search, header, onRename, onSplit }: RoadTripDetailPaneProps) {
  const accessToken = useAuth((auth) => auth.accessToken);
  const basemap = useBasemapConfig();
  const isDark = useDocumentTheme();
  const efficiency = useEfficiencyUnit();
  const [renaming, setRenaming] = React.useState(false);
  const [draftName, setDraftName] = React.useState(roadTrip.name);

  React.useEffect(() => {
    setDraftName(roadTrip.name);
    setRenaming(false);
  }, [roadTrip.id, roadTrip.name]);

  const firstUnpriced = roadTrip.charges.find((charge) => charge.cost == null && !charge.active) ?? null;
  const startLabel = formatAppDate(roadTrip.start, { weekday: 'short', month: 'short', day: 'numeric', year: undefined });
  const endLabel = formatAppDate(roadTrip.end, { weekday: 'short', month: 'short', day: 'numeric', year: undefined });
  const dateRange = startLabel === endLabel ? startLabel : `${startLabel} – ${endLabel}`;
  const measuredEff = roadTrip.whPerMile != null ? efficiency.format(roadTrip.whPerMile) : null;

  const submitRename = (event: React.FormEvent) => {
    event.preventDefault();
    onRename(roadTrip.id, draftName);
    setRenaming(false);
  };

  return (
    <div className="r-act-detail" data-road-trip-detail={roadTrip.id}>
      {header}
      <div className="r-act-dbody">
        {roadTrip.route.length > 1 ? (
          <RouteReplayMap
            route={roadTrip.route}
            mode={isDark ? 'dark' : 'light'}
            basemapConfig={basemap.data}
            accessToken={accessToken}
            height="auto"
            className="r-act-map"
            label={`Map for ${roadTrip.name}`}
          />
        ) : (
          <div className="r-act-map r-act-map-empty"><p>No combined route was recorded for this road trip.</p></div>
        )}

        <div className="r-act-rt-head">
          <div>
            <div className="r-act-rt-kicker">
              <span className="r-act-rt-pill">Road trip</span>
              <span className="muted num">{roadTrip.trips.length} drives · {roadTrip.charges.length} charges</span>
            </div>
            <h1 className="r-act-dtitle">{roadTrip.name}</h1>
            <p className="muted">{dateRange} · {roadTrip.inferenceReason}</p>
          </div>
          <div className="r-act-rt-actions">
            <button
              type="button"
              className="r-act-rt-btn"
              onClick={() => setRenaming((open) => !open)}
              data-road-trip-rename
            >
              <Pencil aria-hidden="true" />
              Rename
            </button>
            <button
              type="button"
              className="r-act-rt-btn"
              onClick={() => onSplit(roadTrip.id)}
              data-road-trip-split
            >
              <Split aria-hidden="true" />
              Split
            </button>
          </div>
        </div>

        {renaming ? (
          <form className="r-act-cost-ed r-act-rt-rename-form" onSubmit={submitRename}>
            <div className="r-act-cost-row">
              <input
                type="text"
                value={draftName}
                onChange={(event) => setDraftName(event.target.value)}
                aria-label="Road trip name"
                placeholder={roadTrip.defaultName}
                maxLength={80}
              />
              <button type="submit" className="r-act-btn">Save</button>
              <button
                type="button"
                className="r-act-rt-btn"
                onClick={() => { onRename(roadTrip.id, ''); setRenaming(false); }}
              >
                Reset
              </button>
            </div>
          </form>
        ) : null}

        <div className="r-act-hero" data-road-trip-hero>
          <div>
            <p className="r-act-label">Distance</p>
            <p className="v num">{n1(roadTrip.miles)} <small>mi</small></p>
          </div>
          <div>
            <p className="r-act-label">Charging cost</p>
            <p className="v num">
              {roadTrip.cost > 0 ? usd(roadTrip.cost) : roadTrip.unpricedCount > 0 ? 'Unpriced' : 'Free'}
            </p>
          </div>
          <div>
            <p className="r-act-label">Efficiency</p>
            <p className="v num">
              {measuredEff ? <>{measuredEff} <small>{efficiency.label}</small></> : '–'}
            </p>
          </div>
          <div>
            <p className="r-act-label">Energy added</p>
            <p className="v num">+{n1(roadTrip.kwh)} <small>kWh</small></p>
          </div>
        </div>

        {firstUnpriced ? (
          <ActivityItemLink item={firstUnpriced} search={search} className="r-act-rt-callout" data-road-trip-unpriced>
            <TriangleAlert aria-hidden="true" />
            <span>
              <b>{roadTrip.unpricedCount} {roadTrip.unpricedCount === 1 ? 'charge has' : 'charges have'} no cost yet.</b>
              {' '}Edits live on individual sessions — open {chargeTitle(firstUnpriced)} to enter cost.
            </span>
            <ChevronRight aria-hidden="true" />
          </ActivityItemLink>
        ) : null}

        {roadTrip.days.map((group) => (
          <section key={group.day} className="r-act-day">
            <div className="r-act-day-h">
              <h3>{dayLabel(group.day, today)}</h3>
              <span className="num">
                {[
                  group.miles ? `${n1(group.miles)} mi` : '',
                  group.kwh ? `+${n1(group.kwh)} kWh` : '',
                  group.cost > 0 ? usd(group.cost) : '',
                ].filter(Boolean).join(' · ')}
              </span>
            </div>
            <div className="r-act-list">
              {group.events.map((event) => {
                if (event.kind === 'trip') {
                  const eff = event.whPerMile != null && event.miles >= SHORT_TRIP_MI ? efficiency.format(event.whPerMile) : null;
                  return (
                    <ActivityItemLink key={`trip-${event.id}`} item={event} search={search} className="r-act-item">
                      <span className="r-act-ig" aria-hidden="true" />
                      <span className="r-act-main">
                        <span className="ttl">{tripTitle(event)}</span>
                        <span className="meta">{formatAppTime(event.start)} · {duration(event.minutes)}</span>
                      </span>
                      <span className="val num">
                        {n1(event.miles)} mi
                        <span className="muted">{eff ? `${eff} ${efficiency.label}` : 'Short trip'}</span>
                      </span>
                    </ActivityItemLink>
                  );
                }
                if (event.kind === 'charge') {
                  return (
                    <ActivityItemLink key={`charge-${event.id}`} item={event} search={search} className="r-act-item">
                      <span className="r-act-ig bolt" style={{ color: CATEGORY_COLOR[event.category] }}>
                        <Zap aria-hidden="true" fill="currentColor" />
                      </span>
                      <span className="r-act-main">
                        <span className="ttl">{chargeTitle(event)}</span>
                        <span className="meta">
                          {CATEGORY_LABEL[event.category]} · {formatAppTime(event.start)}
                          {event.socStart != null && event.socEnd != null ? ` · ${Math.round(event.socStart)}% → ${Math.round(event.socEnd)}%` : ''}
                        </span>
                      </span>
                      <span className="val num">
                        +{n1(event.kwh)} kWh
                        <span className="muted">
                          {event.active ? 'Charging' : event.cost == null ? <span className="r-act-pill">Add cost</span> : event.cost === 0 ? 'Free' : usd(event.cost)}
                        </span>
                      </span>
                    </ActivityItemLink>
                  );
                }
                return null;
              })}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}

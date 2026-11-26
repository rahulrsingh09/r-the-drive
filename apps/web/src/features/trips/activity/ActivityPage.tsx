import React from 'react';
import { useNavigate } from '@tanstack/react-router';
import { ChevronDown, X } from 'lucide-react';
import { useResolvedVehicleSelection } from '@panda/hooks';
import { Button, Skeleton } from '@panda/ui/primitives';
import { formatAppDate } from '@panda/ui/lib/dateTime';
import { AppLayout } from '../../../components/layout/AppLayout';
import { NoVehicleState } from '../../../components/layout/NoVehicleState';
import { RMissedDriveBanner } from '../../r-experience/RMissedDrives';
import { RPageActions, useHidePageActions } from '../../r-experience/RAppLayout';
import { useEfficiencyUnit } from '../../r-experience/efficiencyUnit';
import { ChargeDetailPane, CATEGORY_COLOR } from '../../charging/detail/ChargeDetailPane';
import { RoadTripDetailPane } from '../detail/RoadTripDetailPane';
import { TripDetailPane } from '../detail/TripDetailPane';
import {
  activityFeed, detailOrder, groupByDay, n0, n1, shiftDay, shortPlace,
  summarize, usd, type ActivityData, type ActivityEvent, type ActivityFilter, type ActivityItem, type ActivityPeriod, type ChargeSummary,
  type FeedOptions, type TripSummary,
} from './activityModel';
import type { ActivityRoadTrip } from './activityModel';
import { BarStrip } from './ActivityCharts';
import { ActivityListItem, type ActivityItemSelection } from './ActivityListItem';
import { searchFilter, searchPeriod, withFilter, withoutRoadTrip, withPeriod, type ActivitySearch } from './activitySearch';
import { DetailHeader, type DetailPosition } from './DetailHeader';
import { Segmented } from './Segmented';
import { useActivityData } from './useActivityData';
import './activity.css';

export type ActivitySelection = ActivityItemSelection;

const WIDE_QUERY = '(min-width: 960px)';

function useWide() {
  const [wide, setWide] = React.useState(() => typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia(WIDE_QUERY).matches);
  React.useEffect(() => {
    if (typeof window.matchMedia !== 'function') return undefined;
    const media = window.matchMedia(WIDE_QUERY);
    const update = () => setWide(media.matches);
    update();
    media.addEventListener?.('change', update);
    return () => media.removeEventListener?.('change', update);
  }, []);
  return wide;
}

/*
 * The app shell remounts the page on every URL change. These values keep the
 * list where the owner left it when a selection changes the URL.
 */
const listMemory = { key: '', listTop: 0, windowTop: 0, animated: false };

/** Activity route content: vehicle gating, then the Activity page. */
export function ActivityContent({ selection, search }: { selection: ActivitySelection; search: ActivitySearch }) {
  const { authReady, effectiveVehicleId, vehicles, vehicleSelectionReady } = useResolvedVehicleSelection();
  const role = vehicles?.find((vehicle) => vehicle.id === effectiveVehicleId)?.membership_role ?? '';
  return (
    <AppLayout activeKey="activity">
      {!authReady || !vehicleSelectionReady ? (
        <div className="p-4 text-xs text-fg-tertiary" aria-busy="true">Loading...</div>
      ) : !effectiveVehicleId ? (
        <NoVehicleState title="No vehicle selected" description="Connect your Rivian account to see your trips and charges." />
      ) : (
        <ActivityPage vehicleId={effectiveVehicleId} selection={selection} search={search} canManage={role === 'owner' || role === 'manager'} />
      )}
    </AppLayout>
  );
}

export function ActivityPage({ vehicleId, selection, search, canManage }: {
  vehicleId: string; selection: ActivitySelection; search: ActivitySearch; canManage: boolean;
}) {
  const navigate = useNavigate();
  const wide = useWide();
  const filter = searchFilter(search);
  const period = searchPeriod(search);
  const place = search.place ?? null;
  const activity = useActivityData(vehicleId, period);
  const options: FeedOptions = { filter, period, place, today: activity.today };
  const items = React.useMemo(() => activityFeed(activity.data, options), [activity.data, filter, period, place, activity.today]); // eslint-disable-line react-hooks/exhaustive-deps
  const order = React.useMemo(() => detailOrder(items), [items]);
  const first = order[0] ?? null;
  const roadTripSelection: ActivitySelection = selection == null && search.rt ? { kind: 'road_trip', id: search.rt } : null;
  const explicitSelection = selection ?? roadTripSelection;
  // Desktop opens the newest item when nothing is selected.
  const selected: ActivitySelection = explicitSelection ?? (wide && first ? { kind: first.kind, id: first.id } : null);
  const memoryKey = JSON.stringify([filter, period, place]);

  const goBack = () => navigate({ to: '/trips', search: withoutRoadTrip(search) });
  const handleSplitRoadTrip = (id: string) => {
    activity.roadTripControls.split(id);
    if (search.rt === id) {
      navigate({ to: '/trips', search: withoutRoadTrip(search) });
    }
  };
  const selectedRoadTrip = selected?.kind === 'road_trip'
    ? (activity.data.roadTrips ?? []).find((rt) => rt.id === selected.id) ?? null
    : null;
  const header = selected ? <DetailHeader kind={selected.kind} position={positionOf(order, selected)} search={search} onBack={goBack} /> : null;
  const detail = selected ? (
    selected.kind === 'road_trip' ? (
      selectedRoadTrip ? (
        <RoadTripDetailPane
          key={selectedRoadTrip.id}
          roadTrip={selectedRoadTrip}
          today={activity.today}
          search={search}
          header={header}
          onRename={activity.roadTripControls.rename}
          onSplit={handleSplitRoadTrip}
        />
      ) : activity.loading ? (
        <div className="r-act-detail" data-road-trip-detail="loading">
          {header}
          <div className="r-act-dbody" aria-busy="true">
            <Skeleton className="h-72 w-full" />
            <Skeleton className="mt-4 h-10 w-2/3" />
          </div>
        </div>
      ) : (
        <div className="r-act-detail" data-road-trip-detail="missing">
          {header}
          <div className="r-act-card pad"><p>This road trip was split or is not in the selected period.</p></div>
        </div>
      )
    ) : selected.kind === 'trip' ? (
      <TripDetailPane key={selected.id} tripId={selected.id} vehicleId={vehicleId} routePreview={activity.routes.get(selected.id) ?? null} header={header} />
    ) : (
      <ChargeDetailPane key={selected.id} sessionId={selected.id} vehicleId={vehicleId} canManage={canManage} charges={activity.data.charges} header={header} />
    )
  ) : null;

  React.useEffect(() => { listMemory.animated = true; }, []);

  if (!wide && explicitSelection) {
    return <div className="r-act r-act-single" data-activity-detail-page><HidePageActions />{detail}</div>;
  }

  const list = (
    <ActivityList
      data={activity.data}
      items={items}
      options={options}
      search={search}
      selected={selected}
      loading={activity.loading}
      error={activity.error}
      onRetry={activity.retry}
      onSelect={() => { listMemory.windowTop = window.scrollY; }}
    />
  );
  const top = (
    <header className="r-act-top">
      <div>
        <h1>Activity</h1>
        <p className="r-act-sub">Trips, charges and parked time</p>
      </div>
      <RPageActions />
    </header>
  );

  if (wide) {
    return (
      <ActivitySplit memoryKey={memoryKey} top={top} banner={<RMissedDriveBanner vehicleId={vehicleId} />} list={list} detail={detail} />
    );
  }
  return (
    <div className="r-act r-act-single" data-activity-list-page>
      <PhoneScrollMemory memoryKey={memoryKey} />
      {top}
      <RMissedDriveBanner vehicleId={vehicleId} />
      {list}
    </div>
  );
}

/** The phone detail page has its own back row, so the gear and theme row does not show above it. */
function HidePageActions() {
  useHidePageActions();
  return null;
}

function positionOf(order: ActivityEvent[], selected: NonNullable<ActivitySelection>): DetailPosition | null {
  const index = order.findIndex((item) => item.kind === selected.kind && item.id === selected.id);
  if (index < 0) return null;
  return { index, total: order.length, newer: order[index - 1] ?? null, older: order[index + 1] ?? null };
}

function PhoneScrollMemory({ memoryKey }: { memoryKey: string }) {
  React.useLayoutEffect(() => {
    if (listMemory.key === memoryKey && listMemory.windowTop > 0) window.scrollTo(0, listMemory.windowTop);
    listMemory.key = memoryKey;
  }, [memoryKey]);
  return null;
}

/** Desktop: header fixed, list and detail columns each scroll on their own, the page does not scroll. */
function ActivitySplit({ memoryKey, top, banner, list, detail }: {
  memoryKey: string; top: React.ReactNode; banner: React.ReactNode; list: React.ReactNode; detail: React.ReactNode;
}) {
  const rootRef = React.useRef<HTMLDivElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);
  const [height, setHeight] = React.useState<number | null>(null);

  React.useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return undefined;
    const fit = () => {
      const topEdge = root.getBoundingClientRect().top + window.scrollY;
      let next = Math.max(420, window.innerHeight - topEdge - 16);
      // Shell padding under the page can still overflow the window; take it off so the page itself never scrolls.
      root.style.height = `${next}px`;
      const overflow = document.documentElement.scrollHeight - window.innerHeight;
      if (overflow > 0) next = Math.max(420, next - overflow);
      root.style.height = '';
      setHeight(next);
    };
    fit();
    document.documentElement.classList.add('r-act-locked');
    window.addEventListener('resize', fit);
    return () => {
      window.removeEventListener('resize', fit);
      document.documentElement.classList.remove('r-act-locked');
    };
  }, []);

  // Restore after the column has its height; before that it cannot scroll.
  const restoredKey = React.useRef<string | null>(null);
  React.useLayoutEffect(() => {
    const element = listRef.current;
    if (!element || height == null || restoredKey.current === memoryKey) return;
    if (listMemory.key === memoryKey) element.scrollTop = listMemory.listTop;
    else { listMemory.listTop = 0; element.scrollTop = 0; }
    listMemory.key = memoryKey;
    restoredKey.current = memoryKey;
  }, [height, memoryKey]);

  return (
    <div ref={rootRef} className="r-act r-act-split" style={height ? { height } : undefined} data-activity-split>
      {top}
      {banner}
      <div className="r-act-cols">
        <div
          ref={listRef}
          className="r-act-col"
          data-col="list"
          onScroll={(event) => { if (restoredKey.current === memoryKey) listMemory.listTop = event.currentTarget.scrollTop; }}
        >
          {list}
        </div>
        <div className="r-act-col r-act-pane" data-col="detail">{detail}</div>
      </div>
    </div>
  );
}

function ActivityList({ data, items, options, search, selected, loading, error, onRetry, onSelect }: {
  data: ActivityData; items: ActivityItem[]; options: FeedOptions; search: ActivitySearch; selected: ActivitySelection;
  loading: boolean; error: boolean; onRetry: () => void; onSelect: () => void;
}) {
  const navigate = useNavigate();
  const efficiency = useEfficiencyUnit();
  const groups = React.useMemo(() => groupByDay(items), [items]);
  const [expandedIds, setExpandedIds] = React.useState<Set<string>>(() => new Set());
  const chargeRoadTripMap = React.useMemo(() => {
    const map = new Map<string, ActivityRoadTrip>();
    for (const rt of data.roadTrips ?? []) {
      for (const charge of rt.charges) map.set(charge.id, rt);
    }
    return map;
  }, [data.roadTrips]);
  const toggleExpanded = (id: string) => setExpandedIds((prev) => {
    const next = new Set(prev);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  const animate = !listMemory.animated;
  let order = 0;
  const setFilter = (value: string) => navigate({ to: '/trips', search: withFilter(search, value as ActivityFilter) });
  const setPeriod = (value: string) => navigate({ to: '/trips', search: withPeriod(search, value as ActivityPeriod) });

  return (
    <>
      <div className="r-act-filters" role="group" aria-label="Filter activity">
        <Segmented
          label="Show"
          value={options.place ? null : options.filter}
          options={[{ value: 'all', label: 'All' }, { value: 'trips', label: 'Trips' }, { value: 'charges', label: 'Charges' }]}
          onChange={(value) => setFilter(value)}
        />
        <label className="r-act-period">
          <span className="sr-only">Period</span>
          <select value={options.period} onChange={(event) => setPeriod(event.target.value)} data-activity-period>
            <option value="7">7 days</option>
            <option value="30">30 days</option>
            <option value="all">All time</option>
          </select>
          <ChevronDown aria-hidden="true" />
        </label>
      </div>
      {options.place ? (
        <div className="r-act-pillrow">
          <button type="button" className="r-act-fchip" aria-label="Show all activity" onClick={() => navigate({ to: '/trips', search: withFilter(search, 'all') })}>
            {shortPlace(options.place)}<X aria-hidden="true" />
          </button>
          <span className="muted">Charges at this place</span>
        </div>
      ) : null}
      {loading ? (
        <div aria-busy="true"><Skeleton className="h-48 w-full rounded-2xl" /><Skeleton className="mt-4 h-64 w-full rounded-2xl" /></div>
      ) : error ? (
        <div className="r-act-card r-act-empty">
          <p>Activity did not load.</p>
          <Button type="button" variant="secondary" className="mt-3 min-h-11" onClick={onRetry}>Try again</Button>
        </div>
      ) : (
        <>
          <SummaryCard summary={summarize(data, options)} efficiency={efficiency} />
          {groups.length ? groups.map((group) => (
            <section key={group.day} className="r-act-day" aria-label={dayLabel(group.day, options.today)}>
              <div className="r-act-day-h">
                <h3>{dayLabel(group.day, options.today)}</h3>
                <span className="num">{[group.miles ? `${n1(group.miles)} mi` : '', group.kwh ? `+${n0(group.kwh)} kWh` : ''].filter(Boolean).join(' · ')}</span>
              </div>
              <div className="r-act-list">
                {group.items.map((item) => (
                  <ActivityListItem
                    key={`${item.kind}-${item.id}`}
                    efficiency={efficiency}
                    item={item}
                    index={Math.min(order++, 14)}
                    animate={animate}
                    search={search}
                    selected={selected}
                    expanded={item.kind === 'road_trip' ? expandedIds.has(item.id) : false}
                    onToggleExpand={toggleExpanded}
                    chargeRoadTrip={item.kind === 'charge' && options.filter === 'charges' ? chargeRoadTripMap.get(item.id) ?? null : null}
                    onSelect={onSelect}
                  />
                ))}
              </div>
            </section>
          )) : <div className="r-act-card r-act-empty">Nothing in this period.</div>}
          <div style={{ height: 24 }} />
        </>
      )}
    </>
  );
}

function dayLabel(day: string, today: string) {
  if (day === today) return 'Today';
  if (day === shiftDay(today, -1)) return 'Yesterday';
  return formatAppDate(new Date(`${day}T12:00:00Z`), { weekday: 'short', month: 'short', day: 'numeric', year: undefined });
}

function SummaryCard({ summary, efficiency }: { summary: TripSummary | ChargeSummary; efficiency: ReturnType<typeof useEfficiencyUnit> }) {
  const dayTip = (day: string) => formatAppDate(new Date(`${day}T12:00:00Z`), { weekday: 'short', month: 'short', day: 'numeric', year: undefined });
  if (summary.kind === 'charges') {
    return (
      <section className="r-act-card r-act-sum" data-activity-summary="charges">
        <div className="r-act-sum-head">
          <div><p className="r-act-label">Charged</p><p className="r-act-big num">{n0(summary.kwh)}<small>kWh</small></p></div>
          <p className="r-act-label r-act-right">{summary.count} sessions</p>
        </div>
        <div className="r-act-sum-strip">
          <BarStrip
            label="Energy charged per day"
            bars={summary.perDay.map((day) => ({
              segments: [{ value: day.home, color: CATEGORY_COLOR.home }, { value: day.office, color: CATEGORY_COLOR.office }, { value: day.public, color: CATEGORY_COLOR.public }],
              tip: `${dayTip(day.day)}: ${n1(day.home + day.office + day.public)} kWh`,
            }))}
          />
        </div>
        <div className="r-act-sum-grid">
          <div><p className="r-act-label">Cost</p><p className="v num">{usd(summary.cost)}</p></div>
          <div><p className="r-act-label">At home</p><p className="v num">{n0(summary.homePercent)}<small>%</small></p></div>
          <div><p className="r-act-label">No cost yet</p><p className="v num">{summary.unpriced}</p></div>
        </div>
      </section>
    );
  }
  return (
    <section className="r-act-card r-act-sum" data-activity-summary="trips">
      <div className="r-act-sum-head">
        <div><p className="r-act-label">Trip</p><p className="r-act-big num">{n0(summary.miles)}<small>mi</small></p></div>
        <p className="r-act-label r-act-right">{summary.count} trips<br />{summary.days.length} days</p>
      </div>
      <div className="r-act-sum-strip">
        <BarStrip
          label="Trip miles per day"
          bars={summary.perDay.map((day) => ({ segments: [{ value: day.miles, color: CATEGORY_COLOR.home }], tip: `${dayTip(day.day)}: ${n1(day.miles)} mi` }))}
        />
      </div>
      <div className="r-act-sum-grid">
        <div><p className="r-act-label">Efficiency</p><p className="v num">{efficiency.format(summary.whPerMile) ?? '–'} <small>{efficiency.label}</small></p></div>
        <div><p className="r-act-label">Avg trip</p><p className="v num">{summary.averageMiles != null ? n1(summary.averageMiles) : '–'} <small>mi</small></p></div>
        <div><p className="r-act-label">Drive time</p><p className="v num">{n0(summary.driveHours)} <small>h</small></p></div>
      </div>
    </section>
  );
}

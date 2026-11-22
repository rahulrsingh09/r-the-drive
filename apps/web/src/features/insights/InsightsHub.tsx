import React from 'react';
import { Link } from '@tanstack/react-router';
import { useCurrentVehicleStatus, useDegradation, useMe, useResolvedVehicleSelection, useSavedPlaces } from '@panda/hooks';
import { getAppTimezone } from '@panda/ui/lib/dateTime';
import { NoVehicleState } from '../../components/layout/NoVehicleState';
import { RAppLayout, RPageActions } from '../r-experience/RAppLayout';
import { TipLayer } from './chartKit';
import { useSegInk } from './controls';
import type { InsightsData } from './InsightsContext';
import { usableCapacityNow } from '../../lib/batteryHealth';
import { capacityByDay, chargeRows, dayKeyOf, firstDay, monthDay, tripRows } from './insightsModel';
import { useAllCharges, useAllTrips } from './useInsightsData';
import { BatteryTab } from './tabs/BatteryTab';
import { ChargingTab } from './tabs/ChargingTab';
import { CostTab } from './tabs/CostTab';
import { EfficiencyTab } from './tabs/EfficiencyTab';
import { LifetimeTab } from './tabs/LifetimeTab';
import './insights.css';

export const INSIGHT_TABS = [
  ['cost', 'Cost'], ['charging', 'Charging'], ['battery', 'Battery'], ['efficiency', 'Efficiency'], ['lifetime', 'Lifetime'],
] as const;
export type InsightTab = typeof INSIGHT_TABS[number][0];
export const isInsightTab = (value: unknown): value is InsightTab => INSIGHT_TABS.some(([key]) => key === value);

function InsightTabs({ tab }: { tab: InsightTab }) {
  const { ref, inkEl } = useSegInk([tab]);
  return (
    <div className="ri-filters">
      <nav ref={ref} className="ri-seg ri-seg-five" aria-label="Insights sections">
        {inkEl}
        {INSIGHT_TABS.map(([key, label]) => (
          <Link key={key} to="/insights/$tab" params={{ tab: key }} aria-current={key === tab ? 'page' : undefined}>{label}</Link>
        ))}
      </nav>
    </div>
  );
}

function Skeleton() {
  return (
    <div className="ri-ins-grid" aria-busy="true">
      {[0, 1, 2].map(i => <div key={i} className={`ri-card ri-pad ri-skel${i === 0 ? ' ri-wide' : ''}`} />)}
      <p className="ri-sr" role="status">Loading insights…</p>
    </div>
  );
}

function InsightsBody({ tab, vehicleId, onFirstDay }: { tab: InsightTab; vehicleId: string; onFirstDay: (day: string | null) => void }) {
  const tripsQuery = useAllTrips(vehicleId);
  const chargesQuery = useAllCharges(vehicleId);
  const placesQuery = useSavedPlaces();
  const status = useCurrentVehicleStatus(vehicleId);
  const me = useMe();
  const degradation = useDegradation(vehicleId);
  const timezone = getAppTimezone();
  const [now] = React.useState(() => Date.now());

  const core = React.useMemo<Omit<InsightsData, 'status'> | null>(() => {
    if (!tripsQuery.data || !chargesQuery.data) return null;
    const places = placesQuery.data ?? [];
    const today = dayKeyOf(now, timezone);
    const trips = tripRows(tripsQuery.data, timezone, places);
    const charges = chargeRows(chargesQuery.data, places, timezone);
    return {
      vehicleId, userId: me.data?.user_id ?? null, timezone, today, first: firstDay(trips, charges, today), trips, charges, places,
    };
  }, [tripsQuery.data, chargesQuery.data, placesQuery.data, me.data?.user_id, now, timezone, vehicleId]);
  const liveStatus = status.data?.vehicle_id === vehicleId ? status.data : null;
  const data: InsightsData | null = core ? { ...core, status: liveStatus } : null;
  const capacity = React.useMemo(() => capacityByDay(degradation.data ?? [], timezone), [degradation.data, timezone]);

  const failed = tripsQuery.isError || chargesQuery.isError;
  const first = core && (core.trips.length || core.charges.length) ? core.first : null;
  React.useEffect(() => { onFirstDay(first); }, [first, onFirstDay]);
  return (
    <>
      {failed ? <p className="ri-card ri-pad ri-empty" role="alert">Insights did not load. Check the connection and try again.</p>
        : !data ? <Skeleton />
          : !data.trips.length && !data.charges.length ? (
            <div className="ri-card ri-pad ri-empty-card"><b>No trips or charges yet</b>
              <p className="ri-muted">Insights fill in after the first drive or charge.</p></div>
          ) : (
            <TipLayer className="ri-ins-grid">
              {tab === 'cost' && <CostTab data={data} />}
              {tab === 'charging' && <ChargingTab data={data} capacityKwh={usableCapacityNow(capacity)} />}
              {tab === 'battery' && <BatteryTab data={data} capacity={capacity} />}
              {tab === 'efficiency' && <EfficiencyTab data={data} />}
              {tab === 'lifetime' && <LifetimeTab data={data} />}
            </TipLayer>
          )}
    </>
  );
}

/** Insights: five tabs, Cost · Charging · Battery · Efficiency · Lifetime, over all data since the first day. */
export function InsightsHub({ tab = 'cost' }: { tab?: InsightTab }) {
  const { authReady, effectiveVehicleId, vehicleSelectionReady } = useResolvedVehicleSelection();
  const [first, setFirst] = React.useState<string | null>(null);
  return (
    <RAppLayout activeKey="insights">
      <div className="ri">
        <div className="ri-top">
          <div className="ri-top-text">
            <h1>Insights</h1>
            <p className="ri-sub">{first ? `Since ${monthDay(first)}, the first day of data` : '\u00a0'}</p>
          </div>
          <RPageActions />
        </div>
        <InsightTabs tab={tab} />
        {!authReady || !vehicleSelectionReady ? <p className="ri-muted" role="status" aria-busy="true">Loading your vehicle…</p>
          : !effectiveVehicleId ? <NoVehicleState />
            : <InsightsBody key={effectiveVehicleId} tab={tab} vehicleId={effectiveVehicleId} onFirstDay={setFirst} />}
        <div className="ri-end" />
      </div>
    </RAppLayout>
  );
}

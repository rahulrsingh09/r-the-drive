import React from 'react';
import type { Vehicle } from '@panda/types';
import {
  useChargeSessions, useCurrentVehicleStatus, usePhantomDrainPeriods, useSavedPlaces, useTrips, useTripTrack,
} from '@panda/hooks';
import { vehicleReading } from '../r-experience/readings';
import { EmptyCard, NowSetupChecklist, NowSkeleton } from './NowEmpty';
import { NowLiveCharging, NowLiveDriving } from './NowLive';
import { BatteryCard, NowHeader, NowLatest, NowMonth, NowStage, NowTiles, NowWeek, type ChipState } from './NowParts';
import {
  monthSummary, nowActivity, nowPlace, nowWindows, parkedChip, updatedLabel, vehicleName, weekSummary,
} from './nowModel';
import { useClock } from './useNowLive';
import { useMotionAllowed } from './useNowMotion';
import './now.css';

const VISIBILITY_REFETCH_MS = 15_000;
const PULL_REFRESH_PX = 64;

/** Fixed windows for one page visit, so query keys stay stable while the page is open. */
function useVisitWindows() {
  const [now] = React.useState(() => Date.now());
  return React.useMemo(() => ({ now, ...nowWindows(now) }), [now]);
}

/**
 * The Now page for one vehicle. Parked: car, battery, tiles, Latest, This week and Last 30 days.
 * Charging or driving (from the live vehicle status): the live view. A car with no trips and no
 * charges yet: Day one. Before the first reads arrive: the skeleton.
 */
export function NowView({ vehicle, vehicleControl }: { vehicle: Vehicle; vehicleControl?: React.ReactNode }) {
  const animate = useMotionAllowed();
  const { now, from, to } = useVisitWindows();
  const clockNow = useClock(true, 30_000);
  const snapshot = useCurrentVehicleStatus(vehicle.id);
  const status = snapshot.data?.vehicle_id === vehicle.id ? snapshot.data : null;
  const places = useSavedPlaces();
  const latestTrips = useTrips(vehicle.id, null, null, 1, 1);
  const latestCharges = useChargeSessions(vehicle.id, null, null, 1, 1);
  const trips = useTrips(vehicle.id, from, to, 1, 200);
  const charges = useChargeSessions(vehicle.id, from, to, 1, 200);
  const parks = usePhantomDrainPeriods(vehicle.id, from, to);
  const latestTrip = latestTrips.data?.items?.[0];
  const latestCharge = latestCharges.isPlaceholderData ? undefined : latestCharges.data?.items?.[0];
  const track = useTripTrack(latestTrip?.id ?? null, vehicle.id);

  const [refreshing, setRefreshing] = React.useState(false);
  const [pullReady, setPullReady] = React.useState(false);
  const hiddenAtRef = React.useRef<number | null>(null);
  const pullStartYRef = React.useRef<number | null>(null);

  const refreshAll = React.useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.allSettled([
        snapshot.refetch(),
        latestTrips.refetch(),
        latestCharges.refetch(),
        trips.refetch(),
        charges.refetch(),
        parks.refetch(),
        track.refetch(),
      ]);
    } finally {
      setRefreshing(false);
    }
  }, [charges, latestCharges, latestTrips, parks, snapshot, track, trips]);

  React.useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') {
        hiddenAtRef.current = Date.now();
        return;
      }
      if (document.visibilityState === 'visible' && hiddenAtRef.current != null) {
        const awayMs = Date.now() - hiddenAtRef.current;
        hiddenAtRef.current = null;
        if (awayMs >= VISIBILITY_REFETCH_MS) void refreshAll();
      }
    };
    document.addEventListener('visibilitychange', onVisibility);
    return () => document.removeEventListener('visibilitychange', onVisibility);
  }, [refreshAll]);

  const onTouchStart = (event: React.TouchEvent) => {
    if (typeof window !== 'undefined' && window.scrollY <= 0 && event.touches.length === 1) {
      pullStartYRef.current = event.touches[0]!.clientY;
    } else {
      pullStartYRef.current = null;
    }
  };
  const onTouchMove = (event: React.TouchEvent) => {
    if (pullStartYRef.current == null || event.touches.length !== 1) return;
    setPullReady(event.touches[0]!.clientY - pullStartYRef.current >= PULL_REFRESH_PX);
  };
  const onTouchEnd = () => {
    const triggered = pullReady;
    pullStartYRef.current = null;
    setPullReady(false);
    if (triggered && !refreshing) void refreshAll();
  };

  const name = vehicleName(vehicle);
  const activity = nowActivity(status);
  const place = nowPlace(status, places.data, latestTrip);

  if ((snapshot.isLoading && !snapshot.data) || latestTrips.isLoading || latestCharges.isLoading) return <NowSkeleton name={name} />;

  if (activity === 'charging') {
    const open = latestCharge && latestCharge.ended_at == null ? latestCharge : undefined;
    return <NowLiveCharging vehicle={vehicle} status={status} session={open} place={place} places={places.data} control={vehicleControl} />;
  }
  if (activity === 'driving') return <NowLiveDriving vehicle={vehicle} status={status} lastTrip={latestTrip} control={vehicleControl} />;

  const stale = Boolean(status?.telemetry_stale);
  const chipState: ChipState = !status ? 'waiting' : stale ? 'stale' : 'live';
  const chip = stale ? parkedChip(status, place).replace(/^Parked/, 'Not updating') : parkedChip(status, place);
  const dayOne = !latestTrip && !latestCharge && !latestTrips.isError && !latestCharges.isError;
  const subLabel = refreshing ? 'Refreshing…' : updatedLabel(status, clockNow) ?? (dayOne ? 'Connected' : undefined);
  const header = <NowHeader name={name} chip={chip} chipState={chipState} control={vehicleControl}
    sub={subLabel} onRefresh={() => void refreshAll()} refreshing={refreshing} />;
  const battery = <BatteryCard battery={vehicleReading(status, 'battery_level')} limit={vehicleReading(status, 'battery_limit')}
    rangeMi={vehicleReading(status, 'range_miles')} animate={animate} />;

  if (dayOne) {
    return <div className="now" data-activity="parked" data-state="day-one" onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd}>
      {pullReady && <div className="now-pull-hint" role="status">Release to refresh</div>}
      {header}
      <div className="now-grid">
        <div className="now-main"><NowStage vehicle={vehicle} />{battery}<NowTiles status={status} /></div>
        <div className="now-side">
          <h2 className="now-h2">Latest</h2>
          <EmptyCard title="Your first drive shows up here" text="R records every drive and charge from now on. A trip appears a few minutes after you park." />
          <NowSetupChecklist vehicle={vehicle} status={status} name={name} />
        </div>
      </div>
    </div>;
  }

  const tripItems = trips.data?.items ?? [];
  const chargeItems = charges.data?.items ?? [];
  const week = trips.data && charges.data ? weekSummary(tripItems, chargeItems, now) : null;
  const month = trips.data && charges.data ? monthSummary(tripItems, chargeItems, parks.data?.periods ?? [], places.data, now) : null;
  const retry = () => { void trips.refetch(); void charges.refetch(); void parks.refetch(); };
  const failed = trips.isError || charges.isError;
  return <div className="now" data-activity="parked" onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd}>
    {pullReady && <div className="now-pull-hint" role="status">Release to refresh</div>}
    {header}
    {snapshot.isError && <p className="now-alert" role="alert">The car readings could not be refreshed.{' '}
      <button type="button" className="now-retry" onClick={() => void snapshot.refetch()}>Retry</button></p>}
    <div className="now-grid">
      <div className="now-main">
        <NowStage vehicle={vehicle} />
        {battery}
        <NowTiles status={status} />
      </div>
      <div className="now-side">
        <h2 className="now-h2">Latest</h2>
        <NowLatest trip={latestTrip} track={track.isPlaceholderData ? undefined : track.data} charge={latestCharge} places={places.data} />
        <h2 className="now-h2">This week</h2>
        {week ? <NowWeek week={week} animate={animate} />
          : failed ? <p className="now-alert" role="alert">This week could not be loaded.{' '}
            <button type="button" className="now-retry" onClick={retry}>Retry</button></p>
          : <div className="now-skel" style={{ height: 220, borderRadius: 18 }} aria-busy="true" aria-label="Loading this week" />}
      </div>
    </div>
    {month && <NowMonth month={month} />}
  </div>;
}

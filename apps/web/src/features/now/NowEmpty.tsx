import React from 'react';
import { Link } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Check, ChevronRight, Minus } from 'lucide-react';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { api, useAuthReady } from '@panda/hooks';
import { RPageActions } from '../r-experience/RAppLayout';

const ROAD = <svg viewBox="0 0 120 64" aria-hidden="true">
  <path d="M8 52 C 40 52, 40 14, 72 14 S 104 30, 112 12" className="now-ec-road" />
  <circle cx="8" cy="52" r="6" className="now-ec-start" /><circle cx="112" cy="12" r="7" className="now-ec-dot" />
</svg>;

/** A card for a section that has nothing yet. */
export function EmptyCard({ title, text, icon = ROAD }: { title: string; text: string; icon?: React.ReactNode }) {
  return <section className="now-card now-empty-card now-rise">
    <div className="now-ec-ic">{icon}</div><b>{title}</b><p>{text}</p>
  </section>;
}

/** Day one: the car is connected, but no trip or charge is recorded yet. */
export function NowSetupChecklist({ vehicle, status, name }: { vehicle: Vehicle; status: VehicleStatus | null; name: string }) {
  const authReady = useAuthReady();
  const backfill = useQuery({
    queryKey: ['now', 'backfill-status', vehicle.id],
    queryFn: () => api.getBackfillStatus(vehicle.id),
    enabled: authReady,
    refetchInterval: query => (query.state.data?.history_backfilled_at ? false : 20_000),
    retry: 1,
  });
  const connected = status != null && (!status.auth_state || status.auth_state === 'authorized');
  const loaded = Boolean(backfill.data?.history_backfilled_at);
  return <>
    <h2 className="now-h2">Getting set up</h2>
    <section className="now-card now-checklist now-rise" aria-label="Getting set up">
      <div className="now-ck" data-done={connected || undefined}>
        {connected ? <Check aria-hidden="true" /> : <Minus aria-hidden="true" />}
        <span>{connected ? 'Rivian account connected' : 'Waiting for the first reading from Rivian'}</span>
      </div>
      <div className="now-ck" data-done><Check aria-hidden="true" /><span>Car found: {name}</span></div>
      <div className="now-ck" data-done={loaded || undefined}>
        {loaded ? <Check aria-hidden="true" /> : <Minus aria-hidden="true" />}
        <span>{loaded ? 'Rivian history loaded' : 'Loading Rivian history · about a minute'}</span>
        {!loaded && <span className="now-spinner" aria-hidden="true" />}
      </div>
      <Link className="now-ck now-ck-link" to="/settings"><ChevronRight aria-hidden="true" /><span>Check your Home and Office rates</span></Link>
    </section>
  </>;
}

/** Loading: the same layout as the page it replaces. */
export function NowSkeleton({ name }: { name: string }) {
  const block = (height: number, radius = 18, width = '100%') =>
    <div className="now-skel" style={{ height, width, borderRadius: radius }} />;
  return <div className="now" aria-busy="true" aria-label="Loading">
    <header className="now-top"><div className="now-top-text"><h1>{name}</h1>
      <div className="now-sub"><span className="now-skel" style={{ display: 'inline-block', height: 14, width: 140, borderRadius: 7 }} /></div></div><RPageActions /></header>
    <div className="now-grid">
      <div className="now-main">{block(220, 28)}{block(130)}<div className="now-tiles">{block(76, 16)}{block(76, 16)}{block(76, 16)}</div></div>
      <div className="now-side"><div className="now-skel-gap" />{block(78)}{block(78)}<div className="now-skel-gap" />{block(220)}</div>
    </div>
    <span className="sr-only" role="status">Loading your car</span>
  </div>;
}

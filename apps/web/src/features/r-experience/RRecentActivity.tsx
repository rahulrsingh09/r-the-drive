import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight, ChevronRight, Route, Zap } from 'lucide-react';
import { useChargeSessions, useTrips } from '@panda/hooks';
import { activityPresentation } from './activityPresentation';
import type { WidgetCtx } from '@panda/dashboards';
import './r-overview.css';

export function RRecentActivity({ ctx }: { ctx: WidgetCtx }) {
  const { from, to } = ctx;
  const trips = useTrips(ctx.vehicleId ?? null, from ?? null, to ?? null, 1, 2);
  const charges = useChargeSessions(ctx.vehicleId ?? null, from ?? null, to ?? null, 1, 1);
  const view = activityPresentation(trips, charges);
  const { charge } = view.charges;
  const retryTrips = () => { void trips.refetch(); };
  const retryCharges = () => { void charges.refetch(); };
  return (
    <aside className="r-home-activity" aria-label="Recent activity">
      <section className="r-recent">
        <div className="r-section-heading"><h2>Recent trips</h2><Link to="/trips">All trips <ArrowUpRight /></Link></div>
        {view.trips.phase === 'loading' ? <p className="r-empty" role="status">Loading trips…</p>
          : view.trips.phase === 'error' ? <p className="r-empty" role="alert">Trips could not be loaded. <button type="button" onClick={retryTrips}>Retry</button></p>
          : !view.trips.rows.length ? <p className="r-empty">No trips in this period.</p>
          : <div className="r-trip-list">{view.trips.rows.map(trip => <Link key={trip.id} to="/trips/$tripId" params={{ tripId: trip.id }} className="r-trip-row">
            <span className="r-route-tile"><Route aria-hidden="true" /></span>
            <span className="r-trip-description"><strong>{trip.start} → {trip.end}</strong>
              <small>{trip.time} · {trip.duration}</small></span>
            <span className="r-trip-value">{trip.distance}<small>{trip.efficiency}</small></span><ChevronRight aria-hidden="true" />
          </Link>)}</div>}
      </section>
      <section className="r-last-charge">
        <div className="r-section-heading"><h2>Last charge in this period</h2><Link to="/charging">Charging <ArrowUpRight /></Link></div>
        {view.charges.phase === 'error' ? <p className="r-empty" role="alert">Charging history could not be loaded. <button type="button" onClick={retryCharges}>Retry</button></p>
          : view.charges.phase === 'loading' ? <p className="r-empty" role="status">Loading charging history…</p>
          : !charge ? <p className="r-empty">No charging sessions in this period.</p>
          : <Link className="r-charge-summary" to="/charging/$sessionId" params={{ sessionId: charge.id }}>
            <Zap /><span><strong>{charge.name}</strong><small>{charge.time}</small></span>
            <span>{charge.energy}<small>{charge.cost}</small></span><ChevronRight />
          </Link>}
      </section>
    </aside>
  );
}

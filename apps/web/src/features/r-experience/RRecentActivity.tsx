import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowUpRight, ChevronRight, Route, Zap } from 'lucide-react';
import { useChargeSessions, useTrips } from '@panda/hooks';
import { formatCurrency, formatDuration, formatEfficiency, formatKwh, formatMiles } from '@panda/ui/lib/utils';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import type { WidgetCtx } from '@panda/dashboards';
import './r-overview.css';

export function RRecentActivity({ ctx }: { ctx: WidgetCtx }) {
  const { from, to } = ctx;
  const trips = useTrips(ctx.vehicleId ?? null, from ?? null, to ?? null, 1, 2);
  const charges = useChargeSessions(ctx.vehicleId ?? null, from ?? null, to ?? null, 1, 1);
  const charge = charges.isPlaceholderData ? undefined : charges.data?.items?.[0];
  return (
    <aside className="r-home-activity" aria-label="Recent activity">
      <section className="r-recent">
        <div className="r-section-heading"><h2>Recent trips</h2><Link to="/trips">All trips <ArrowUpRight /></Link></div>
        {trips.isLoading || trips.isPlaceholderData ? <p className="r-empty" role="status">Loading trips…</p>
          : trips.isError ? <p className="r-empty" role="alert">Trips could not be loaded. <button type="button" onClick={() => void trips.refetch()}>Retry</button></p>
          : !trips.data?.items?.length ? <p className="r-empty">No trips in this period.</p>
          : <div className="r-trip-list">{trips.data.items.map(trip => <Link key={trip.id} to="/trips/$tripId" params={{ tripId: trip.id }} className="r-trip-row">
            <span className="r-route-tile"><Route aria-hidden="true" /></span>
            <span className="r-trip-description"><strong>{trip.start_place || trip.start_address || 'Start'} → {trip.end_place || trip.end_address || 'Destination'}</strong>
              <small>{formatAppDateTime(trip.started_at)} · {formatDuration(trip.duration_min)}</small></span>
            <span className="r-trip-value">{formatMiles(trip.distance_mi)}<small>{formatEfficiency(trip.efficiency_wh_mi)}</small></span><ChevronRight aria-hidden="true" />
          </Link>)}</div>}
      </section>
      <section className="r-last-charge">
        <div className="r-section-heading"><h2>Last charge in this period</h2><Link to="/charging">Charging <ArrowUpRight /></Link></div>
        {charges.isError ? <p className="r-empty" role="alert">Charging history could not be loaded. <button type="button" onClick={() => void charges.refetch()}>Retry</button></p>
          : charges.isLoading || charges.isPlaceholderData ? <p className="r-empty" role="status">Loading charging history…</p>
          : !charge ? <p className="r-empty">No charging sessions in this period.</p>
          : <Link className="r-charge-summary" to="/charging/$sessionId" params={{ sessionId: charge.id }}>
            <Zap /><span><strong>{charge.location_name || 'Charging session'}</strong><small>{formatAppDateTime(charge.started_at)}</small></span>
            <span>{formatKwh(charge.energy_added_kwh)}<small>{formatCurrency(charge.cost_usd)}</small></span><ChevronRight />
          </Link>}
      </section>
    </aside>
  );
}

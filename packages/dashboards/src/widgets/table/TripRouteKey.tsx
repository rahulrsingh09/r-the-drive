import { tripRouteColor } from '@panda/ui/charts';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import type { TripRow } from '@panda/ui/tables';

export function TripRouteKey({ routes, trips, highlightedId, onHighlight }: {
  routes: Array<{ id: string; index: number }>;
  trips: Record<string, TripRow>;
  highlightedId: string | null;
  onHighlight: (id: string | null) => void;
}) {
  if (routes.length < 2) return null;
  return <div className="flex min-h-0 max-h-[40%] flex-col" data-trip-route-key>
    <p className="mb-2 shrink-0 text-xs text-fg-secondary">Tap a route to highlight it. Tap again to show every selected route.</p>
    <div className="grid min-h-0 max-h-48 grid-cols-1 gap-1 overflow-y-auto sm:grid-cols-2 lg:grid-cols-3" role="group" aria-label="Selected route key">
      {routes.map(({ id, index }) => {
        const trip = trips[id];
        const title = trip
          ? `${trip.start_place ?? trip.start_address ?? 'Start'} → ${trip.end_place ?? trip.end_address ?? 'Destination'}`
          : `Route ${index + 1}`;
        return <button type="button" key={id} data-route-id={id}
          aria-label={`Highlight route ${index + 1}: ${title}`}
          aria-pressed={highlightedId === id}
          onClick={() => onHighlight(highlightedId === id ? null : id)}
          className="flex min-h-11 items-center gap-3 rounded-lg px-3 py-2 text-left hover:bg-bg-elevated aria-pressed:bg-bg-elevated">
          <span className="flex shrink-0 items-center gap-2 text-xs text-fg-secondary">
            <svg width="24" height="8" aria-hidden="true"><path d="M2 4h20" stroke={tripRouteColor(index)} strokeWidth="4" strokeLinecap="round" /></svg>
            {index + 1}
          </span>
          <span className="min-w-0 text-xs text-fg">
            <span className="block break-words">{title}</span>
            {trip && <span className="mt-1 block text-fg-secondary">{formatAppDateTime(trip.started_at)}</span>}
          </span>
        </button>;
      })}
    </div>
  </div>;
}

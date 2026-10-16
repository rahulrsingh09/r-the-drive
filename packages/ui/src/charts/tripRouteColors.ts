export const TRIP_ROUTE_COLOR_COUNT = 16;

export function tripRouteColor(index: number) {
  const slot = index % TRIP_ROUTE_COLOR_COUNT;
  return slot < 6
    ? `var(--rm-map-route-${slot})`
    : `var(--rm-series-${String(slot + 1).padStart(2, '0')})`;
}

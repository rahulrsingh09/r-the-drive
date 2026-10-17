import type { TripRow } from '@panda/ui/tables';
import { createWidgetStore } from '../widgetStore';

type Selection = { selectedIds: string[]; tripRegistry: Record<string, TripRow>; contextKey: string };
type SelectionEvent = { type: 'reset'; context: string; force?: boolean | undefined } | { type: 'toggle'; id: string } | { type: 'clear' };
const selection = createWidgetStore<Selection, SelectionEvent>(
  { selectedIds: [], tripRegistry: {}, contextKey: '' },
  (state, event) => {
    switch (event.type) {
      case 'reset':
        return !event.force && state.contextKey === event.context ? state : { selectedIds: [], tripRegistry: {}, contextKey: event.context };
      case 'clear':
        return state.selectedIds.length ? { ...state, selectedIds: [] } : state;
      case 'toggle': {
        const selectedIds = state.selectedIds.slice();
        const index = selectedIds.indexOf(event.id);
        if (index < 0) selectedIds.push(event.id);
        else selectedIds.splice(index, 1);
        return { ...state, selectedIds };
      }
    }
  },
);

export function resetTripSelection(contextKey: string, options?: { force?: boolean }) {
  selection.send({ type: 'reset', context: contextKey, force: options?.force });
}
export function toggleTripSelection(id: string) { selection.send({ type: 'toggle', id }); }
export function clearTripSelection() { selection.send({ type: 'clear' }); }

/** Registration is deliberately silent; selection events publish the accumulated rows. */
export function registerTripsInStore(trips: TripRow[]) {
  Object.assign(selection.read().tripRegistry, Object.fromEntries(trips.map((trip) => [trip.id, trip])));
}
export function useTripSelection() {
  const { selectedIds, tripRegistry } = selection.useSnapshot();
  return { selectedIds, tripRegistry };
}

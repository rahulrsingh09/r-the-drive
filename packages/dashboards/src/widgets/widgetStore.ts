import { useSyncExternalStore } from 'react';

/** Commits one observable snapshot per event; reducers may suppress no-op events. */
export function createWidgetStore<State, Event>(
  initial: State,
  reduce: (state: State, event: Event) => State
) {
  let snapshot = initial;
  const subscriptions = new Set<() => void>();
  const read = () => snapshot;
  const subscribe = (listener: () => void) => {
    subscriptions.add(listener);
    return () => {
      subscriptions.delete(listener);
    };
  };
  return {
    read,
    send(event: Event) {
      const next = reduce(snapshot, event);
      if (Object.is(next, snapshot)) return;
      snapshot = next;
      subscriptions.forEach((listener) => listener());
    },
    useSnapshot() {
      return useSyncExternalStore(subscribe, read, read);
    },
  };
}

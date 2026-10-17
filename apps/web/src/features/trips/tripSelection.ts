import { useMemo, useSyncExternalStore } from 'react';

export function createTripSelection() {
  let value: number | null = null;
  let queued: number | null = null;
  let frame: number | undefined;
  const listeners = new Set<() => void>();
  const snapshot = () => value;
  const publish = () => {
    frame = undefined;
    if (queued === value) return;
    value = queued;
    for (const listener of listeners) listener();
  };
  return {
    snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
        if (!listeners.size && frame !== undefined) {
          cancelAnimationFrame(frame);
          frame = undefined;
          queued = value;
        }
      };
    },
    select(next: number | null) {
      if (queued === next) return;
      queued = next;
      if (frame === undefined) frame = requestAnimationFrame(publish);
    },
  };
}

export function useTripSelection(tripId: string) {
  const selection = useMemo(() => createTripSelection(), [tripId]);
  const index = useSyncExternalStore(selection.subscribe, selection.snapshot, selection.snapshot);
  return [index, selection.select] as const;
}

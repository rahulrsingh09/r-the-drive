import React from 'react';
import type { ReplaySpeed } from '../activity/replayModel';

export interface ReplayState {
  /** Replay clock, in trip minutes from the start. */
  minute: number;
  playing: boolean;
  speed: ReplaySpeed;
}

export interface ReplayStore {
  get(): ReplayState;
  subscribe(listener: () => void): () => void;
  toggle(): void;
  seek(minute: number): void;
  setSpeed(speed: ReplaySpeed): void;
  stop(): void;
}

/**
 * Replay clock for one trip. The replay never starts by itself; it starts at
 * the end of the trip so the whole route shows first. At 60× one second of
 * replay is one minute of the trip.
 */
export function createReplayStore(totalMinutes: number, now: () => number = () => performance.now()): ReplayStore {
  const total = Math.max(0, totalMinutes);
  let state: ReplayState = { minute: total, playing: false, speed: 60 };
  let frame: number | null = null;
  let last: number | null = null;
  const listeners = new Set<() => void>();
  const emit = (next: Partial<ReplayState>) => {
    state = { ...state, ...next };
    listeners.forEach((listener) => listener());
  };
  const cancel = () => {
    if (frame != null) cancelAnimationFrame(frame);
    frame = null;
    last = null;
  };
  const tick = () => {
    if (!state.playing) return;
    const at = now();
    const seconds = last == null ? 0 : (at - last) / 1000;
    last = at;
    const minute = Math.min(total, state.minute + (seconds * state.speed) / 60);
    if (minute >= total) {
      cancel();
      emit({ minute: total, playing: false });
      return;
    }
    emit({ minute });
    frame = requestAnimationFrame(tick);
  };
  return {
    get: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    toggle() {
      if (state.playing) { cancel(); emit({ playing: false }); return; }
      if (total <= 0) return;
      emit({ playing: true, minute: state.minute >= total ? 0 : state.minute });
      last = null;
      frame = requestAnimationFrame(tick);
    },
    seek(minute) { emit({ minute: Math.max(0, Math.min(total, minute)) }); },
    setSpeed(speed) { emit({ speed }); },
    stop() { cancel(); if (state.playing) emit({ playing: false }); },
  };
}

export function useReplayState(store: ReplayStore) {
  return React.useSyncExternalStore(store.subscribe, store.get, store.get);
}

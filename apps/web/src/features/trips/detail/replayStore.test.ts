import { afterEach, describe, expect, it, vi } from 'vitest';
import { createReplayStore } from './replayStore';

describe('replay store', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('starts at the end, restarts from 0 on play, and stops at the end', () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback));
    vi.stubGlobal('cancelAnimationFrame', () => undefined);
    let clock = 0;
    const store = createReplayStore(10, () => clock);
    expect(store.get()).toMatchObject({ minute: 10, playing: false, speed: 60 });
    store.toggle();
    expect(store.get()).toMatchObject({ minute: 0, playing: true });
    frames.shift()!(0);
    clock = 3000; // 3 s at 60× = 3 trip minutes
    frames.shift()!(0);
    expect(store.get().minute).toBeCloseTo(3);
    store.setSpeed(240);
    clock = 5000; // 2 s at 240× = 8 trip minutes, capped at the end
    frames.shift()!(0);
    expect(store.get()).toMatchObject({ minute: 10, playing: false });
  });

  it('seeks inside the trip', () => {
    const store = createReplayStore(10);
    store.seek(-4);
    expect(store.get().minute).toBe(0);
    store.seek(4);
    expect(store.get().minute).toBe(4);
  });
});

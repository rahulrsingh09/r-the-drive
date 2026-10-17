import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { useTimedAcknowledgement } from '../features/featureOperation';
import type { RestoreJob } from '@panda/types';
import { observeRestore } from '../components/settings/restorePolling';
import { createTripSelection } from '../features/trips/tripSelection';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

describe('owned asynchronous feature work', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('waits for restore reads to settle before scheduling another and ignores a late failure after disposal', async () => {
    const first = deferred<RestoreJob>();
    const observer = { read: vi.fn(() => first.promise), job: vi.fn(), unavailable: vi.fn(), completed: vi.fn() };
    const stop = observeRestore(observer);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(observer.read).toHaveBeenCalledTimes(1);
    stop();
    first.reject(new Error('local service replaced'));
    await Promise.resolve();
    await vi.runAllTimersAsync();
    expect(observer.unavailable).not.toHaveBeenCalled();
    expect(observer.read).toHaveBeenCalledTimes(1);
  });

  it('retries temporary restore failures and owns the completion reload timer', async () => {
    const observer = { read: vi.fn().mockRejectedValueOnce(new Error('restarting')).mockResolvedValue({ id: 'restore', phase: 'completed' }), job: vi.fn(), unavailable: vi.fn(), completed: vi.fn() };
    const stop = observeRestore(observer);
    await vi.advanceTimersByTimeAsync(1000);
    expect(observer.read).toHaveBeenCalledTimes(2);
    expect(observer.unavailable.mock.calls).toEqual([[true], [false]]);
    stop();
    await vi.runAllTimersAsync();
    expect(observer.completed).not.toHaveBeenCalled();
  });

  it('reloads once after completion and never retries a failed terminal job', async () => {
    const done = vi.fn();
    const stop = observeRestore({ read: vi.fn().mockResolvedValue({ id: 'restore', phase: 'completed' }), job: vi.fn(), unavailable: vi.fn(), completed: done });
    await vi.advanceTimersByTimeAsync(1200);
    expect(done).toHaveBeenCalledTimes(1);
    stop();
    const read = vi.fn().mockResolvedValue({ id: 'failed', phase: 'failed' });
    const failedStop = observeRestore({ read, job: vi.fn(), unavailable: vi.fn(), completed: done });
    await vi.advanceTimersByTimeAsync(5000);
    expect(read).toHaveBeenCalledTimes(1);
    failedStop();
  });

  it('disposes every pending copy acknowledgement timer on unmount', () => {
    const view = renderHook(() => useTimedAcknowledgement(1800));
    act(() => { view.result.current[1](); view.result.current[1](); });
    expect(view.result.current[0]).toBe(true);
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('coalesces trip selections per animation frame and cancels work when unsubscribed', () => {
    const selection = createTripSelection();
    const changed = vi.fn();
    const stop = selection.subscribe(changed);
    selection.select(1);
    selection.select(7);
    vi.advanceTimersToNextFrame();
    expect(selection.snapshot()).toBe(7);
    expect(changed).toHaveBeenCalledTimes(1);
    selection.select(9);
    stop();
    vi.advanceTimersToNextFrame();
    expect(selection.snapshot()).toBe(7);
  });
});

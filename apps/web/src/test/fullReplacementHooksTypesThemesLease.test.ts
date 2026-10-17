import { afterEach, expect, it, vi } from 'vitest';
import { withSessionLock } from '../../../../packages/hooks/src/api/sessionLock';

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

function databaseFixture(abort = false) {
  let lease: { owner: string; expires: number } | undefined;
  const writes: unknown[] = [];
  const close = vi.fn();
  const database = {
    close,
    transaction: () => {
      const transaction = { error: null, oncomplete: null as (() => void) | null, onerror: null as (() => void) | null, onabort: null as (() => void) | null,
        objectStore: () => ({
          get: () => {
            const read = { result: undefined as typeof lease, onsuccess: null as (() => void) | null };
            queueMicrotask(() => {
              if (abort) { transaction.onabort?.(); return; }
              read.result = lease;
              read.onsuccess?.();
              transaction.oncomplete?.();
            });
            return read;
          },
          put: (value: NonNullable<typeof lease>, key: string) => { lease = value; writes.push(['put', key]); },
          delete: (key: string) => { lease = undefined; writes.push(['delete', key]); },
        }),
      };
      return transaction;
    },
  };
  vi.stubGlobal('navigator', { userAgent: 'synthetic-browser' });
  vi.stubGlobal('indexedDB', { open: () => {
    const request = { result: database, onsuccess: null as (() => void) | null };
    queueMicrotask(() => request.onsuccess?.());
    return request;
  } });
  return { close, writes, replace: (value: typeof lease) => { lease = value; }, lease: () => lease };
}

it('releases an acquired lease on operation failure and closes the database', async () => {
  const db = databaseFixture();
  const failure = Object.assign(new Error('API failure'), { detail: { code: 'synthetic' } });
  await expect(withSessionLock(async () => { expect(db.lease()).toBeDefined(); throw failure; })).rejects.toBe(failure);
  expect(db.writes).toEqual([['put', 'panda-session-renewal'], ['delete', 'panda-session-renewal']]);
  expect(db.close).toHaveBeenCalledOnce();
});

it('does not delete a lease belonging to a replacement owner', async () => {
  const db = databaseFixture();
  await withSessionLock(async () => { db.replace({ owner: 'another-tab', expires: Date.now() + 200000 }); });
  expect(db.writes).toEqual([['put', 'panda-session-renewal']]);
  expect(db.lease()?.owner).toBe('another-tab');
});

it('fails closed on an aborted lease transaction', async () => {
  const db = databaseFixture(true);
  const operation = vi.fn();
  await expect(withSessionLock(operation)).rejects.toMatchObject({ code: 'NETWORK_ERROR', cause: expect.any(Error) });
  expect(operation).not.toHaveBeenCalled();
  expect(db.close).toHaveBeenCalledOnce();
});

it('does not acquire a corrupt unexpired lease and stops after its wait deadline', async () => {
  vi.useFakeTimers();
  const db = databaseFixture();
  db.replace({ owner: 'another-tab', expires: NaN });
  const operation = vi.fn();
  const result = withSessionLock(operation);
  const rejected = expect(result).rejects.toMatchObject({ code: 'NETWORK_ERROR' });
  await vi.advanceTimersByTimeAsync(120_250);
  await rejected;
  expect(operation).not.toHaveBeenCalled();
  expect(db.writes).toEqual([]);
  expect(db.close).toHaveBeenCalledOnce();
});

it('rejects before opening the database when lease identity cannot be generated', async () => {
  const open = vi.fn();
  const failure = new Error('identity unavailable');
  vi.stubGlobal('navigator', { userAgent: 'synthetic-browser' });
  vi.stubGlobal('indexedDB', { open });
  vi.stubGlobal('crypto', { randomUUID: () => { throw failure; } });
  await expect(withSessionLock(vi.fn())).rejects.toBe(failure);
  expect(open).not.toHaveBeenCalled();
});

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { dehydrate, QueryClient } from '@tanstack/react-query';
import { clearQueryCacheForUser, hydrateQueryCacheForUser, queryClient } from '../queryClient';

beforeEach(() => { vi.useFakeTimers(); localStorage.clear(); queryClient.clear(); });
afterEach(() => { queryClient.clear(); vi.clearAllTimers(); vi.useRealTimers(); });

it('persists only successful allowlisted queries and honors explicit privacy metadata', () => {
  localStorage.setItem('rm-auth', JSON.stringify({ state: { userId: 'synthetic-a' } }));
  queryClient.setQueryData(['vehicles'], [{ id: 'synthetic-vehicle' }]);
  queryClient.setQueryData(['auth'], { password: 'synthetic-secret' });
  queryClient.setQueryDefaults(['trips', 'private'], { meta: { persist: false } });
  queryClient.setQueryData(['trips', 'private'], ['private']);
  vi.advanceTimersByTime(750);
  const saved = JSON.parse(localStorage.getItem('rm-query-cache-v2-synthetic-a')!);
  expect(saved.userId).toBe('synthetic-a');
  expect(saved.state.queries.map((q: { queryKey: unknown }) => q.queryKey)).toEqual([['vehicles']]);
  expect(JSON.stringify(saved)).not.toContain('synthetic-secret');
});

it('rejects another user and expired cache, hydrates current data, and clears logout state', () => {
  const source = new QueryClient();
  source.setQueryData(['vehicles'], ['synthetic']);
  const state = dehydrate(source);
  const key = 'rm-query-cache-v2-synthetic-a';
  localStorage.setItem(key, JSON.stringify({ timestamp: Date.now(), userId: 'synthetic-b', state }));
  hydrateQueryCacheForUser('synthetic-a');
  expect(queryClient.getQueryData(['vehicles'])).toBeUndefined();
  expect(localStorage.getItem(key)).toBeNull();
  localStorage.setItem(key, JSON.stringify({ timestamp: Date.now() - 86_400_001, userId: 'synthetic-a', state }));
  hydrateQueryCacheForUser('synthetic-a');
  expect(localStorage.getItem(key)).toBeNull();
  localStorage.setItem(key, JSON.stringify({ timestamp: Date.now(), userId: 'synthetic-a', state }));
  hydrateQueryCacheForUser('synthetic-a');
  expect(queryClient.getQueryData(['vehicles'])).toEqual(['synthetic']);
  clearQueryCacheForUser('synthetic-a');
  expect(queryClient.getQueryCache().getAll()).toHaveLength(0);
  expect(localStorage.getItem(key)).toBeNull();
  source.clear();
});

it.each([401, '403', 429, '429'])('does not retry prohibited status %s', status => {
  const retry = queryClient.getDefaultOptions().queries?.retry as (attempt: number, error: unknown) => boolean;
  expect(retry(0, { status })).toBe(false);
  expect(retry(0, { code: 'AUTH_EXPIRED' })).toBe(false);
  expect(retry(1, { status: 500 })).toBe(true);
  expect(retry(2, null)).toBe(false);
});

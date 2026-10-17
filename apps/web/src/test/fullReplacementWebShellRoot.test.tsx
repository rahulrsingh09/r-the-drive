import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ ready: false, token: null as string | null, owner: null as string | null, theme: vi.fn(), timezone: vi.fn() }));
vi.mock('@panda/hooks', () => ({
  api: { getAppTimezone: mocks.timezone }, themeClient: { getPreferences: mocks.theme },
  useAuthReady: () => mocks.ready,
  useAuth: (select: (state: { accessToken: string | null; userId: string | null }) => unknown) => select({ accessToken: mocks.token, userId: mocks.owner }),
  queryKeys: { themePreferences: { forUser: (id: string) => ['theme', id] }, appTimezone: { current: ['timezone'] }, unitPreferences: { current: ['units'] } },
}));
import { useRootPreferences } from '../hooks/useRootPreferences';

beforeEach(() => {
  localStorage.clear();
  mocks.ready = false; mocks.token = null; mocks.owner = null;
  mocks.theme.mockReset().mockResolvedValue({ preferences: { schemaVersion: 2, mode: 'dark', selection: { kind: 'builtin', themeId: 'classic' } }, etag: 'synthetic' });
  mocks.timezone.mockReset().mockResolvedValue({ timezone: 'UTC' });
});

it('defers root preferences until session gating permits them and separates account caches', async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const view = renderHook(() => useRootPreferences(), { wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider> });
  expect(mocks.theme).not.toHaveBeenCalled(); expect(mocks.timezone).not.toHaveBeenCalled();
  mocks.ready = true; mocks.token = 'synthetic';
  view.rerender();
  await waitFor(() => expect(mocks.timezone).toHaveBeenCalledOnce());
  expect(mocks.theme).not.toHaveBeenCalled();
  mocks.owner = 'a'; view.rerender();
  await waitFor(() => expect(mocks.theme).toHaveBeenCalledOnce());
  expect(invalidate).toHaveBeenCalledWith({ queryKey: ['units'] });
  mocks.owner = 'b'; view.rerender();
  await waitFor(() => expect(mocks.theme).toHaveBeenCalledTimes(2));
  expect(client.getQueryData(['theme', 'a'])).toBeDefined();
  expect(client.getQueryData(['theme', 'b'])).toBeDefined();
  view.unmount(); client.clear();
});

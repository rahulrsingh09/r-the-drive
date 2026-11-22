import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const requestResponse = vi.fn();

vi.mock('@panda/hooks', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@panda/hooks')>();
  return { ...actual, useAuthReady: () => true, api: { ...actual.api, requestResponse: (...args: unknown[]) => requestResponse(...args) } };
});

import { ACTIVE_TRIP_API_AVAILABLE, useActiveTrip } from './useNowLive';

describe('useActiveTrip', () => {
  it('does not call the active-trip endpoint while the API does not serve it', async () => {
    expect(ACTIVE_TRIP_API_AVAILABLE).toBe(false);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: React.ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const { result } = renderHook(() => useActiveTrip('vehicle-1', true), { wrapper });
    await waitFor(() => expect(result.current.fetchStatus).toBe('idle'));
    expect(result.current.data ?? null).toBeNull();
    expect(requestResponse).not.toHaveBeenCalled();
  });
});

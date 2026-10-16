import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const hookMocks = vi.hoisted(() => ({
  getAppVersion: vi.fn(),
  getUpdateCheckSettings: vi.fn(),
}));

vi.mock('@panda/hooks', () => ({
  api: hookMocks,
  queryKeys: {
    appVersion: { current: ['app-version'] },
    updateCheck: { current: ['update-check-settings'] },
  },
}));

import { useGithubReleaseCheck } from '../useGithubReleaseCheck';
import {
  EMPTY_RELEASE_CHECK,
  RELEASE_CHECK_STORAGE_KEY,
  UPDATE_CHECK_INTERVALS_MS,
  readReleaseCheckSnapshot,
  writeReleaseCheckSnapshot,
} from '../../lib/releaseCheck';

function createClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
}

function createWrapper(client = createClient()) {
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe('useGithubReleaseCheck', () => {
  beforeEach(() => {
    writeReleaseCheckSnapshot(EMPTY_RELEASE_CHECK);
    localStorage.clear();
    readReleaseCheckSnapshot();
    hookMocks.getAppVersion.mockReset().mockResolvedValue({ version: '2026.09.4+dev' });
    hookMocks.getUpdateCheckSettings.mockReset().mockResolvedValue({ enabled: false, frequency: 'daily' });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it('does not contact GitHub while checks are disabled', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.settings.isSuccess).toBe(true));
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('does not contact GitHub again before the selected interval is due', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
    writeReleaseCheckSnapshot({
      lastAttemptAt: Date.now() - 1000,
      lastSuccessfulAt: Date.now() - 1000,
      latestVersion: '2026.10.1',
      updateAvailable: true,
      error: null,
    });
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.settings.isSuccess).toBe(true));
    expect(result.current.updateAvailable).toBe(true);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('checks the stable endpoint when due and reports the newer version', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ tag_name: '2026.10.1', draft: false, prerelease: false }),
    });
    vi.stubGlobal('fetch', fetchMock);
    hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.status.latestVersion).toBe('2026.10.1'));
    expect(result.current.updateAvailable).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe('https://api.github.com/repos/your-org/r-the-drive/releases/latest');
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ credentials: 'omit', referrerPolicy: 'no-referrer' });
  });

  it('keeps the previous update result after a failed or rate-limited request', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 403 });
    vi.stubGlobal('fetch', fetchMock);
    hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
    writeReleaseCheckSnapshot({
      lastAttemptAt: Date.now() - 2 * 24 * 60 * 60 * 1000,
      lastSuccessfulAt: Date.now() - 3 * 24 * 60 * 60 * 1000,
      latestVersion: '2026.10.1',
      updateAvailable: true,
      error: null,
    });
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper() });

    await waitFor(() => expect(result.current.status.error).toContain('HTTP 403'));
    expect(result.current.updateAvailable).toBe(true);
    expect(result.current.status.updateAvailable).not.toBe(false);
  });

  it('recomputes a cached update after the running version changes without another GitHub request', async () => {
    const client = createClient();
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
    writeReleaseCheckSnapshot({
      lastAttemptAt: Date.now(),
      lastSuccessfulAt: Date.now(),
      latestVersion: '2026.10.2',
      updateAvailable: true,
      error: null,
    });
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper(client) });
    await waitFor(() => expect(result.current.updateAvailable).toBe(true));

    act(() => client.setQueryData(['app-version'], { version: '2026.10.2' }));
    await waitFor(() => expect(result.current.status.updateAvailable).toBe(false));
    expect(result.current.updateAvailable).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it.each(['2026.10.2', '2026.10.3+dev.1', 'unknown', 'invalid'])(
    'does not reuse a persisted update badge on mounting version %s',
    async (version) => {
      vi.stubGlobal('fetch', vi.fn());
      hookMocks.getAppVersion.mockResolvedValue({ version });
      hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
      writeReleaseCheckSnapshot({
        lastAttemptAt: Date.now(),
        lastSuccessfulAt: Date.now(),
        latestVersion: '2026.10.2',
        updateAvailable: true,
        error: null,
      });
      const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper() });
      await waitFor(() => expect(result.current.version.isSuccess).toBe(true));
      expect(result.current.updateAvailable).toBe(false);
      expect(result.current.status.updateAvailable).toBe(version === 'unknown' || version === 'invalid' ? null : false);
      expect(fetch).not.toHaveBeenCalled();
    },
  );

  it('hides the badge when disabled even with a cached newer release', async () => {
    const client = createClient();
    vi.stubGlobal('fetch', vi.fn());
    hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
    writeReleaseCheckSnapshot({
      lastAttemptAt: Date.now(), lastSuccessfulAt: Date.now(),
      latestVersion: '2026.10.2', updateAvailable: true, error: null,
    });
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper(client) });
    await waitFor(() => expect(result.current.updateAvailable).toBe(true));
    act(() => client.setQueryData(['update-check-settings'], { enabled: false, frequency: 'daily' }));
    await waitFor(() => expect(result.current.updateAvailable).toBe(false));
    expect(result.current.status.latestVersion).toBe('2026.10.2');
    expect(fetch).not.toHaveBeenCalled();
  });

  it('reports an initial failed check as unknown rather than up to date', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('Offline')));
    hookMocks.getUpdateCheckSettings.mockResolvedValue({ enabled: true, frequency: 'daily' });
    const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper() });
    await waitFor(() => expect(result.current.status.error).toBe('Offline'));
    expect(result.current.status.updateAvailable).toBeNull();
    expect(result.current.status.lastSuccessfulAt).toBeNull();
    expect(result.current.updateAvailable).toBe(false);
  });

  describe('scheduling with browser storage failures', () => {
    function timedClient(frequency: keyof typeof UPDATE_CHECK_INTERVALS_MS = 'daily') {
      vi.useFakeTimers();
      const client = createClient();
      client.setQueryData(['app-version'], { version: '2026.09.4+dev' });
      client.setQueryData(['update-check-settings'], { enabled: true, frequency });
      return client;
    }

    async function advance(ms: number) {
      await act(async () => vi.advanceTimersByTimeAsync(ms));
    }

    it.each([
      ['reads', true], ['reads', false],
      ['writes', true], ['writes', false],
      ['quota', true], ['quota', false],
      ['access', true], ['access', false],
    ] as const)('keeps the daily cadence across focus and remounts with denied %s (success=%s)', async (failure, success) => {
      const client = timedClient();
      const fetchMock = success
        ? vi.fn().mockResolvedValue({
          ok: true, status: 200,
          json: async () => ({ tag_name: '2026.10.1', draft: false, prerelease: false }),
        })
        : vi.fn().mockResolvedValue({ ok: false, status: 403 });
      vi.stubGlobal('fetch', fetchMock);
      const deny = () => { throw new DOMException('Unavailable', failure === 'quota' ? 'QuotaExceededError' : 'SecurityError'); };
      if (failure === 'access') vi.spyOn(window, 'localStorage', 'get').mockImplementation(deny);
      else vi.spyOn(Storage.prototype, failure === 'reads' ? 'getItem' : 'setItem').mockImplementation(deny);

      const wrapper = createWrapper(client);
      const first = renderHook(() => useGithubReleaseCheck(), { wrapper });
      await advance(0);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      expect(first.result.current.status.checking).toBe(false);
      expect(first.result.current.status.error).toBe(success ? null : 'GitHub returned HTTP 403.');
      expect(first.result.current.status.latestVersion).toBe(success ? '2026.10.1' : null);
      act(() => {
        for (let i = 0; i < 3; i += 1) {
          window.dispatchEvent(new Event('focus'));
          document.dispatchEvent(new Event('visibilitychange'));
        }
      });
      await advance(1500);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      first.unmount();

      const second = renderHook(() => useGithubReleaseCheck(), { wrapper });
      await advance(UPDATE_CHECK_INTERVALS_MS.daily - 1501);
      expect(fetchMock).toHaveBeenCalledTimes(1);
      await advance(1);
      expect(fetchMock).toHaveBeenCalledTimes(2);
      expect(second.result.current.status.error).toBe(success ? null : 'GitHub returned HTTP 403.');
    });

    it('honors a frequency change while writes fail', async () => {
      const client = timedClient();
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 403 }));
      vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
        throw new DOMException('Full', 'QuotaExceededError');
      });
      renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper(client) });
      await advance(0);
      act(() => client.setQueryData(['update-check-settings'], { enabled: true, frequency: 'hourly' }));
      await advance(UPDATE_CHECK_INTERVALS_MS.hourly - 1);
      expect(fetch).toHaveBeenCalledTimes(1);
      await advance(1);
      expect(fetch).toHaveBeenCalledTimes(2);
    });

    it('shares in-memory attempts across simultaneous hook consumers', async () => {
      const client = timedClient();
      let finish!: (response: unknown) => void;
      vi.stubGlobal('fetch', vi.fn().mockImplementation(() => new Promise((resolve) => { finish = resolve; })));
      vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
        throw new DOMException('Full', 'QuotaExceededError');
      });
      const wrapper = createWrapper(client);
      const first = renderHook(() => useGithubReleaseCheck(), { wrapper });
      const second = renderHook(() => useGithubReleaseCheck(), { wrapper });
      await advance(1500);
      expect(fetch).toHaveBeenCalledTimes(1);
      expect(second.result.current.status.checking).toBe(true);
      first.unmount();
      await act(async () => finish({ ok: false, status: 403 }));
      await advance(0);
      expect(second.result.current.status.error).toContain('HTTP 403');
      await advance(UPDATE_CHECK_INTERVALS_MS.daily - 1500);
      expect(fetch).toHaveBeenCalledTimes(2);
      await act(async () => finish({ ok: false, status: 403 }));
    });

    it('adopts another tab’s attempt and reschedules from its timestamp', async () => {
      const client = timedClient();
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 403 }));
      const start = Date.now();
      writeReleaseCheckSnapshot({ ...EMPTY_RELEASE_CHECK, lastAttemptAt: start });
      const { result } = renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper(client) });
      await advance(UPDATE_CHECK_INTERVALS_MS.hourly);
      const remote = {
        lastAttemptAt: Date.now(), lastSuccessfulAt: Date.now(),
        latestVersion: '2026.10.1', updateAvailable: false, error: null,
      };
      localStorage.setItem(RELEASE_CHECK_STORAGE_KEY, JSON.stringify(remote));
      act(() => window.dispatchEvent(new StorageEvent('storage', { key: 'unrelated' })));
      expect(result.current.status.latestVersion).toBeNull();
      act(() => window.dispatchEvent(new StorageEvent('storage', { key: RELEASE_CHECK_STORAGE_KEY })));
      expect(result.current.status.latestVersion).toBe('2026.10.1');
      expect(result.current.updateAvailable).toBe(true);
      await advance(UPDATE_CHECK_INTERVALS_MS.daily - 1);
      expect(fetch).not.toHaveBeenCalled();
      await advance(1);
      expect(fetch).toHaveBeenCalledTimes(1);
    });

    it('splits a monthly wait into browser-safe timers without early requests', async () => {
      const client = timedClient('monthly');
      const start = Date.now();
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 403 }));
      writeReleaseCheckSnapshot({ ...EMPTY_RELEASE_CHECK, lastAttemptAt: start });
      const timers = vi.spyOn(window, 'setTimeout');
      renderHook(() => useGithubReleaseCheck(), { wrapper: createWrapper(client) });
      expect(timers).toHaveBeenCalledWith(expect.any(Function), 2_147_483_647);
      await advance(UPDATE_CHECK_INTERVALS_MS.monthly - 1);
      expect(fetch).not.toHaveBeenCalled();
      await advance(1);
      expect(fetch).toHaveBeenCalledTimes(1);
    });
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  EMPTY_RELEASE_CHECK,
  LATEST_RELEASE_API_URL,
  RELEASE_CHECK_STORAGE_KEY,
  UPDATE_CHECK_INTERVALS_MS,
  fetchLatestStableRelease,
  isNewerRelease,
  isReleaseCheckDue,
  readReleaseCheckSnapshot,
  writeReleaseCheckSnapshot,
} from '../releaseCheck';

describe('GitHub release checks', () => {
  beforeEach(() => {
    writeReleaseCheckSnapshot(EMPTY_RELEASE_CHECK);
    localStorage.clear();
    readReleaseCheckSnapshot();
  });
  afterEach(() => vi.restoreAllMocks());

  it('compares calendar releases and ignores the development build suffix', () => {
    expect(isNewerRelease('2026.09.4+dev', '2026.10.1')).toBe(true);
    expect(isNewerRelease('2026.09.4+dev', '2026.09.4')).toBe(false);
    expect(isNewerRelease('2026.09.4', '2026.09.3')).toBe(false);
    expect(isNewerRelease('unknown', '2026.10.1')).toBeNull();
    expect(isNewerRelease('2026.13.1', '2027.01.0')).toBeNull();
    expect(isNewerRelease('2026.10.1', 'invalid')).toBeNull();
  });

  it('uses exact hourly, daily, weekly, and 30-day monthly intervals', () => {
    const now = 2_000_000;
    expect(UPDATE_CHECK_INTERVALS_MS.monthly).toBe(30 * 24 * 60 * 60 * 1000);
    expect(isReleaseCheckDue(null, 'daily', now)).toBe(true);
    expect(isReleaseCheckDue(now - UPDATE_CHECK_INTERVALS_MS.hourly, 'hourly', now)).toBe(true);
    expect(isReleaseCheckDue(now - UPDATE_CHECK_INTERVALS_MS.weekly + 1, 'weekly', now)).toBe(false);
    expect(isReleaseCheckDue(now - UPDATE_CHECK_INTERVALS_MS.monthly, 'monthly', now)).toBe(true);
  });

  it('stores only browser-local check time and result', () => {
    const result = {
      lastAttemptAt: 100,
      lastSuccessfulAt: 100,
      latestVersion: '2026.10.1',
      updateAvailable: true,
      error: null,
    };
    writeReleaseCheckSnapshot(result);
    expect(readReleaseCheckSnapshot()).toEqual(result);
    expect(Object.keys(JSON.parse(localStorage.getItem(RELEASE_CHECK_STORAGE_KEY) ?? '{}')).sort()).toEqual([
      'error', 'lastAttemptAt', 'lastSuccessfulAt', 'latestVersion', 'updateAvailable',
    ]);
    localStorage.removeItem(RELEASE_CHECK_STORAGE_KEY);
    expect(readReleaseCheckSnapshot()).toEqual(EMPTY_RELEASE_CHECK);
  });

  it('retains a previously read snapshot when reads become denied', () => {
    const snapshot = { ...EMPTY_RELEASE_CHECK, lastAttemptAt: 100, error: 'HTTP 403' };
    localStorage.setItem(RELEASE_CHECK_STORAGE_KEY, JSON.stringify(snapshot));
    expect(readReleaseCheckSnapshot()).toEqual(snapshot);
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('Denied', 'SecurityError');
    });
    expect(readReleaseCheckSnapshot()).toEqual(snapshot);
  });

  it.each(['SecurityError', 'QuotaExceededError'])('retains failed writes after %s even with an older stored result', (name) => {
    const old = { ...EMPTY_RELEASE_CHECK, lastAttemptAt: 100 };
    writeReleaseCheckSnapshot(old);
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Cannot write', name);
    });
    const attempted = { ...old, lastAttemptAt: 200, error: 'HTTP 403' };
    writeReleaseCheckSnapshot(attempted);

    expect(readReleaseCheckSnapshot()).toEqual(attempted);
    expect(JSON.parse(localStorage.getItem(RELEASE_CHECK_STORAGE_KEY)!)).toEqual(old);
    readReleaseCheckSnapshot().lastAttemptAt = null;
    expect(readReleaseCheckSnapshot()).toEqual(attempted);
  });

  it('retains writes when access to localStorage itself is denied', () => {
    vi.spyOn(window, 'localStorage', 'get').mockImplementation(() => {
      throw new DOMException('Denied', 'SecurityError');
    });
    const snapshot = { ...EMPTY_RELEASE_CHECK, lastAttemptAt: 100 };
    writeReleaseCheckSnapshot(snapshot);
    expect(readReleaseCheckSnapshot()).toEqual(snapshot);
  });

  it('accepts a newer result from another tab after a failed local write', () => {
    const writes = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Full', 'QuotaExceededError');
    });
    writeReleaseCheckSnapshot({ ...EMPTY_RELEASE_CHECK, lastAttemptAt: 100 });
    expect(readReleaseCheckSnapshot().lastAttemptAt).toBe(100);
    writes.mockRestore();
    const remote = { ...EMPTY_RELEASE_CHECK, lastAttemptAt: 200, error: 'HTTP 403' };
    localStorage.setItem(RELEASE_CHECK_STORAGE_KEY, JSON.stringify(remote));
    expect(readReleaseCheckSnapshot()).toEqual(remote);

    localStorage.clear();
    expect(readReleaseCheckSnapshot()).toEqual(EMPTY_RELEASE_CHECK);
  });

  it('calls GitHub directly without browser credentials or a referrer', async () => {
    const fetcher = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ tag_name: '2026.10.1', draft: false, prerelease: false }),
    }) as unknown as typeof fetch;

    await expect(fetchLatestStableRelease(fetcher)).resolves.toBe('2026.10.1');
    expect(fetcher).toHaveBeenCalledWith(LATEST_RELEASE_API_URL, expect.objectContaining({
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      cache: 'no-store',
    }));
  });

  it('rejects malformed responses, prereleases, and GitHub rate limiting', async () => {
    const malformed = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({ tag_name: 'latest', draft: false, prerelease: false }) });
    const prerelease = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({ tag_name: '2026.10.1', prerelease: true }) });
    const limited = vi.fn().mockResolvedValue({ ok: false, status: 403 });

    await expect(fetchLatestStableRelease(malformed as unknown as typeof fetch)).rejects.toThrow(/invalid stable release version/i);
    await expect(fetchLatestStableRelease(prerelease as unknown as typeof fetch)).rejects.toThrow(/invalid stable release response/i);
    await expect(fetchLatestStableRelease(limited as unknown as typeof fetch)).rejects.toThrow('HTTP 403');
  });
});

describe('candidate build versions', () => {
  it('treats +dev.N as its base release', () => {
    expect(isNewerRelease('2026.10.1+dev.7', '2026.10.1')).toBe(false);
    expect(isNewerRelease('2026.10.1+dev.7', '2026.10.2')).toBe(true);
  });
});

import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { api, queryKeys } from '@panda/hooks';
import type { UpdateCheckFrequency } from '@panda/types';
import {
  RELEASE_CHECK_STATUS_EVENT,
  RELEASE_CHECK_STORAGE_KEY,
  UPDATE_CHECK_INTERVALS_MS,
  fetchLatestStableRelease,
  isNewerRelease,
  isReleaseCheckDue,
  readReleaseCheckSnapshot,
  writeReleaseCheckSnapshot,
  type ReleaseCheckStatus,
} from '../lib/releaseCheck';

const checkSession = { checking: false };

function announceCheckState() {
  window.dispatchEvent(new Event(RELEASE_CHECK_STATUS_EVENT));
}

function observeReleaseChanges(notify: () => void) {
  const storageChanged = (event: StorageEvent) => {
    if (event.key === null || event.key === RELEASE_CHECK_STORAGE_KEY) notify();
  };
  window.addEventListener(RELEASE_CHECK_STATUS_EVENT, notify);
  window.addEventListener('storage', storageChanged);
  return () => {
    window.removeEventListener(RELEASE_CHECK_STATUS_EVENT, notify);
    window.removeEventListener('storage', storageChanged);
  };
}

export function readReleaseCheckStatus(): ReleaseCheckStatus {
  return { ...readReleaseCheckSnapshot(), checking: checkSession.checking };
}

export function useReleaseCheckStatus(): ReleaseCheckStatus {
  const [status, setStatus] = React.useState(readReleaseCheckStatus);
  const version = useQuery({
    queryKey: queryKeys.appVersion.current,
    queryFn: () => api.getAppVersion(), enabled: false,
  });
  React.useEffect(() => {
    const refresh = () => setStatus(readReleaseCheckStatus());
    const stop = observeReleaseChanges(refresh);
    refresh();
    return stop;
  }, []);
  const currentVersion = version.data?.version;
  const comparable = status.lastSuccessfulAt !== null && status.latestVersion && currentVersion;
  return { ...status, updateAvailable: comparable ? isNewerRelease(currentVersion, status.latestVersion!) : null };
}

async function attemptReleaseCheck(currentVersion: string) {
  const previous = readReleaseCheckSnapshot();
  const timestamp = Date.now();
  checkSession.checking = true;
  announceCheckState();
  writeReleaseCheckSnapshot({ ...previous, lastAttemptAt: timestamp, error: null });
  try {
    const latestVersion = await fetchLatestStableRelease();
    writeReleaseCheckSnapshot({
      lastAttemptAt: timestamp, lastSuccessfulAt: timestamp, latestVersion,
      updateAvailable: isNewerRelease(currentVersion, latestVersion), error: null,
    });
  } catch (cause) {
    writeReleaseCheckSnapshot({
      ...previous, lastAttemptAt: timestamp,
      error: cause instanceof Error ? cause.message : 'The GitHub release check failed.',
    });
  } finally {
    checkSession.checking = false;
    announceCheckState();
  }
}

function startReleaseSchedule(currentVersion: string, frequency: UpdateCheckFrequency) {
  const clock = { disposed: false, timer: undefined as number | undefined };
  const interval = UPDATE_CHECK_INTERVALS_MS[frequency];
  const arm = () => {
    if (clock.disposed) return;
    window.clearTimeout(clock.timer);
    const previous = readReleaseCheckSnapshot().lastAttemptAt;
    const delay = previous === null ? 0 : previous + interval - Date.now();
    clock.timer = window.setTimeout(tick, Math.min(2_147_483_647, Math.max(250, delay)));
  };
  const tick = () => {
    if (clock.disposed || document.visibilityState === 'hidden') return;
    window.clearTimeout(clock.timer);
    if (checkSession.checking) return;
    if (isReleaseCheckDue(readReleaseCheckSnapshot().lastAttemptAt, frequency)) {
      void attemptReleaseCheck(currentVersion);
    } else {
      arm();
    }
  };
  const onVisible = () => { if (document.visibilityState === 'visible') tick(); };
  const stopChanges = observeReleaseChanges(arm);
  window.addEventListener('focus', onVisible);
  document.addEventListener('visibilitychange', onVisible);
  tick();
  return () => {
    clock.disposed = true;
    window.clearTimeout(clock.timer);
    stopChanges();
    window.removeEventListener('focus', onVisible);
    document.removeEventListener('visibilitychange', onVisible);
  };
}

export function useGithubReleaseCheck() {
  const status = useReleaseCheckStatus();
  const settings = useQuery({
    queryKey: queryKeys.updateCheck.current, queryFn: () => api.getUpdateCheckSettings(),
    staleTime: 60_000, refetchOnWindowFocus: true, retry: false,
  });
  const version = useQuery({
    queryKey: queryKeys.appVersion.current, queryFn: () => api.getAppVersion(),
    staleTime: 5 * 60_000, retry: false,
  });
  const enabled = settings.data?.enabled === true;
  const frequency = settings.data?.frequency ?? 'daily';
  const currentVersion = version.data?.version;
  React.useEffect(() => {
    const validVersion = currentVersion && isNewerRelease(currentVersion, currentVersion) !== null;
    if (enabled && validVersion) return startReleaseSchedule(currentVersion, frequency);
  }, [currentVersion, enabled, frequency]);
  return { settings, version, status, updateAvailable: enabled && status.updateAvailable === true };
}

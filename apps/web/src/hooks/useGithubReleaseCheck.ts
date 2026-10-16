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
  type ReleaseCheckSnapshot,
  type ReleaseCheckStatus,
} from '../lib/releaseCheck';

let releaseCheckInProgress = false;

export function readReleaseCheckStatus(): ReleaseCheckStatus {
  return { ...readReleaseCheckSnapshot(), checking: releaseCheckInProgress };
}

export function useReleaseCheckStatus(): ReleaseCheckStatus {
  const [status, setStatus] = React.useState<ReleaseCheckStatus>(readReleaseCheckStatus);
  const version = useQuery({
    queryKey: queryKeys.appVersion.current,
    queryFn: () => api.getAppVersion(),
    enabled: false,
  });
  React.useEffect(() => {
    const refresh = () => setStatus(readReleaseCheckStatus());
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key === RELEASE_CHECK_STORAGE_KEY) refresh();
    };
    window.addEventListener(RELEASE_CHECK_STATUS_EVENT, refresh);
    window.addEventListener('storage', onStorage);
    refresh();
    return () => {
      window.removeEventListener(RELEASE_CHECK_STATUS_EVENT, refresh);
      window.removeEventListener('storage', onStorage);
    };
  }, []);
  return {
    ...status,
    updateAvailable: status.lastSuccessfulAt !== null && status.latestVersion && version.data?.version
      ? isNewerRelease(version.data.version, status.latestVersion)
      : null,
  };
}

function snapshotForResult(
  now: number,
  latestVersion: string,
  currentVersion: string,
): ReleaseCheckSnapshot {
  return {
    lastAttemptAt: now,
    lastSuccessfulAt: now,
    latestVersion,
    updateAvailable: isNewerRelease(currentVersion, latestVersion),
    error: null,
  };
}

function frequencyInterval(frequency: UpdateCheckFrequency) {
  return UPDATE_CHECK_INTERVALS_MS[frequency];
}

export function useGithubReleaseCheck() {
  const status = useReleaseCheckStatus();
  const settings = useQuery({
    queryKey: queryKeys.updateCheck.current,
    queryFn: () => api.getUpdateCheckSettings(),
    staleTime: 60_000,
    refetchOnWindowFocus: true,
    retry: false,
  });
  const version = useQuery({
    queryKey: queryKeys.appVersion.current,
    queryFn: () => api.getAppVersion(),
    staleTime: 5 * 60_000,
    retry: false,
  });

  const enabled = settings.data?.enabled === true;
  const frequency = settings.data?.frequency ?? 'daily';
  const currentVersion = version.data?.version;

  React.useEffect(() => {
    if (!enabled || !currentVersion || isNewerRelease(currentVersion, currentVersion) === null) return;
    let disposed = false;
    let timer: number | undefined;
    const interval = frequencyInterval(frequency);

    const scheduleNext = (snapshot = readReleaseCheckSnapshot()) => {
      if (disposed) return;
      window.clearTimeout(timer);
      const dueAt = snapshot.lastAttemptAt === null ? Date.now() : snapshot.lastAttemptAt + interval;
      timer = window.setTimeout(runIfDue, Math.min(2_147_483_647, Math.max(250, dueAt - Date.now())));
    };

    const runIfDue = async () => {
      if (disposed || document.visibilityState === 'hidden') return;
      window.clearTimeout(timer);
      if (releaseCheckInProgress) return;
      const previous = readReleaseCheckSnapshot();
      if (!isReleaseCheckDue(previous.lastAttemptAt, frequency)) {
        scheduleNext(previous);
        return;
      }
      releaseCheckInProgress = true;
      window.dispatchEvent(new Event(RELEASE_CHECK_STATUS_EVENT));
      const attemptedAt = Date.now();
      const attempted: ReleaseCheckSnapshot = { ...previous, lastAttemptAt: attemptedAt, error: null };
      writeReleaseCheckSnapshot(attempted);

      try {
        const latestVersion = await fetchLatestStableRelease();
        const result = snapshotForResult(attemptedAt, latestVersion, currentVersion);
        writeReleaseCheckSnapshot(result);
      } catch (error) {
        const result: ReleaseCheckSnapshot = {
          ...previous,
          lastAttemptAt: attemptedAt,
          error: error instanceof Error ? error.message : 'The GitHub release check failed.',
        };
        writeReleaseCheckSnapshot(result);
      } finally {
        releaseCheckInProgress = false;
        window.dispatchEvent(new Event(RELEASE_CHECK_STATUS_EVENT));
      }
    };

    const checkWhenVisible = () => {
      if (document.visibilityState === 'visible') void runIfDue();
    };
    const reschedule = () => scheduleNext();
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key === RELEASE_CHECK_STORAGE_KEY) reschedule();
    };
    window.addEventListener(RELEASE_CHECK_STATUS_EVENT, reschedule);
    window.addEventListener('storage', onStorage);
    void runIfDue();
    window.addEventListener('focus', checkWhenVisible);
    document.addEventListener('visibilitychange', checkWhenVisible);
    return () => {
      disposed = true;
      if (timer !== undefined) window.clearTimeout(timer);
      window.removeEventListener('focus', checkWhenVisible);
      document.removeEventListener('visibilitychange', checkWhenVisible);
      window.removeEventListener(RELEASE_CHECK_STATUS_EVENT, reschedule);
      window.removeEventListener('storage', onStorage);
    };
  }, [currentVersion, enabled, frequency]);

  return {
    settings,
    version,
    status,
    updateAvailable: enabled && status.updateAvailable === true,
  };
}

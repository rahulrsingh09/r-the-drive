export const RELEASES_URL = 'https://github.com/your-org/r-the-drive/releases';
export const LATEST_RELEASE_API_URL = 'https://api.github.com/repos/your-org/r-the-drive/releases/latest';
export const RELEASE_CHECK_STORAGE_KEY = 'rm-github-release-check-v1';
export const RELEASE_CHECK_STATUS_EVENT = 'rm-github-release-status-change';

export const UPDATE_CHECK_INTERVALS_MS = {
  hourly: 60 * 60 * 1000,
  daily: 24 * 60 * 60 * 1000,
  weekly: 7 * 24 * 60 * 60 * 1000,
  monthly: 30 * 24 * 60 * 60 * 1000,
} as const;

export interface ReleaseCheckSnapshot {
  lastAttemptAt: number | null;
  lastSuccessfulAt: number | null;
  latestVersion: string | null;
  updateAvailable: boolean | null;
  error: string | null;
}

export interface ReleaseCheckStatus extends ReleaseCheckSnapshot {
  checking: boolean;
}

export const EMPTY_RELEASE_CHECK: ReleaseCheckSnapshot = {
  lastAttemptAt: null,
  lastSuccessfulAt: null,
  latestVersion: null,
  updateAvailable: null,
  error: null,
};

const localResult: { snapshot: ReleaseCheckSnapshot; durable: boolean } = {
  snapshot: { ...EMPTY_RELEASE_CHECK }, durable: true,
};

function parseVersion(value: string): string | null {
  const parts = /^v?(\d{4})\.(\d{1,2})\.(\d+)(?:\+dev(?:\.\d+)?)?$/.exec(value.trim());
  if (!parts) return null;
  const [year, month, patch] = parts.slice(1).map(Number) as [number, number, number];
  if (![year, month, patch].every(Number.isSafeInteger) || month < 1 || month > 12) return null;
  return `${year.toString().padStart(4, '0')}.${month.toString().padStart(2, '0')}.${patch.toString().padStart(16, '0')}`;
}

export function isNewerRelease(currentVersion: string, releaseTag: string): boolean | null {
  const current = parseVersion(currentVersion);
  const latest = parseVersion(releaseTag);
  return current === null || latest === null ? null : latest > current;
}

export function isReleaseCheckDue(
  lastAttemptAt: number | null,
  frequency: keyof typeof UPDATE_CHECK_INTERVALS_MS,
  now = Date.now(),
): boolean {
  return lastAttemptAt === null || now - lastAttemptAt >= UPDATE_CHECK_INTERVALS_MS[frequency];
}

function decodeSnapshot(value: unknown): ReleaseCheckSnapshot {
  const result = { ...EMPTY_RELEASE_CHECK };
  if (!value || typeof value !== 'object') return result;
  const fields = value as Record<string, unknown>;
  for (const key of ['lastAttemptAt', 'lastSuccessfulAt'] as const) {
    if (typeof fields[key] === 'number' && Number.isFinite(fields[key])) result[key] = fields[key];
  }
  for (const key of ['latestVersion', 'error'] as const) {
    if (typeof fields[key] === 'string') result[key] = fields[key];
  }
  if (typeof fields.updateAvailable === 'boolean') result.updateAvailable = fields.updateAvailable;
  return result;
}

export function readReleaseCheckSnapshot(storage?: Storage): ReleaseCheckSnapshot {
  try {
    const raw = (storage ?? window.localStorage).getItem(RELEASE_CHECK_STORAGE_KEY);
    const stored = decodeSnapshot(raw ? JSON.parse(raw) : null);
    const storedAttempt = stored.lastAttemptAt ?? -Infinity;
    const localAttempt = localResult.snapshot.lastAttemptAt ?? -Infinity;
    if (localResult.durable || storedAttempt > localAttempt) {
      Object.assign(localResult, { snapshot: stored, durable: true });
    }
  } catch {
    // A denied read cannot erase a result held by this tab.
  }
  return { ...localResult.snapshot };
}

export function writeReleaseCheckSnapshot(snapshot: ReleaseCheckSnapshot, storage?: Storage): void {
  Object.assign(localResult, { snapshot: { ...snapshot }, durable: false });
  try {
    (storage ?? window.localStorage).setItem(RELEASE_CHECK_STORAGE_KEY, JSON.stringify(snapshot));
    localResult.durable = true;
  } catch {
    // Keep the attempted timestamp even when persistence fails.
  }
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(RELEASE_CHECK_STATUS_EVENT));
}

function stableReleaseTag(payload: unknown): string {
  const fields = payload as { tag_name?: unknown; draft?: unknown; prerelease?: unknown };
  if (fields.draft !== false || fields.prerelease !== false || typeof fields.tag_name !== 'string') {
    throw new Error('GitHub returned an invalid stable release response.');
  }
  const tag = fields.tag_name.trim();
  if (tag.includes('+') || parseVersion(tag) === null) throw new Error('GitHub returned an invalid stable release version.');
  return tag;
}

export async function fetchLatestStableRelease(fetcher: typeof fetch = fetch): Promise<string> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), 15_000);
  try {
    const response = await fetcher(LATEST_RELEASE_API_URL, {
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      cache: 'no-store',
      signal: controller.signal,
    });
    if (!response.ok) throw new Error(`GitHub returned HTTP ${response.status}.`);
    return stableReleaseTag(await response.json());
  } finally {
    window.clearTimeout(timeout);
  }
}

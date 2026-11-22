import {
  MutationCache,
  QueryClient,
  QueryCache,
  dehydrate,
  hydrate,
  type DehydratedState,
  type Mutation,
  type Query,
  type QueryKey,
} from '@tanstack/react-query';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

const QUERY_CACHE_MAX_AGE_MS = 24 * 60 * 60 * 1000;
const MAX_PERSISTED_CACHE_BYTES = 1_500_000;
const QUERY_CACHE_SAVE_DELAY_MS = 750;
const PERSISTED_QUERY_ROOTS = new Set([
  'vehicles',
  'stats',
  'battery',
  'charging',
  'efficiency',
  'trips',
  'dashboards',
]);

interface PersistedQueryCache {
  timestamp: number;
  userId: string;
  state: DehydratedState;
}

/** Returns a per-user localStorage key so User A's cache never loads for User B. */
function cacheKey(userId: string) {
  return `rm-query-cache-v2-${userId}`;
}

const MAX_QUERY_RETRIES = 2;

/**
 * Retry policy for every query.
 * - 401, 403 and AUTH_EXPIRED: no retry. The user must sign in again.
 * - 404: no retry. The resource does not exist, so a retry gives the same answer.
 * - 429: no retry. A retry makes the throttle worse.
 * - Network errors, 5xx and errors without a status: retry up to 2 times.
 */
export function shouldRetryQuery(failureCount: number, error: unknown): boolean {
  const e = error as { status?: unknown; code?: unknown } | null;
  if (e && typeof e === 'object') {
    const status = Number(e.status);
    if (status === 401 || status === 403 || e.code === 'AUTH_EXPIRED') return false;
    if (status === 404) return false;
    if (status === 429) return false;
  }
  return failureCount < MAX_QUERY_RETRIES;
}

export const queryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: (error, query) => {
      reportClientError(error, {
        event: 'query.failed',
        area: 'query',
        operation: queryOperation(query.queryKey),
        severity: 'warn',
      });
    },
  }),
  mutationCache: new MutationCache({
    onError: (error, _variables, _context, mutation) => {
      reportClientError(error, {
        event: 'mutation.failed',
        area: 'mutation',
        operation: mutationOperation(mutation),
        severity: 'warn',
      });
    },
  }),
  defaultOptions: {
    queries: {
      retry: shouldRetryQuery,
      staleTime: 30 * 1000,
      gcTime: 24 * 60 * 60 * 1000,
      refetchOnReconnect: false,
      refetchOnWindowFocus: false,
      refetchOnMount: true,
    },
  },
});

installQueryCachePersistence(queryClient);
installSessionClearedHandler(queryClient);

/** Call this after the user's identity is known (post-login or post-refresh). */
export function hydrateQueryCacheForUser(userId: string) {
  if (typeof window === 'undefined') return;
  const key = cacheKey(userId);
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null || raw === '') return;
    const cached = JSON.parse(raw) as PersistedQueryCache;
    const usable = cached?.timestamp && cached.userId === userId
      && !(Date.now() - cached.timestamp > QUERY_CACHE_MAX_AGE_MS);
    if (usable) {
      hydrate(queryClient, cached!.state);
      return;
    }
  } catch {
    // Invalid snapshots are removed by the same path as expired snapshots.
  }
  window.localStorage.removeItem(key);
}

/** Call this on logout to wipe the persisted cache. */
export function clearQueryCacheForUser(userId: string) {
  if (typeof window === 'undefined') return;
  window.localStorage.removeItem(cacheKey(userId));
  queryClient.clear();
}

function installSessionClearedHandler(client: QueryClient) {
  if (typeof window === 'undefined') return;
  window.addEventListener('panda:session-cleared', () => {
    client.cancelQueries();
    client.clear();
  });
}

function installQueryCachePersistence(client: QueryClient) {
  if (typeof window === 'undefined') return;
  type PendingSave = { kind: 'delay'; id: ReturnType<typeof setTimeout> } | { kind: 'idle'; id: number };
  let pending: PendingSave | null = null;
  const browser = window as Window & {
    requestIdleCallback?: (callback: () => void, options: { timeout: number }) => number;
    cancelIdleCallback?: (id: number) => void;
  };

  const cancelPending = () => {
    if (pending?.kind === 'delay') clearTimeout(pending.id);
    if (pending?.kind === 'idle') browser.cancelIdleCallback?.(pending.id);
    pending = null;
  };
  const flush = () => {
    cancelPending();
    try {
      const auth = JSON.parse(window.localStorage.getItem('rm-auth') ?? 'null') as { state?: { userId?: string } } | null;
      const userId = auth?.state?.userId;
      if (!userId) return;
      const state = dehydrate(client, { shouldDehydrateQuery });
      const record: PersistedQueryCache = { timestamp: Date.now(), userId, state };
      const serialized = JSON.stringify(record);
      const key = cacheKey(userId);
      if (new TextEncoder().encode(serialized).byteLength <= MAX_PERSISTED_CACHE_BYTES) {
        window.localStorage.setItem(key, serialized);
      } else {
        window.localStorage.removeItem(key);
      }
    } catch {
      // The active cache remains usable without browser storage.
    }
  };
  const requestIdleSave = () => {
    pending = null;
    const requestIdle = browser.requestIdleCallback;
    if (!requestIdle) { flush(); return; }
    pending = { kind: 'idle', id: requestIdle(flush, { timeout: 2_000 }) };
  };
  client.getQueryCache().subscribe(() => {
    cancelPending();
    pending = { kind: 'delay', id: setTimeout(requestIdleSave, QUERY_CACHE_SAVE_DELAY_MS) };
  });
  window.addEventListener('beforeunload', flush);
}

function shouldDehydrateQuery(query: Query) {
  const eligible = query.state.status === 'success' && isPersistedQueryKey(query.queryKey);
  if (!eligible) return false;
  const { persist = true } = query.meta ?? {};
  return persist !== false;
}

function isPersistedQueryKey(queryKey: QueryKey) {
  const root = queryKey[0];
  return typeof root === 'string' && PERSISTED_QUERY_ROOTS.has(root);
}

function queryOperation(queryKey: QueryKey): string {
  return diagnosticOperation(queryKey, 'query');
}

function mutationOperation(mutation: Mutation<unknown, unknown, unknown, unknown>): string {
  return diagnosticOperation(mutation.options.mutationKey ?? [], 'mutation');
}

function diagnosticOperation(key: readonly unknown[], area: string): string {
  const [operation] = key;
  if (typeof operation !== 'string') return `unknown-${area}`;
  return operation;
}

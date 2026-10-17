export interface ApiFailureDetail {
  status?: number;
  code: string;
  message: string;
  method: string;
  path: string;
  requestId?: string | undefined;
  rateLimitSource?: string;
  rateLimitClass?: string;
  rateLimitLimit?: number;
  rateLimitRemaining?: number;
  rateLimitResetSeconds?: number;
  retryAfterSeconds?: number;
}

const publicPrefixes = ['/v1/auth/login', '/v1/auth/config', '/v1/auth/oidc/start', '/v1/auth/register', '/v1/auth/setup', '/v1/auth/account-invitations/', '/v1/auth/bootstrap', '/v1/auth/refresh'];
const metadataPrefixes = ['/v1/auth/me', '/v1/auth/preferences', '/v2/auth/preferences', '/v2/themes', '/v1/dashboards/by-slug/'];
const requestSources = new Map([
  ['/v1/auth/bootstrap', 'bootstrap'], ['/v1/auth/refresh', 'refresh'], ['/v1/auth/me', 'metadata'], ['/v1/auth/preferences', 'metadata'], ['/v1/vehicles/live', 'live_websocket'],
]);

export function isPublicAuthPath(path: string) { return publicPrefixes.some((prefix) => path.startsWith(prefix)); }
export function inferClientRateLimitClass(method: string, path: string) {
  const policies: Array<[boolean, string]> = [
    [isPublicAuthPath(path), 'auth_public'],
    [metadataPrefixes.some((prefix) => path.startsWith(prefix)), 'auth_metadata'],
    [path === '/v1/vehicles/live' || path.includes('/live-session'), 'heavy_read'],
    [/^(GET|HEAD|OPTIONS)$/.test(method.toUpperCase()), 'auth_read'],
  ];
  return policies.find(([matches]) => matches)?.[1] ?? 'auth_write';
}
export function classifyClientRequestSource(path: string) {
  return requestSources.get(path) ?? (path.startsWith('/v1/dashboards/by-slug/') ? 'dashboard_metadata' : 'protected_api');
}

function excerpt(value: string, limit: number) {
  const truncated = value.slice(0, limit - 1);
  return value.length > limit ? truncated + '…' : value;
}
export function formatApiError(detail: ApiFailureDetail) {
  return detail.status == null ? detail.message : [detail.status, detail.code + ':', excerpt(detail.message, 160)].join(' ');
}

const messages = {
  connection: { title: 'Connection lost', message: 'Unable to reach the server. Check your connection and try again.' },
  expired: { title: 'Session expired', message: 'Please sign in again to continue.' },
  denied: { title: 'Access denied', message: "You don't have permission to do that." },
  missing: { title: 'Not found', message: 'The requested resource could not be found.' },
  unavailable: { title: 'Server unavailable', message: 'The server is currently unavailable. Please try again in a moment.' },
  server: { title: 'Server error', message: 'Something went wrong on our end. Please try again later.' },
};

export function friendlyApiError(detail: ApiFailureDetail): { title: string; message: string } {
  const { code, status } = detail;
  if (code === 'NETWORK_ERROR') return { ...messages.connection };
  if (code === 'AUTH_EXPIRED') return { ...messages.expired };
  const fixed = new Map([[401, messages.expired], [403, messages.denied], [404, messages.missing], [502, messages.unavailable], [503, messages.unavailable], [504, messages.unavailable]]);
  const known = status === undefined ? undefined : fixed.get(status);
  if (known) return { ...known };
  if (status === 429) {
    const origins: Record<string, string> = { nginx: 'Edge proxy rate limit reached.', api: 'API rate limit reached.' };
    const text = origins[detail.rateLimitSource ?? ''] ?? 'Please wait a moment and try again.';
    const wait = detail.retryAfterSeconds;
    return { title: 'Too many requests', message: text + (wait == null ? '' : ` Try again in about ${Math.max(1, Math.ceil(wait))}s.`) };
  }
  return status != null && status >= 500 ? { ...messages.server } : { title: 'Something went wrong', message: excerpt(detail.message, 120) || 'An unexpected error occurred.' };
}

export function apiFailureDetailFrom(error: unknown): ApiFailureDetail | undefined {
  const detail = error instanceof Error ? (error as Error & { detail?: Partial<ApiFailureDetail> }).detail : undefined;
  return detail && typeof detail.code === 'string' && typeof detail.message === 'string' ? detail as ApiFailureDetail : undefined;
}

export function parsePositiveNumberHeader(value: string | null) {
  const candidate = value ? Number(value) : NaN;
  return candidate >= 0 && Number.isFinite(candidate) ? candidate : undefined;
}

export class RequestCooldowns {
  private deadlines: Record<string, number> = Object.create(null) as Record<string, number>;
  remember(detail: Pick<ApiFailureDetail, 'method' | 'path' | 'rateLimitClass' | 'retryAfterSeconds' | 'rateLimitResetSeconds'>) {
    const period = [detail.retryAfterSeconds, detail.rateLimitResetSeconds, 5].find((value) => value != null)!;
    const key = detail.rateLimitClass ?? inferClientRateLimitClass(detail.method, detail.path);
    this.deadlines[key] = Date.now() + period * 1000;
  }
  pending(method: string, path: string): ApiFailureDetail | undefined {
    const key = inferClientRateLimitClass(method, path);
    const deadline = this.deadlines[key];
    if (deadline == null) return undefined;
    const remaining = deadline - Date.now();
    if (remaining <= 0) { delete this.deadlines[key]; return undefined; }
    const seconds = Math.max(1, Math.ceil(remaining / 1000));
    return { status: 429, code: 'RATE_LIMITED', message: `Local backoff active for ${key}.`, method, path, rateLimitSource: 'client-cooldown', rateLimitClass: key, retryAfterSeconds: seconds, rateLimitResetSeconds: seconds };
  }
}

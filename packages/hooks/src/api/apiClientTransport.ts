// Session-aware execution stages for the Panda REST API.

import type { AuthTokens, RestoreJob, UploadBackupResponse } from '@panda/types';
import { isRecord } from './normalization';
import { withSessionLock } from './sessionLock';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

// ── Schedule & live-session types ─────────────────────────────────────────────

export interface ChargingSchedule {
  id: string;
  enabled: boolean;
  start_time_minutes: number | null;
  duration_minutes: number | null;
  amperage: number | null;
  location_lat: number | null;
  location_lng: number | null;
  week_days: string[] | null;
  rivian_updated_at: string | null;
  updated_at: string;
}

export interface ChargingScheduleInput {
  enabled: boolean;
  start_time_minutes?: number | null;
  duration_minutes?: number | null;
  amperage?: number | null;
  location_lat?: number | null;
  location_lng?: number | null;
  week_days?: string[] | null;
}

export interface DepartureOccurrence {
  type: 'RepeatsWeekly' | 'Once';
  days?: string[];
  time_minutes?: number;
}

export interface DepartureComfortSettings {
  seat_fl_heat?: number;
  seat_fr_heat?: number;
  seat_rl_heat?: number;
  seat_rr_heat?: number;
  cabin_temp_c?: number;
  defrost?: boolean;
}

export interface DepartureSchedule {
  id: string;
  rivian_schedule_id: string;
  name: string | null;
  enabled: boolean;
  occurrence: DepartureOccurrence | null;
  comfort_settings: DepartureComfortSettings | null;
  updated_at: string;
}

export interface DepartureScheduleInput {
  name?: string | null;
  enabled: boolean;
  occurrence?: DepartureOccurrence | null;
  comfort_settings?: DepartureComfortSettings | null;
}

export interface LiveSession {
  soc_pct: number | null;
  power_kw: number | null;
  energy_kwh: number | null;
  range_added_km: number | null;
  time_remaining_min: number | null;
  charger_type: string | null;
  ts: string;
  charge_rate_kph?: number | null;
  time_elapsed_seconds?: number | null;
  price?: number | null;
  currency?: string | null;
  is_free_session?: boolean | null;
  vehicle_charger_state?: string | null;
  started_at?: string | null;
  pack_energy_kwh?: number | null;
  thermal_energy_kwh?: number | null;
  provenance?: Partial<
    Record<
      'power_kw' | 'energy_kwh' | 'pack_energy_kwh' | 'thermal_energy_kwh' | 'time_remaining_min',
      { source: 'canonical' | 'parallax' | 'legacy_charging_session'; observed_at: string }
    >
  >;
}

export interface BackfillStatus {
  vehicle_id: string;
  history_backfilled_at: string | null;
  status: string | null;
  rivian_session_count: number | null;
  local_session_count: number;
  missing_source_count?: number | null;
}

// ─────────────────────────────────────────────────────────────────────────────

function isLoopbackHostname(hostname: string) {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '::1';
}

export function resolveApiBaseUrl(
  configuredBaseUrl = (() => {
    const env =
      typeof import.meta !== 'undefined'
        ? (import.meta as { env?: { VITE_API_URL?: string; VITE_PANDA_API_BASE_URL?: string } }).env
        : undefined;
    return env?.VITE_API_URL ?? env?.VITE_PANDA_API_BASE_URL ?? '';
  })(),
  location: Pick<Location, 'hostname' | 'origin'> | undefined = typeof window === 'undefined'
    ? undefined
    : window.location
) {
  if (!configuredBaseUrl || !location) {
    return configuredBaseUrl;
  }

  try {
    const url = new URL(configuredBaseUrl, location.origin);
    // Route loopback targets through the app origin so dev traffic uses the
    // Vite proxy (`/v1`) rather than direct cross-origin calls to :3001.
    // This avoids CORS preflight amplification and keeps WS/HTTP behavior
    // consistent for both localhost and LAN clients.
    if (isLoopbackHostname(url.hostname)) {
      return '';
    }
  } catch {
    return configuredBaseUrl;
  }

  return configuredBaseUrl;
}

let _base: string | undefined;
function getBase(): string {
  if (_base === undefined) _base = resolveApiBaseUrl();
  return _base;
}

/** Override the API base URL — useful in tests and server-side rendering contexts.
 *  Pass `undefined` to clear the override and re-derive from env on next call. */
export function setApiBaseUrl(url: string | undefined): void {
  _base = url;
}

interface ApiFailureDetail {
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

const AUTH_REFRESH_EXCLUDED_PATHS = new Set([
  '/v1/auth/login',
  '/v1/auth/config',
  '/v1/auth/oidc/start',
  '/v1/auth/register',
  '/v1/auth/setup',
  '/v1/auth/account-invitations/preview',
  '/v1/auth/account-invitations/accept',
  '/v1/auth/bootstrap',
  '/v1/auth/refresh',
]);

const CLIENT_RATE_LIMIT_SOURCE = 'client-cooldown';
const DEFAULT_CLIENT_BACKOFF_SECONDS = 5;

type AuthChangeHandler = (tokens: AuthTokens | null) => void;

export class RequestTransport {
  private accessToken: string | null = null;
  private authChangeHandler: AuthChangeHandler | null = null;
  private refreshPromise: Promise<AuthTokens | null> | null = null;
  private authExpiredReported = false;
  private rateLimitCooldowns = new Map<string, number>();

  setToken(token: string | null) {
    this.accessToken = token;
    if (token) this.authExpiredReported = false;
  }

  onAuthChange(handler: AuthChangeHandler) {
    this.authChangeHandler = handler;
  }

  private applyTokens(tokens: AuthTokens) {
    this.setToken(tokens.access_token);
    this.authExpiredReported = false;
    this.authChangeHandler?.(tokens);
  }

  private clearTokens() {
    this.setToken(null);
    this.authChangeHandler?.(null);
  }

  protected renewSession(bootstrap = false): Promise<AuthTokens | null> {
    if (!this.refreshPromise) {
      this.refreshPromise = withSessionLock(async () => {
        const abort = new AbortController();
        const timeout = setTimeout(() => abort.abort(), 30_000);
        try {
          const res = await this.requestResponse(
            'POST',
            bootstrap ? '/v1/auth/bootstrap' : '/v1/auth/refresh',
            undefined,
            undefined,
            false,
            false,
            undefined,
            abort.signal
          );
          if (res.status === 204) return null;
          const request = {
            method: 'POST',
            path: bootstrap ? '/v1/auth/bootstrap' : '/v1/auth/refresh',
            reportErrors: false,
          };
          const tokens = await this.decodeJson<AuthTokens>(res, request);
          if (
            !isRecord(tokens) ||
            typeof tokens.access_token !== 'string' ||
            !tokens.access_token ||
            typeof tokens.expires_in !== 'number' ||
            !Number.isFinite(tokens.expires_in) ||
            !(tokens.default_vehicle_id === null || typeof tokens.default_vehicle_id === 'string')
          ) {
            throw this.responseFailure(res, request, 'INVALID_RESPONSE', undefined);
          }
          this.applyTokens(tokens);
          return tokens;
        } finally {
          clearTimeout(timeout);
        }
      }).finally(() => {
        this.refreshPromise = null;
      });
    }

    return this.refreshPromise;
  }

  protected async refreshAccessToken(): Promise<AuthTokens> {
    const tokens = await this.renewSession();
    if (!tokens)
      throw Object.assign(new Error('Session expired. Sign in again.'), {
        status: 401,
        code: 'AUTH_EXPIRED',
      });
    return tokens;
  }

  private headers(path: string): Record<string, string> {
    const h: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.accessToken && !isPublicAuthPath(path)) {
      h['Authorization'] = `Bearer ${this.accessToken}`;
    }
    return h;
  }

  /**
   * Fetch a first-party browser proxy without ever copying credentials to an
   * upstream URL. MapLibre and Iconify own their transport, so they cannot use
   * the normal typed request method directly.
   */
  proxyFetch(input: RequestInfo | URL, init: RequestInit = {}): Promise<Response> {
    const rawUrl =
      typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    const origin = typeof window === 'undefined' ? 'http://localhost' : window.location.origin;
    let firstPartyProxy = false;
    let proxyUrl: URL | undefined;
    try {
      const url = new URL(rawUrl, origin);
      proxyUrl = url;
      firstPartyProxy =
        url.origin === origin && /^\/v1\/external\/(basemap|iconify)(?:\/|$)/.test(url.pathname);
    } catch {
      // Let fetch report an invalid URL; it is not a connection proxy request.
    }

    const requestInit =
      firstPartyProxy && this.accessToken
        ? (() => {
            const headers = new Headers(init.headers);
            headers.set('Authorization', `Bearer ${this.accessToken}`);
            return { ...init, credentials: 'same-origin' as const, headers };
          })()
        : init;

    const request = fetch(input, requestInit);
    if (!firstPartyProxy) return request;

    const method = init.method ?? 'GET';
    return request
      .then((response) => {
        if (!response.ok) {
          reportClientError(new Error(`First-party proxy returned HTTP ${response.status}`), {
            event: 'proxy.response_failed',
            area: 'api',
            operation: 'first-party-proxy',
            method,
            path: proxyUrl?.pathname,
            status: response.status,
            code: `HTTP_${response.status}`,
            requestId: response.headers.get('x-request-id') ?? undefined,
            severity: 'warn',
          });
        }
        return response;
      })
      .catch((error: unknown) => {
        reportClientError(error, {
          event: 'proxy.request_failed',
          area: 'api',
          operation: 'first-party-proxy',
          method,
          path: proxyUrl?.pathname,
          severity: 'warn',
        });
        throw error;
      });
  }

  /**
   * Retrieve a first-party protected asset with the normal session bearer
   * token. This deliberately keeps credentials out of browser image URLs.
   */
  async authenticatedAsset(path: string): Promise<Response> {
    if (!path.startsWith('/v1/vehicle-image-cache/')) {
      throw new Error('authenticatedAsset only accepts first-party vehicle artwork paths');
    }
    return this.requestResponse('GET', path, undefined, undefined, true, false);
  }

  private assertRateLimitCooldown(method: string, path: string) {
    const limiterClass = inferClientRateLimitClass(method, path);
    const cooldownUntil = this.rateLimitCooldowns.get(limiterClass);
    if (cooldownUntil == null) return;

    const remainingMs = cooldownUntil - Date.now();
    if (remainingMs <= 0) {
      this.rateLimitCooldowns.delete(limiterClass);
      return;
    }

    const retryAfterSeconds = Math.max(1, Math.ceil(remainingMs / 1000));
    const detail: ApiFailureDetail = {
      status: 429,
      code: 'RATE_LIMITED',
      message: `Local backoff active for ${limiterClass}.`,
      method,
      path,
      rateLimitSource: CLIENT_RATE_LIMIT_SOURCE,
      rateLimitClass: limiterClass,
      retryAfterSeconds,
      rateLimitResetSeconds: retryAfterSeconds,
    };
    throw Object.assign(new Error(formatApiError(detail)), {
      status: 429,
      code: detail.code,
      detail,
    });
  }

  private rememberRateLimitCooldown(
    detail: Pick<
      ApiFailureDetail,
      'method' | 'path' | 'rateLimitClass' | 'retryAfterSeconds' | 'rateLimitResetSeconds'
    >
  ) {
    const limiterClass =
      detail.rateLimitClass ?? inferClientRateLimitClass(detail.method, detail.path);
    const waitSeconds =
      detail.retryAfterSeconds ?? detail.rateLimitResetSeconds ?? DEFAULT_CLIENT_BACKOFF_SECONDS;
    this.rateLimitCooldowns.set(limiterClass, Date.now() + waitSeconds * 1000);
  }

  protected execute<T>(
    request: ApiRequest & { response: 'raw'; decode: (response: Response) => T | Promise<T> }
  ): Promise<T>;
  protected execute<T, Wire = T>(
    request: ApiRequest & { response?: 'json'; decode?: (value: Wire) => T | Promise<T> }
  ): Promise<T>;
  protected async execute<T, Wire = T>(
    request: ApiRequest & {
      response?: 'json' | 'raw';
      decode?: ((value: Wire) => T | Promise<T>) | ((response: Response) => T | Promise<T>);
    }
  ): Promise<T> {
    const response = await this.send(request);
    if (request.response === 'raw') {
      return (request.decode as (response: Response) => T | Promise<T>)(response);
    }
    const payload = await this.decodeJson<Wire>(response, request);
    if (!request.decode) return payload as unknown as T;
    try {
      return await (request.decode as (value: Wire) => T | Promise<T>)(payload);
    } catch (cause) {
      throw this.responseFailure(response, request, 'INVALID_RESPONSE', cause);
    }
  }

  private async decodeJson<T>(response: Response, request: ApiRequest): Promise<T> {
    if (response.status === 204) return undefined as T;
    try {
      return (await response.json()) as T;
    } catch (cause) {
      throw this.responseFailure(response, request, 'INCOMPLETE_RESPONSE', cause);
    }
  }

  private responseFailure(response: Response, request: ApiRequest, code: string, cause: unknown) {
    const detail: ApiFailureDetail = {
      code,
      message:
        code === 'INCOMPLETE_RESPONSE'
          ? 'The server response was interrupted. Please retry.'
          : 'The server returned an invalid response. Please retry.',
      method: request.method,
      path: request.path,
      requestId: response.headers.get('x-request-id') ?? undefined,
    };
    const error = Object.assign(apiFailure(detail), { cause });
    if (request.reportErrors !== false) this.reportFailure(detail, error);
    return error;
  }

  async requestResponse(
    method: string,
    path: string,
    body?: unknown,
    params?: Record<string, string | number>,
    retryOnUnauthorized = true,
    reportErrors = true,
    extraHeaders?: Record<string, string>,
    signal?: AbortSignal
  ): Promise<Response> {
    return this.send({
      method,
      path,
      body,
      params,
      retryOnUnauthorized,
      reportErrors,
      extraHeaders,
      signal,
    });
  }

  private async send(request: ApiRequest): Promise<Response> {
    this.assertRateLimitCooldown(request.method, request.path);
    const normalized = normalizeRequest(request);
    const tokenAtStart = this.accessToken;
    const response = await this.fetchAttempt(normalized);
    if (response.ok) return response;

    const canRenew =
      response.status === 401 &&
      normalized.retryOnUnauthorized &&
      tokenAtStart &&
      this.accessToken &&
      !AUTH_REFRESH_EXCLUDED_PATHS.has(request.path);
    if (canRenew) {
      // A delayed 401 may belong to a token that another request already renewed.
      if (this.accessToken === tokenAtStart) await this.recoverSession(response, request);
      this.assertRateLimitCooldown(request.method, request.path);
      const retry = await this.fetchAttempt(normalized);
      if (retry.ok) return retry;
      throw await this.httpFailure(retry, request);
    }
    throw await this.httpFailure(response, request);
  }

  private async fetchAttempt(request: NormalizedRequest): Promise<Response> {
    const { method, path, reportErrors, signal } = request;
    try {
      signal?.throwIfAborted();
      return await fetch(request.url, {
        method,
        headers: { ...this.headers(path), ...request.extraHeaders },
        credentials: 'include',
        ...(signal ? { signal } : {}),
        ...(request.body !== undefined ? { body: request.body } : {}),
      });
    } catch (cause) {
      if (!isFetchConnectionError(cause) && !signal?.aborted) {
        if (reportErrors) {
          reportClientError(cause, {
            event: 'api.request_failed',
            area: 'api',
            operation: classifyClientRequestSource(path),
            method,
            path,
            severity: 'warn',
          });
        }
        throw cause;
      }
      const detail: ApiFailureDetail = {
        code: 'NETWORK_ERROR',
        message: 'Unable to reach the server. Check your connection and try again.',
        method,
        path,
      };
      const error = Object.assign(apiFailure(detail), { cause });
      if (reportErrors) this.reportFailure(detail, error);
      throw error;
    }
  }

  private async recoverSession(response: Response, request: ApiRequest): Promise<void> {
    try {
      await this.refreshAccessToken();
    } catch (cause) {
      const failure = apiFailureDetailFrom(cause);
      const code =
        failure?.code ?? (cause instanceof Error && 'code' in cause ? cause.code : undefined);
      if (
        code === 'NETWORK_ERROR' ||
        code === 'INCOMPLETE_RESPONSE' ||
        code === 'INVALID_RESPONSE' ||
        (failure?.status != null && failure.status >= 500)
      ) {
        if (request.reportErrors !== false && failure) {
          this.reportFailure(failure, cause instanceof Error ? cause : new Error(failure.message));
        }
        throw cause;
      }
      this.clearTokens();
      const detail: ApiFailureDetail = {
        status: response.status,
        code: 'AUTH_EXPIRED',
        message: `Session expired while calling ${request.method} ${request.path}. Sign in again.`,
        method: request.method,
        path: request.path,
        requestId: response.headers.get('x-request-id') ?? undefined,
        ...parseRateLimitHeaders(response.headers, request.method, request.path),
      };
      const error = apiFailure(detail);
      this.reportFailure(detail, error);
      throw error;
    }
  }

  private async httpFailure(response: Response, request: ApiRequest): Promise<Error> {
    const payload: unknown = await response.json().catch(() => null);
    const envelope = isRecord(payload) && isRecord(payload.error) ? payload.error : {};
    const detail: ApiFailureDetail = {
      status: response.status,
      code: typeof envelope.code === 'string' ? envelope.code : 'unknown',
      message: typeof envelope.message === 'string' ? envelope.message : response.statusText,
      method: request.method,
      path: request.path,
      requestId: response.headers.get('x-request-id') ?? undefined,
      ...parseRateLimitHeaders(response.headers, request.method, request.path),
    };
    if (response.status === 429) this.rememberRateLimitCooldown(detail);
    const error = apiFailure(detail);
    if (request.reportErrors !== false) this.reportFailure(detail, error);
    return error;
  }

  async apiFetch<T>(method: string, path: string, body?: unknown): Promise<T> {
    return this.execute<T>({ method, path, body });
  }

  protected async uploadFile(
    file: File,
    onProgress?: (loaded: number, total: number) => void
  ): Promise<UploadBackupResponse> {
    return new Promise((resolve, reject) => {
      const request = new XMLHttpRequest();
      request.open('POST', `${getBase()}/v1/admin/backups/imports`);
      request.withCredentials = true;
      request.setRequestHeader('Content-Type', file.type || 'application/gzip');
      request.setRequestHeader('X-Panda-File-Name', file.name.replace(/[^a-zA-Z0-9._-]/g, '_'));
      if (this.accessToken) request.setRequestHeader('Authorization', `Bearer ${this.accessToken}`);
      request.upload.onprogress = (event) =>
        onProgress?.(event.loaded, event.lengthComputable ? event.total : file.size);
      request.onerror = request.onabort = () => reject(new Error('Backup upload was interrupted.'));
      request.onload = () => {
        let payload: unknown;
        try {
          payload = request.responseText ? JSON.parse(request.responseText) : null;
        } catch {
          payload = null;
        }
        if (request.status >= 200 && request.status < 300) {
          if (!isRecord(payload) || !isRecord(payload.artifact)) {
            reject(
              Object.assign(new Error('Backup upload returned an invalid response.'), {
                code: 'INVALID_RESPONSE',
                status: request.status,
              })
            );
          } else {
            resolve(payload as unknown as UploadBackupResponse);
          }
          return;
        }
        const message =
          isRecord(payload) && isRecord(payload.error) && typeof payload.error.message === 'string'
            ? payload.error.message
            : undefined;
        reject(
          Object.assign(new Error(message || `Backup upload failed (${request.status}).`), {
            status: request.status,
          })
        );
      };
      request.send(file);
    });
  }

  protected async restoreStatus(jobId: string, capabilityToken: string): Promise<RestoreJob> {
    const response = await fetch(`${getBase()}/v1/restore-runtime/jobs/${jobId}`, {
      headers: { 'X-Panda-Restore-Token': capabilityToken },
      credentials: 'same-origin',
    });
    if (!response.ok) throw new Error(`Restore status is unavailable (${response.status}).`);
    return response.json() as Promise<RestoreJob>;
  }

  private reportFailure(detail: ApiFailureDetail, error: Error = new Error(detail.message)) {
    if (detail.path === '/v1/auth/refresh' && detail.status === 401) return;

    if (detail.code === 'AUTH_EXPIRED') {
      if (this.authExpiredReported) return;
      this.authExpiredReported = true;
    }

    reportClientError(error, {
      event: 'api.request_failed',
      area: 'api',
      operation: classifyClientRequestSource(detail.path),
      method: detail.method,
      path: detail.path,
      status: detail.status,
      code: detail.code,
      requestId: detail.requestId,
      severity: 'warn',
    });

    if (typeof window !== 'undefined') {
      const onLoginRoute = window.location.pathname === '/login';
      if (onLoginRoute && !isPublicAuthPath(detail.path)) {
        return;
      }

      const { title, message } = friendlyApiError(detail);
      window.dispatchEvent(
        new CustomEvent('panda:toast', {
          detail: {
            title,
            message,
            variant: 'error',
            code: detail.code,
          },
        })
      );

      if (detail.code === 'AUTH_EXPIRED') {
        window.dispatchEvent(new CustomEvent('panda:auth-expired', { detail }));
      }
    }
  }
}

function parseRateLimitHeaders(headers: Headers, method: string, path: string) {
  const rateLimitSource = headers.get('x-panda-ratelimit-source') ?? undefined;
  const rateLimitClass =
    headers.get('x-panda-ratelimit-class') ?? inferClientRateLimitClass(method, path);
  const rateLimitLimit = parsePositiveNumberHeader(headers.get('x-ratelimit-limit'));
  const rateLimitRemaining = parsePositiveNumberHeader(headers.get('x-ratelimit-remaining'));
  const rateLimitResetSeconds = parsePositiveNumberHeader(
    headers.get('x-panda-ratelimit-reset') ?? headers.get('x-ratelimit-after')
  );
  const retryAfterSeconds =
    parsePositiveNumberHeader(headers.get('retry-after')) ?? rateLimitResetSeconds;

  return {
    ...(rateLimitSource ? { rateLimitSource } : {}),
    ...(rateLimitClass ? { rateLimitClass } : {}),
    ...(rateLimitLimit !== undefined ? { rateLimitLimit } : {}),
    ...(rateLimitRemaining !== undefined ? { rateLimitRemaining } : {}),
    ...(rateLimitResetSeconds !== undefined ? { rateLimitResetSeconds } : {}),
    ...(retryAfterSeconds !== undefined ? { retryAfterSeconds } : {}),
  };
}

function parsePositiveNumberHeader(value: string | null) {
  if (!value) return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : undefined;
}

function inferClientRateLimitClass(method: string, path: string) {
  if (isPublicAuthPath(path)) {
    return 'auth_public';
  }

  if (
    path.startsWith('/v1/auth/me') ||
    path.startsWith('/v1/auth/preferences') ||
    path.startsWith('/v2/auth/preferences') ||
    path.startsWith('/v2/themes') ||
    path.startsWith('/v1/dashboards/by-slug/')
  ) {
    return 'auth_metadata';
  }

  if (path === '/v1/vehicles/live' || path.includes('/live-session')) {
    return 'heavy_read';
  }

  if (['GET', 'HEAD', 'OPTIONS'].includes(method.toUpperCase())) {
    return 'auth_read';
  }

  return 'auth_write';
}

function isPublicAuthPath(path: string) {
  return (
    path.startsWith('/v1/auth/login') ||
    path.startsWith('/v1/auth/config') ||
    path.startsWith('/v1/auth/oidc/start') ||
    path.startsWith('/v1/auth/register') ||
    path.startsWith('/v1/auth/setup') ||
    path.startsWith('/v1/auth/account-invitations/') ||
    path.startsWith('/v1/auth/bootstrap') ||
    path.startsWith('/v1/auth/refresh')
  );
}

function classifyClientRequestSource(path: string) {
  if (path === '/v1/auth/bootstrap') return 'bootstrap';
  if (path === '/v1/auth/refresh') return 'refresh';
  if (path === '/v1/auth/me' || path === '/v1/auth/preferences') return 'metadata';
  if (path === '/v1/vehicles/live') return 'live_websocket';
  if (path.startsWith('/v1/dashboards/by-slug/')) return 'dashboard_metadata';
  return 'protected_api';
}

function formatApiError(detail: ApiFailureDetail) {
  if (detail.status == null) return detail.message;
  return `${detail.status} ${detail.code}: ${truncate(detail.message, 160)}`;
}

function friendlyApiError(detail: ApiFailureDetail): { title: string; message: string } {
  const { status, code } = detail;
  if (code === 'NETWORK_ERROR') {
    return {
      title: 'Connection lost',
      message: 'Unable to reach the server. Check your connection and try again.',
    };
  }

  if (code === 'AUTH_EXPIRED')
    return { title: 'Session expired', message: 'Please sign in again to continue.' };
  if (status === 401)
    return { title: 'Session expired', message: 'Please sign in again to continue.' };
  if (status === 403)
    return { title: 'Access denied', message: "You don't have permission to do that." };
  if (status === 404)
    return { title: 'Not found', message: 'The requested resource could not be found.' };
  if (status === 429) {
    const source = detail.rateLimitSource;
    const waitHint =
      detail.retryAfterSeconds != null
        ? ` Try again in about ${Math.max(1, Math.ceil(detail.retryAfterSeconds))}s.`
        : '';
    if (source === 'nginx') {
      return { title: 'Too many requests', message: `Edge proxy rate limit reached.${waitHint}` };
    }
    if (source === 'api') {
      return { title: 'Too many requests', message: `API rate limit reached.${waitHint}` };
    }
    return {
      title: 'Too many requests',
      message: `Please wait a moment and try again.${waitHint}`,
    };
  }
  if (status != null && [502, 503, 504].includes(status)) {
    return {
      title: 'Server unavailable',
      message: 'The server is currently unavailable. Please try again in a moment.',
    };
  }
  if (status != null && status >= 500)
    return {
      title: 'Server error',
      message: 'Something went wrong on our end. Please try again later.',
    };
  return {
    title: 'Something went wrong',
    message: truncate(detail.message, 120) || 'An unexpected error occurred.',
  };
}

function isFetchConnectionError(error: unknown): boolean {
  return (
    error instanceof TypeError ||
    (isRecord(error) &&
      typeof error.name === 'string' &&
      ['NetworkError', 'AbortError'].includes(error.name))
  );
}

function apiFailureDetailFrom(error: unknown): ApiFailureDetail | undefined {
  if (!(error instanceof Error) || !('detail' in error)) return undefined;
  const detail = (error as Error & { detail?: ApiFailureDetail }).detail;
  if (typeof detail?.code !== 'string' || typeof detail.message !== 'string') return undefined;
  return detail;
}

function truncate(value: string, maxLength: number) {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, maxLength - 1)}…`;
}

interface ApiRequest {
  method: string;
  path: string;
  body?: unknown;
  params?: Record<string, string | number> | undefined;
  retryOnUnauthorized?: boolean | undefined;
  reportErrors?: boolean | undefined;
  extraHeaders?: Record<string, string> | undefined;
  signal?: AbortSignal | undefined;
}

interface NormalizedRequest extends Omit<ApiRequest, 'body' | 'params'> {
  url: string;
  body?: string | undefined;
  retryOnUnauthorized: boolean;
  reportErrors: boolean;
}

function normalizeRequest(request: ApiRequest): NormalizedRequest {
  const query =
    request.params === undefined
      ? ''
      : `?${new URLSearchParams(
          Object.entries(request.params).map(([key, value]) => [key, String(value)])
        )}`;
  return {
    ...request,
    url: `${getBase()}${request.path}${query}`,
    body: request.body === undefined ? undefined : JSON.stringify(request.body),
    extraHeaders: request.extraHeaders ? { ...request.extraHeaders } : undefined,
    retryOnUnauthorized: request.retryOnUnauthorized !== false && !isPublicAuthPath(request.path),
    reportErrors: request.reportErrors !== false,
  };
}

function apiFailure(detail: ApiFailureDetail): Error & { code: string; detail: ApiFailureDetail } {
  return Object.assign(new Error(formatApiError(detail)), {
    ...(detail.status !== undefined ? { status: detail.status } : {}),
    code: detail.code,
    detail,
  });
}

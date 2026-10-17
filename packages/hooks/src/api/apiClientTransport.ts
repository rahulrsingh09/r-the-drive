// Session-aware execution stages for the Panda REST API.

import type { AuthTokens, RestoreJob, UploadBackupResponse } from '@panda/types';
import { isRecord } from './normalization';
import { restoreJobStatus, uploadBackup } from './fileTransfers';
import { withSessionLock } from './sessionLock';
import { configuredTarget } from '../urlPolicy';
import { isProtectedArtworkPath } from '../outboundPolicy';
import { RequestCooldowns, apiFailureDetailFrom, classifyClientRequestSource, formatApiError, friendlyApiError, inferClientRateLimitClass, isPublicAuthPath, parsePositiveNumberHeader, type ApiFailureDetail } from './requestPolicy';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

export type * from './apiTypes';

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
  const target = configuredBaseUrl && location ? configuredTarget(configuredBaseUrl, location.origin) : null;
  return target?.loopback ? '' : configuredBaseUrl;
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

type AuthChangeHandler = (tokens: AuthTokens | null) => void;

export class RequestTransport {
  private accessToken: string | null = null;
  private authChangeHandler: AuthChangeHandler | null = null;
  private refreshPromise: Promise<AuthTokens | null> | null = null;
  private sessionGeneration = 0;
  private authExpiredReported = false;
  private rateLimitCooldowns = new RequestCooldowns();

  setToken(token: string | null) {
    // Explicit clearing also invalidates a bootstrap that began without a token.
    // Publishing a renewal below is still part of the same session generation.
    if (token === null || token !== this.accessToken) {
      this.sessionGeneration++;
      this.refreshPromise = null;
    }
    this.accessToken = token;
    if (token) this.authExpiredReported = false;
  }

  onAuthChange(handler: AuthChangeHandler) {
    this.authChangeHandler = handler;
  }

  private applyTokens(tokens: AuthTokens) {
    this.accessToken = tokens.access_token;
    this.authExpiredReported = false;
    this.authChangeHandler?.(tokens);
  }

  private clearTokens() {
    // Shared waiters for this rejected renewal retain their AUTH_EXPIRED result.
    // Explicit logout/account replacement goes through setToken and changes generation.
    this.accessToken = null;
    this.authChangeHandler?.(null);
  }

  protected renewSession(bootstrap = false): Promise<AuthTokens | null> {
    if (!this.refreshPromise) {
      const generation = this.sessionGeneration;
      const pending = withSessionLock(async () => {
        this.assertSessionGeneration(generation);
        const abort = new AbortController();
        const timeout = setTimeout(() => abort.abort(), 30_000);
        try {
          const request = {
            method: 'POST',
            path: bootstrap ? '/v1/auth/bootstrap' : '/v1/auth/refresh',
            retryOnUnauthorized: false,
            reportErrors: false,
            signal: abort.signal,
            sessionBound: true,
          };
          const res = await this.send(request);
          this.assertSessionGeneration(generation);
          if (res.status === 204) return null;
          const tokens = await this.decodeJson<AuthTokens>(res, request, generation);
          this.assertSessionGeneration(generation);
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
        } catch (cause) {
          this.assertSessionGeneration(generation);
          throw cause;
        } finally {
          clearTimeout(timeout);
        }
      }).finally(() => {
        if (this.refreshPromise === pending) this.refreshPromise = null;
      });
      this.refreshPromise = pending;
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
    return Object.assign({ 'Content-Type': 'application/json' }, !isPublicAuthPath(path) && this.accessToken ? { Authorization: `Bearer ${this.accessToken}` } : {});
  }

  /**
   * Fetch a first-party browser proxy without ever copying credentials to an
   * upstream URL. MapLibre and Iconify own their transport, so they cannot use
   * the normal typed request method directly.
   */
  proxyFetch(input: RequestInfo | URL, init: RequestInit = {}): Promise<Response> {
    const origin = typeof window === 'undefined' ? 'http://localhost' : window.location.origin;
    const raw = input instanceof URL ? input.href : typeof input === 'string' ? input : input.url;
    const target = configuredTarget(raw, origin).url;
    const proxy = target?.origin === origin && /^\/v1\/external\/(basemap|iconify)(?:\/|$)/.test(target.pathname);
    let options = init;
    if (proxy && this.accessToken) {
      const headers = new Headers(init.headers);
      headers.set('Authorization', `Bearer ${this.accessToken}`);
      options = { ...init, credentials: 'same-origin', headers };
    }
    const pending = fetch(input, options);
    if (!proxy) return pending;
    const report = (error: unknown, response?: Response) => reportClientError(error, {
      event: response ? 'proxy.response_failed' : 'proxy.request_failed', area: 'api', operation: 'first-party-proxy', method: init.method ?? 'GET', path: target?.pathname,
      ...(response ? { status: response.status, code: `HTTP_${response.status}`, requestId: response.headers.get('x-request-id') ?? undefined } : {}), severity: 'warn',
    });
    return pending.then((response) => {
      if (!response.ok) report(new Error(`First-party proxy returned HTTP ${response.status}`), response);
      return response;
    }).catch((error: unknown) => { report(error); throw error; });
  }

  /**
   * Retrieve a first-party protected asset with the normal session bearer
   * token. This deliberately keeps credentials out of browser image URLs.
   */
  async authenticatedAsset(path: string): Promise<Response> {
    if (!isProtectedArtworkPath(path)) {
      throw new Error('authenticatedAsset only accepts first-party vehicle artwork paths');
    }
    return this.requestResponse('GET', path, undefined, undefined, true, false);
  }

  private assertRateLimitCooldown(method: string, path: string) {
    const pending = this.rateLimitCooldowns.pending(method, path);
    if (pending) throw apiFailure(pending);
  }

  private rememberRateLimitCooldown(detail: Pick<ApiFailureDetail, 'method' | 'path' | 'rateLimitClass' | 'retryAfterSeconds' | 'rateLimitResetSeconds'>) {
    this.rateLimitCooldowns.remember(detail);
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
    const generation = this.sessionGeneration;
    const response = await this.send(request);
    if (request.response === 'raw') {
      const result = await (request.decode as (response: Response) => T | Promise<T>)(response);
      this.assertRequestSession(request, generation);
      return result;
    }
    const payload = await this.decodeJson<Wire>(response, request, generation);
    this.assertRequestSession(request, generation);
    if (!request.decode) return payload as unknown as T;
    try {
      const result = await (request.decode as (value: Wire) => T | Promise<T>)(payload);
      this.assertRequestSession(request, generation);
      return result;
    } catch (cause) {
      this.assertRequestSession(request, generation);
      throw this.responseFailure(response, request, 'INVALID_RESPONSE', cause);
    }
  }

  private async decodeJson<T>(
    response: Response,
    request: ApiRequest,
    generation = this.sessionGeneration
  ): Promise<T> {
    if (response.status === 204) return undefined as T;
    try {
      const payload = (await response.json()) as T;
      this.assertRequestSession(request, generation);
      return payload;
    } catch (cause) {
      this.assertRequestSession(request, generation);
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
    const generation = this.sessionGeneration;
    const tokenAtStart = this.accessToken;
    const response = await this.fetchAttempt(normalized, generation);
    this.assertRequestSession(request, generation);
    if (response.ok) return response;

    const canRenew =
      response.status === 401 &&
      normalized.retryOnUnauthorized &&
      tokenAtStart &&
      this.accessToken &&
      !AUTH_REFRESH_EXCLUDED_PATHS.has(request.path);
    if (canRenew) {
      // A delayed 401 may belong to a token that another request already renewed.
      if (this.accessToken === tokenAtStart)
        await this.recoverSession(response, request, generation);
      this.assertRateLimitCooldown(request.method, request.path);
      const retry = await this.fetchAttempt(normalized, generation);
      this.assertRequestSession(request, generation);
      if (retry.ok) return retry;
      throw await this.httpFailure(retry, request, generation);
    }
    throw await this.httpFailure(response, request, generation);
  }

  private async fetchAttempt(request: NormalizedRequest, generation: number): Promise<Response> {
    const { method, path, reportErrors, signal } = request;
    try {
      this.assertRequestSession(request, generation);
      signal?.throwIfAborted();
      return await fetch(request.url, {
        method,
        headers: { ...this.headers(path), ...request.extraHeaders },
        credentials: 'include',
        ...(signal ? { signal } : {}),
        ...(request.body !== undefined ? { body: request.body } : {}),
      });
    } catch (cause) {
      this.assertRequestSession(request, generation);
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

  private async recoverSession(
    response: Response,
    request: ApiRequest,
    generation: number
  ): Promise<void> {
    try {
      await this.refreshAccessToken();
    } catch (cause) {
      this.assertSessionGeneration(generation);
      const failure = apiFailureDetailFrom(cause);
      const code =
        failure?.code ?? (cause instanceof Error && 'code' in cause ? cause.code : undefined);
      if (
        code === 'NETWORK_ERROR' ||
        code === 'INCOMPLETE_RESPONSE' ||
        code === 'INVALID_RESPONSE' ||
        failure?.status === 429 ||
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

  private async httpFailure(
    response: Response,
    request: ApiRequest,
    generation: number
  ): Promise<Error> {
    const payload: unknown = await response.json().catch(() => null);
    this.assertRequestSession(request, generation);
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

  private assertSessionGeneration(generation: number) {
    if (generation !== this.sessionGeneration) {
      throw Object.assign(new Error('The session changed before the request completed.'), {
        code: 'SESSION_CHANGED',
      });
    }
  }

  private assertRequestSession(request: ApiRequest, generation: number) {
    if (request.sessionBound || !isPublicAuthPath(request.path)) {
      this.assertSessionGeneration(generation);
    }
  }

  async apiFetch<T>(method: string, path: string, body?: unknown): Promise<T> {
    return this.execute<T>({ method, path, body });
  }

  protected async uploadFile(file: File, onProgress?: (loaded: number, total: number) => void): Promise<UploadBackupResponse> {
    return uploadBackup(getBase(), this.accessToken, file, onProgress);
  }

  protected async restoreStatus(jobId: string, capabilityToken: string): Promise<RestoreJob> {
    return restoreJobStatus(getBase(), jobId, capabilityToken);
  }

  private reportFailure(detail: ApiFailureDetail, error: Error = new Error(detail.message)) {
    const expired = detail.code === 'AUTH_EXPIRED';
    const silentRefresh = detail.path === '/v1/auth/refresh' && detail.status === 401;
    if (silentRefresh || expired && this.authExpiredReported) return;
    this.authExpiredReported ||= expired;
    reportClientError(error, {
      event: 'api.request_failed', area: 'api', operation: classifyClientRequestSource(detail.path),
      method: detail.method, path: detail.path, status: detail.status, code: detail.code, requestId: detail.requestId, severity: 'warn',
    });
    if (typeof window === 'undefined' || window.location.pathname === '/login' && !isPublicAuthPath(detail.path)) return;
    const events: Array<[string, unknown]> = [
      ['panda:toast', { ...friendlyApiError(detail), variant: 'error', code: detail.code }],
    ];
    if (expired) events.push(['panda:auth-expired', detail]);
    events.forEach(([type, payload]) => window.dispatchEvent(new CustomEvent(type, { detail: payload })));
  }
}

function parseRateLimitHeaders(headers: Headers, method: string, path: string) {
  const numericHeaders = {
    rateLimitLimit: ['x-ratelimit-limit'], rateLimitRemaining: ['x-ratelimit-remaining'],
    rateLimitResetSeconds: ['x-panda-ratelimit-reset', 'x-ratelimit-after'],
  };
  const parsed: { rateLimitLimit?: number; rateLimitRemaining?: number; rateLimitResetSeconds?: number; retryAfterSeconds?: number } = {};
  for (const [field, names] of Object.entries(numericHeaders)) {
    const raw = names.map((name) => headers.get(name)).find((value) => value !== null) ?? null;
    const value = parsePositiveNumberHeader(raw);
    if (value !== undefined) Object.assign(parsed, { [field]: value });
  }
  const retry = parsePositiveNumberHeader(headers.get('retry-after')) ?? parsed.rateLimitResetSeconds;
  if (retry !== undefined) parsed.retryAfterSeconds = retry;
  const source = headers.get('x-panda-ratelimit-source');
  const limiter = headers.get('x-panda-ratelimit-class') ?? inferClientRateLimitClass(method, path);
  return { ...(source ? { rateLimitSource: source } : {}), ...(limiter ? { rateLimitClass: limiter } : {}), ...parsed };
}

function isFetchConnectionError(error: unknown): boolean {
  return (
    error instanceof TypeError ||
    (isRecord(error) &&
      typeof error.name === 'string' &&
      ['NetworkError', 'AbortError'].includes(error.name))
  );
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
  sessionBound?: boolean;
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

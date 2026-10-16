import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AuthenticatedTransport,
  setApiBaseUrl,
} from '../../../../packages/hooks/src/api/transport';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

vi.mock('@panda/ui/lib/clientDiagnostics', () => ({ reportClientError: vi.fn() }));

const tokens = { access_token: 'synthetic-renewed', expires_in: 900, default_vehicle_id: null };
const json = (value: unknown, status = 200, headers?: HeadersInit) =>
  new Response(JSON.stringify(value), { status, ...(headers ? { headers } : {}) });
const denied = () => json({ error: { code: 'UNAUTHORIZED', message: 'Expired' } }, 401);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

let client: AuthenticatedTransport;
beforeEach(() => {
  client = new AuthenticatedTransport();
  client.setToken('synthetic-old');
  setApiBaseUrl('');
  vi.clearAllMocks();
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  setApiBaseUrl(undefined);
});

describe('normalized request execution', () => {
  it('encodes query values and JSON exactly once, retaining extra headers and cookies', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(json({ ok: true }));
    await client.requestResponse(
      'PATCH',
      '/v2/themes/a',
      { value: null },
      { text: 'a/b & c', zero: 0 },
      true,
      true,
      { 'If-Match': '"v1"' }
    );
    expect(fetchMock).toHaveBeenCalledWith('/v2/themes/a?text=a%2Fb+%26+c&zero=0', {
      method: 'PATCH',
      credentials: 'include',
      body: '{"value":null}',
      headers: {
        'Content-Type': 'application/json',
        Authorization: 'Bearer synthetic-old',
        'If-Match': '"v1"',
      },
    });
  });

  it('retains absent-body, null-body, and empty-query distinctions', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => json({}));
    await client.requestResponse('POST', '/v1/example');
    await client.requestResponse('POST', '/v1/example', null, {});
    expect(fetchMock.mock.calls[0]?.[1]).not.toHaveProperty('body');
    expect(fetchMock.mock.calls[1]).toMatchObject(['/v1/example?', { body: 'null' }]);
  });

  it('returns 204 as undefined for generic JSON and null for live-session', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(
      async () => new Response(null, { status: 204 })
    );
    expect(await client.apiFetch('DELETE', '/v1/example')).toBeUndefined();
    expect(await client.getLiveSession('vehicle')).toBeNull();
  });

  it('keeps requestResponse bodies unread for consumers such as themeClient', async () => {
    const response = json({ version: 2 }, 200, { etag: '"two"' });
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(response);
    const result = await client.requestResponse('GET', '/v2/themes/example');
    expect(result).toBe(response);
    expect(result.bodyUsed).toBe(false);
    expect(result.headers.get('etag')).toBe('"two"');
  });

  it('does not fetch when request serialization fails', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch');
    const body: { self?: unknown } = {};
    body.self = body;
    await expect(client.apiFetch('POST', '/v1/example', body)).rejects.toBeInstanceOf(TypeError);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('replays the original serialized request if caller data changes during renewal', async () => {
    const renewal = deferred<Response>();
    const started = deferred<void>();
    const body = { value: 'original' };
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(denied())
      .mockImplementationOnce(() => {
        started.resolve();
        return renewal.promise;
      })
      .mockResolvedValueOnce(json({ ok: true }));
    const request = client.apiFetch('PUT', '/v1/example', body);
    await started.promise;
    body.value = 'changed';
    renewal.resolve(json(tokens));
    await request;
    expect(fetchMock.mock.calls[0]?.[1]?.body).toBe('{"value":"original"}');
    expect(fetchMock.mock.calls[2]?.[1]?.body).toBe('{"value":"original"}');
    expect(fetchMock.mock.calls[2]?.[1]?.headers).toMatchObject({
      Authorization: 'Bearer synthetic-renewed',
    });
  });
});

describe('session coordination and bounded retries', () => {
  it('coalesces simultaneous unauthorized requests with manual renewal and notifies auth once', async () => {
    const renewal = deferred<Response>();
    const started = deferred<void>();
    const changed = vi.fn();
    client.onAuthChange(changed);
    let refreshes = 0;
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(async (url, init) => {
      if (String(url).endsWith('/auth/refresh')) {
        refreshes++;
        started.resolve();
        return renewal.promise;
      }
      return new Headers(init?.headers).get('Authorization') === 'Bearer synthetic-old'
        ? denied()
        : json({ ok: true });
    });
    const a = client.vehicleStatus('a');
    const b = client.vehicleStatus('b');
    await started.promise;
    const manual = client.refresh();
    renewal.resolve(json(tokens));
    await Promise.all([a, b, manual]);
    expect(refreshes).toBe(1);
    expect(fetchMock).toHaveBeenCalledTimes(5);
    expect(changed).toHaveBeenCalledExactlyOnceWith(tokens);
  });

  it('reuses a token already renewed before a delayed 401 arrives', async () => {
    const late = deferred<Response>();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementationOnce(() => late.promise)
      .mockResolvedValueOnce(denied())
      .mockResolvedValueOnce(json(tokens))
      .mockResolvedValueOnce(json({ ok: true }))
      .mockResolvedValueOnce(json({ ok: true }));
    const slow = client.vehicleStatus('slow');
    await client.vehicleStatus('fast');
    late.resolve(denied());
    await slow;
    expect(
      fetchMock.mock.calls.filter(([url]) => String(url).endsWith('/auth/refresh'))
    ).toHaveLength(1);
  });

  it('does not renew or replay a request after its session was cleared', async () => {
    const pending = deferred<Response>();
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(() => pending.promise);
    const result = client.vehicleStatus('a');
    client.setToken(null);
    pending.resolve(denied());
    await expect(result).rejects.toMatchObject({ code: 'SESSION_CHANGED' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('allows only one renewal and one replay when the replay is also unauthorized', async () => {
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(denied())
      .mockResolvedValueOnce(json(tokens))
      .mockResolvedValueOnce(denied());
    await expect(client.vehicleStatus('a')).rejects.toMatchObject({
      status: 401,
      code: 'UNAUTHORIZED',
    });
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it.each([false, true])(
    'does not renew without permission/token (retry disabled: %s)',
    async (disabled) => {
      if (!disabled) client.setToken(null);
      const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(denied());
      await expect(
        client.requestResponse('GET', '/v1/vehicles', undefined, undefined, !disabled)
      ).rejects.toMatchObject({ status: 401 });
      expect(fetchMock).toHaveBeenCalledTimes(1);
    }
  );

  it.each([
    '/v1/auth/login',
    '/v1/auth/config',
    '/v1/auth/oidc/start',
    '/v1/auth/register',
    '/v1/auth/setup',
    '/v1/auth/account-invitations/preview',
    '/v1/auth/account-invitations/accept',
    '/v1/auth/account-invitations/oidc/start',
    '/v1/auth/bootstrap',
    '/v1/auth/refresh',
  ])('omits bearer and renewal for %s', async (path) => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(denied());
    await expect(
      client.requestResponse('POST', path, undefined, undefined, true, false)
    ).rejects.toMatchObject({ status: 401 });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ credentials: 'include' });
    expect(new Headers(fetchMock.mock.calls[0]?.[1]?.headers).has('Authorization')).toBe(false);
  });

  it('uses the unchanged cross-instance session lock', async () => {
    const first = deferred<Response>();
    const started = deferred<void>();
    const other = new AuthenticatedTransport();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementationOnce(() => {
        started.resolve();
        return first.promise;
      })
      .mockResolvedValueOnce(json(tokens));
    const a = client.refresh();
    const b = other.resumeSession();
    await started.promise;
    expect(fetchMock).toHaveBeenCalledTimes(1);
    first.resolve(json(tokens));
    await Promise.all([a, b]);
    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      '/v1/auth/refresh',
      '/v1/auth/bootstrap',
    ]);
  });

  it('uses navigator.locks with the existing name and exclusive mode', async () => {
    const request = vi.fn(async (_name, _options, operation: () => Promise<unknown>) =>
      operation()
    );
    vi.stubGlobal('navigator', { locks: { request } });
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(tokens));
    await client.refresh();
    expect(request).toHaveBeenCalledWith(
      'panda-session-renewal',
      { mode: 'exclusive' },
      expect.any(Function)
    );
  });

  it('returns no session on bootstrap 204 without changing auth and rejects refresh 204', async () => {
    const changed = vi.fn();
    client.onAuthChange(changed);
    vi.spyOn(globalThis, 'fetch').mockImplementation(
      async () => new Response(null, { status: 204 })
    );
    expect(await client.resumeSession()).toBeNull();
    await expect(client.refresh()).rejects.toMatchObject({ code: 'AUTH_EXPIRED', status: 401 });
    expect(changed).not.toHaveBeenCalled();
  });

  it.each(['network', 'server', 'json', 'shape'] as const)(
    'preserves the session when renewal fails temporarily: %s',
    async (kind) => {
      const changed = vi.fn();
      client.onAuthChange(changed);
      const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(denied());
      if (kind === 'network') fetchMock.mockRejectedValueOnce(new TypeError('Failed to fetch'));
      if (kind === 'server')
        fetchMock.mockResolvedValueOnce(
          json({ error: { code: 'UNAVAILABLE', message: 'Offline' } }, 503)
        );
      if (kind === 'json') fetchMock.mockResolvedValueOnce(new Response('{"access_token":'));
      if (kind === 'shape') fetchMock.mockResolvedValueOnce(json({ access_token: 42 }));
      const code = {
        network: 'NETWORK_ERROR',
        server: 'UNAVAILABLE',
        json: 'INCOMPLETE_RESPONSE',
        shape: 'INVALID_RESPONSE',
      }[kind];
      await expect(client.vehicleStatus('a')).rejects.toMatchObject({ code });
      expect(changed).not.toHaveBeenCalled();
      fetchMock.mockResolvedValueOnce(json({ ok: true }));
      await client.vehicleStatus('b');
      expect(fetchMock.mock.calls[2]?.[1]?.headers).toMatchObject({
        Authorization: 'Bearer synthetic-old',
      });
    }
  );

  it('preserves the session and cooldown metadata when automatic renewal is throttled', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    const changed = vi.fn();
    const expired = vi.fn();
    client.onAuthChange(changed);
    window.addEventListener('panda:auth-expired', expired);
    try {
      const fetchMock = vi
        .spyOn(globalThis, 'fetch')
        .mockResolvedValueOnce(denied())
        .mockResolvedValueOnce(
          json({ error: { code: 'RATE_LIMITED', message: 'Wait' } }, 429, {
            'retry-after': '2',
            'x-panda-ratelimit-class': 'auth_public',
            'x-panda-ratelimit-source': 'api',
          })
        )
        .mockResolvedValueOnce(denied())
        .mockResolvedValueOnce(json({ id: 'same-account' }))
        .mockResolvedValueOnce(json(tokens));
      await expect(client.vehicleStatus('a')).rejects.toMatchObject({
        status: 429,
        code: 'RATE_LIMITED',
        detail: {
          path: '/v1/auth/refresh',
          rateLimitSource: 'api',
          rateLimitClass: 'auth_public',
          retryAfterSeconds: 2,
        },
      });
      await expect(client.vehicleStatus('b')).rejects.toMatchObject({
        status: 429,
        code: 'RATE_LIMITED',
        detail: { rateLimitSource: 'client-cooldown', retryAfterSeconds: 2 },
      });
      expect(fetchMock).toHaveBeenCalledTimes(3);
      expect(changed).not.toHaveBeenCalled();
      expect(expired).not.toHaveBeenCalled();
      await client.me();
      expect(new Headers(fetchMock.mock.calls[3]?.[1]?.headers).get('Authorization')).toBe(
        'Bearer synthetic-old'
      );
      vi.advanceTimersByTime(2000);
      expect(await client.refresh()).toEqual(tokens);
      expect(fetchMock).toHaveBeenCalledTimes(5);
      expect(changed).toHaveBeenCalledExactlyOnceWith(tokens);
    } finally {
      window.removeEventListener('panda:auth-expired', expired);
    }
  });

  it.each([401, 403])('clears renewal %s and deduplicates auth-expired events', async (status) => {
    const changed = vi.fn();
    const events = vi.fn();
    client.onAuthChange(changed);
    window.addEventListener('panda:auth-expired', events);
    try {
      vi.spyOn(globalThis, 'fetch').mockImplementation(async (url) =>
        String(url).endsWith('/auth/refresh')
          ? json({ error: { code: 'UNAUTHORIZED', message: 'Rejected session' } }, status)
          : denied()
      );
      const results = await Promise.allSettled([
        client.vehicleStatus('a'),
        client.vehicleStatus('b'),
      ]);
      expect(
        results.every(
          (result) => result.status === 'rejected' && result.reason.code === 'AUTH_EXPIRED'
        )
      ).toBe(true);
      expect(changed).toHaveBeenCalledWith(null);
      expect(events).toHaveBeenCalledTimes(1);
    } finally {
      window.removeEventListener('panda:auth-expired', events);
    }
  });
});

describe('errors, backoff, and cancellation', () => {
  it.each([
    null,
    [],
    'invalid',
    { error: null },
    { error: 'invalid' },
    { error: { code: 3, message: {} } },
  ])('decodes malformed HTTP error envelopes without losing status: %j', async (payload) => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      json(payload, 403, { 'x-request-id': 'synthetic-id' })
    );
    await expect(client.vehicleStatus('a')).rejects.toMatchObject({
      status: 403,
      code: 'unknown',
      detail: { requestId: 'synthetic-id', path: '/v1/vehicles/a/status' },
    });
  });

  it('retains JSON and HTML error fallbacks and reportErrors suppression', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('<html>Unavailable</html>', { status: 502, statusText: 'Bad Gateway' })
    );
    await expect(
      client.requestResponse('GET', '/v1/example', undefined, undefined, true, false)
    ).rejects.toMatchObject({ status: 502, code: 'unknown', detail: { message: 'Bad Gateway' } });
    expect(reportClientError).not.toHaveBeenCalled();
  });

  it('reports interrupted JSON with request metadata but no request secrets', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('{', { headers: { 'x-request-id': 'decode-1' } })
    );
    await expect(
      client.apiFetch('POST', '/v1/example', { password: 'synthetic-secret' })
    ).rejects.toMatchObject({ code: 'INCOMPLETE_RESPONSE', detail: { requestId: 'decode-1' } });
    expect(JSON.stringify(vi.mocked(reportClientError).mock.calls)).not.toContain(
      'synthetic-secret'
    );
    expect(JSON.stringify(vi.mocked(reportClientError).mock.calls)).not.toContain('synthetic-old');
  });

  it('keeps body-read cancellation distinct from a fetch connection failure', async () => {
    const cause = new DOMException('Body read cancelled', 'AbortError');
    const response = json({});
    vi.spyOn(response, 'json').mockRejectedValue(cause);
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(response);
    await expect(client.vehicleStatus('a')).rejects.toMatchObject({
      code: 'INCOMPLETE_RESPONSE',
      cause,
    });
  });

  it('applies class-specific cooldowns and resumes at the deadline without an automatic retry', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(
        json({ error: { code: 'RATE_LIMITED', message: 'Wait' } }, 429, {
          'x-panda-ratelimit-source': 'api',
          'x-panda-ratelimit-class': 'auth_read',
          'x-ratelimit-limit': '10',
          'x-ratelimit-remaining': '0',
          'retry-after': '2',
        })
      )
      .mockImplementation(async () => json({ ok: true }));
    await expect(client.vehicleStatus('a')).rejects.toMatchObject({
      detail: { rateLimitLimit: 10, rateLimitRemaining: 0, retryAfterSeconds: 2 },
    });
    await expect(client.vehicleStatus('b')).rejects.toMatchObject({
      detail: { rateLimitSource: 'client-cooldown', retryAfterSeconds: 2 },
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await client.me();
    await client.updateVehicleName('a', 'Name');
    vi.advanceTimersByTime(2000);
    await client.vehicleStatus('a');
    expect(fetchMock).toHaveBeenCalledTimes(4);
  });

  it.each([
    [{ 'x-panda-ratelimit-reset': '3' }, 3],
    [{ 'x-ratelimit-after': '4' }, 4],
    [{ 'retry-after': 'invalid', 'x-panda-ratelimit-reset': '-1' }, 5],
  ])('uses reset headers/default for cooldown: %j', async (headers, seconds) => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementation(async () => json({}, 429, headers as HeadersInit));
    await expect(client.vehicleStatus('a')).rejects.toMatchObject({ status: 429 });
    await expect(client.vehicleStatus('b')).rejects.toMatchObject({
      detail: { retryAfterSeconds: seconds },
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('preserves AbortError as NETWORK_ERROR with the original cause', async () => {
    const abort = new AbortController();
    const cause = new DOMException('Synthetic abort', 'AbortError');
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => {
      expect(init?.signal).toBe(abort.signal);
      throw cause;
    });
    await expect(
      client.requestResponse(
        'GET',
        '/v1/example',
        undefined,
        undefined,
        true,
        false,
        undefined,
        abort.signal
      )
    ).rejects.toMatchObject({ code: 'NETWORK_ERROR', cause });
  });

  it('stops an already aborted request before fetch', async () => {
    const abort = new AbortController();
    abort.abort();
    const fetchMock = vi.spyOn(globalThis, 'fetch');
    await expect(
      client.requestResponse(
        'GET',
        '/v1/example',
        undefined,
        undefined,
        true,
        false,
        undefined,
        abort.signal
      )
    ).rejects.toMatchObject({ code: 'NETWORK_ERROR', cause: abort.signal.reason });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('cancels one renewal waiter without cancelling shared renewal or another request', async () => {
    const abort = new AbortController();
    const renewal = deferred<Response>();
    const started = deferred<void>();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(denied())
      .mockResolvedValueOnce(denied())
      .mockImplementationOnce(() => {
        started.resolve();
        return renewal.promise;
      })
      .mockResolvedValueOnce(json({ ok: true }));
    const a = client.requestResponse(
      'GET',
      '/v1/a',
      undefined,
      undefined,
      true,
      false,
      undefined,
      abort.signal
    );
    const aResult = expect(a).rejects.toMatchObject({ code: 'NETWORK_ERROR' });
    const b = client.vehicleStatus('b');
    await started.promise;
    abort.abort();
    renewal.resolve(json(tokens));
    await Promise.all([aResult, b]);
    expect(fetchMock).toHaveBeenCalledTimes(4);
  });

  it('bounds renewal network time to 30 seconds and releases the pending renewal', async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementationOnce(
        (_url, init) =>
          new Promise((_resolve, reject) =>
            init?.signal?.addEventListener('abort', () => reject(init.signal?.reason))
          )
      );
    const result = expect(client.refresh()).rejects.toMatchObject({ code: 'NETWORK_ERROR' });
    await vi.advanceTimersByTimeAsync(30_000);
    await result;
    fetchMock.mockResolvedValueOnce(json(tokens));
    expect(await client.refresh()).toEqual(tokens);
    expect(vi.getTimerCount()).toBe(0);
  });
});

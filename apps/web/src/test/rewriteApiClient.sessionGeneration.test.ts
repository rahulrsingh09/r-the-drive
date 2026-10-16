import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AuthenticatedTransport,
  setApiBaseUrl,
} from '../../../../packages/hooks/src/api/transport';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

vi.mock('@panda/ui/lib/clientDiagnostics', () => ({ reportClientError: vi.fn() }));

const tokens = (access_token: string) => ({
  access_token,
  expires_in: 900,
  default_vehicle_id: null,
});
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status });
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
  client.setToken('account-a');
  setApiBaseUrl('');
  vi.clearAllMocks();
});
afterEach(() => {
  vi.restoreAllMocks();
  setApiBaseUrl(undefined);
});

describe('explicit session changes during requests', () => {
  it.each([null, 'account-b'])(
    'discards an in-flight renewal after setToken(%s)',
    async (replacement) => {
      const response = deferred<Response>();
      const started = deferred<void>();
      const changed = vi.fn();
      client.onAuthChange(changed);
      const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementationOnce(() => {
        started.resolve();
        return response.promise;
      });
      const result = expect(client.refresh()).rejects.toMatchObject({
        code: 'SESSION_CHANGED',
      });
      await started.promise;
      client.setToken(replacement);
      response.resolve(json(tokens('renewed-a')));
      await result;

      expect(changed).not.toHaveBeenCalled();
      expect(reportClientError).not.toHaveBeenCalled();
      fetchMock.mockResolvedValueOnce(json({}));
      await client.me();
      expect(new Headers(fetchMock.mock.calls[1]?.[1]?.headers).get('Authorization')).toBe(
        replacement ? `Bearer ${replacement}` : null
      );
    }
  );

  it('invalidates a tokenless bootstrap when the user explicitly clears the session', async () => {
    client.setToken(null);
    const response = deferred<Response>();
    const started = deferred<void>();
    const changed = vi.fn();
    client.onAuthChange(changed);
    vi.spyOn(globalThis, 'fetch').mockImplementation(() => {
      started.resolve();
      return response.promise;
    });
    const result = expect(client.resumeSession()).rejects.toMatchObject({
      code: 'SESSION_CHANGED',
    });
    await started.promise;
    client.setToken(null);
    response.resolve(json(tokens('late-bootstrap')));
    await result;
    expect(changed).not.toHaveBeenCalled();
  });

  it('does not clear a newer account when an old automatic renewal is rejected', async () => {
    const response = deferred<Response>();
    const started = deferred<void>();
    const changed = vi.fn();
    const expired = vi.fn();
    client.onAuthChange(changed);
    window.addEventListener('panda:auth-expired', expired);
    try {
      const fetchMock = vi
        .spyOn(globalThis, 'fetch')
        .mockResolvedValueOnce(denied())
        .mockImplementationOnce(() => {
          started.resolve();
          return response.promise;
        });
      const result = expect(client.me()).rejects.toMatchObject({ code: 'SESSION_CHANGED' });
      await started.promise;
      client.setToken('account-b');
      response.resolve(denied());
      await result;
      expect(changed).not.toHaveBeenCalled();
      expect(expired).not.toHaveBeenCalled();
      expect(reportClientError).not.toHaveBeenCalled();
      fetchMock.mockResolvedValueOnce(json({}));
      await client.me();
      expect(new Headers(fetchMock.mock.calls[2]?.[1]?.headers).get('Authorization')).toBe(
        'Bearer account-b'
      );
    } finally {
      window.removeEventListener('panda:auth-expired', expired);
    }
  });

  it('does not replay an old account write under the replacement account', async () => {
    const response = deferred<Response>();
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockReturnValue(response.promise);
    const result = expect(
      client.apiFetch('PATCH', '/v1/auth/preferences', { theme: 'account-a-choice' })
    ).rejects.toMatchObject({ code: 'SESSION_CHANGED' });
    client.setToken('account-b');
    response.resolve(denied());
    await result;
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(new Headers(fetchMock.mock.calls[0]?.[1]?.headers).get('Authorization')).toBe(
      'Bearer account-a'
    );
  });

  it('discards a successful protected response that arrives after an account change', async () => {
    const response = deferred<Response>();
    vi.spyOn(globalThis, 'fetch').mockReturnValue(response.promise);
    const result = expect(client.me()).rejects.toMatchObject({ code: 'SESSION_CHANGED' });
    client.setToken('account-b');
    response.resolve(json({ id: 'account-a' }));
    await result;
  });

  it('discards an old protected body that finishes decoding after an account change', async () => {
    const body = deferred<unknown>();
    const reading = deferred<void>();
    const response = json({});
    vi.spyOn(response, 'json').mockImplementation(() => {
      reading.resolve();
      return body.promise;
    });
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(response);
    const result = expect(client.me()).rejects.toMatchObject({ code: 'SESSION_CHANGED' });
    await reading.promise;
    client.setToken('account-b');
    body.resolve({ id: 'account-a' });
    await result;
    expect(reportClientError).not.toHaveBeenCalled();
  });

  it('skips an invalidated renewal waiting for the cross-instance lock', async () => {
    const response = deferred<Response>();
    const started = deferred<void>();
    const other = new AuthenticatedTransport();
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(() => {
      started.resolve();
      return response.promise;
    });
    const occupying = other.refresh();
    await started.promise;
    const result = expect(client.refresh()).rejects.toMatchObject({
      code: 'SESSION_CHANGED',
    });
    client.setToken(null);
    response.resolve(json(tokens('other-client')));
    await Promise.all([occupying, result]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it('keeps the newer shared renewal registered when the old renewal finishes', async () => {
    const oldResponse = deferred<Response>();
    const oldStarted = deferred<void>();
    const newResponse = deferred<Response>();
    const newStarted = deferred<void>();
    const changed = vi.fn();
    client.onAuthChange(changed);
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementationOnce(() => {
        oldStarted.resolve();
        return oldResponse.promise;
      })
      .mockImplementationOnce(() => {
        newStarted.resolve();
        return newResponse.promise;
      });
    const oldResult = expect(client.refresh()).rejects.toMatchObject({
      code: 'SESSION_CHANGED',
    });
    await oldStarted.promise;
    client.setToken('account-b');
    const newResult = client.refresh();
    oldResponse.resolve(json(tokens('renewed-a')));
    await oldResult;
    await newStarted.promise;
    const sharedResult = client.refresh();
    newResponse.resolve(json(tokens('renewed-b')));
    expect(await Promise.all([newResult, sharedResult])).toEqual([
      tokens('renewed-b'),
      tokens('renewed-b'),
    ]);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(changed).toHaveBeenCalledExactlyOnceWith(tokens('renewed-b'));
  });

  it('does not apply an old renewal throttle to the replacement session', async () => {
    const response = deferred<Response>();
    const started = deferred<void>();
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementationOnce(() => {
      started.resolve();
      return response.promise;
    });
    const oldResult = expect(client.refresh()).rejects.toMatchObject({
      code: 'SESSION_CHANGED',
    });
    await started.promise;
    client.setToken('account-b');
    response.resolve(new Response('{}', { status: 429, headers: { 'retry-after': '60' } }));
    await oldResult;
    fetchMock.mockResolvedValueOnce(json(tokens('renewed-b')));
    expect(await client.refresh()).toEqual(tokens('renewed-b'));
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('keeps in-flight work valid when auth consumers publish the same renewed token', async () => {
    const response = deferred<Response>();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(json(tokens('renewed-a')))
      .mockReturnValueOnce(response.promise);
    const renewed = await client.resumeSession();
    const request = client.me();
    client.setToken(renewed!.access_token);
    response.resolve(json({ id: 'account-a' }));
    expect(await request).toEqual({ id: 'account-a' });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});

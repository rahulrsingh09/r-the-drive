import { afterEach, describe, expect, it, vi } from 'vitest';
import { installGlobalClientErrorHandlers, reportClientError, isAbortError } from '@panda/ui/lib/clientDiagnostics';
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); });

describe('diagnostic privacy and lifecycle', () => {
  it.each(['/v1/auth/login', '/v1/vehicles/connect'])('does not expose credential failures for %s', path => {
    const output = vi.spyOn(console, 'warn').mockImplementation(() => {});
    reportClientError(new Error('password=hunter-synthetic refresh_token=refresh-synthetic account=user@example.test'), {
      event: 'api.request_failed', area: 'api', operation: 'request', path, status: 401,
      requestId: 'session-sensitive-id', sourceId: 'provider-sensitive-id', code: 'AUTH_FAILED',
    });
    expect(output).toHaveBeenCalledTimes(1);
    const written = JSON.stringify(output.mock.calls);
    for (const secret of ['hunter-synthetic', 'refresh-synthetic', 'user@example.test', 'session-sensitive-id', 'provider-sensitive-id']) expect(written).not.toContain(secret);
    expect(output.mock.calls[0]?.[1]).toEqual(expect.objectContaining({ path, status: 401, code: 'AUTH_FAILED', error: { name: 'Error', message: 'Authentication request failed' } }));
  });

  it('redacts credentials outside auth paths and in caller context labels', () => {
    const output = vi.spyOn(console, 'error').mockImplementation(() => {});
    reportClientError(new Error('password="synthetic password" token=token-synthetic Bearer bearer-synthetic https://x.example.test/path?key=url-synthetic'), {
      event: 'browser.failure', area: 'browser', operation: 'access_token=context-synthetic',
    });
    const written = JSON.stringify(output.mock.calls);
    for (const secret of ['synthetic password', 'token-synthetic', 'bearer-synthetic', 'url-synthetic', 'context-synthetic']) expect(written).not.toContain(secret);
  });

  it('keeps cancellation silent and expires content dedupe at five seconds', () => {
    vi.useFakeTimers(); vi.setSystemTime(10000);
    const output = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const context = { event: 'dedupe.test', area: 'query', operation: 'fixture' };
    expect(isAbortError(new DOMException('cancel', 'AbortError'))).toBe(true);
    reportClientError({ name: 'AbortError' }, context);
    reportClientError('unique synthetic timeout', context);
    vi.advanceTimersByTime(4999);
    reportClientError('unique synthetic timeout', context);
    expect(output).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    reportClientError('unique synthetic timeout', context);
    expect(output).toHaveBeenCalledTimes(2);
  });

  it('unregisters all global handlers and prevents raw rejection output', () => {
    const output = vi.spyOn(console, 'error').mockImplementation(() => {});
    const release = installGlobalClientErrorHandlers();
    const event = new Event('unhandledrejection', { cancelable: true });
    Object.defineProperty(event, 'reason', { value: new Error('synthetic rejected operation') });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(output).toHaveBeenCalledTimes(1);
    release();
    window.dispatchEvent(new Event('securitypolicyviolation', { cancelable: true }));
    expect(output).toHaveBeenCalledTimes(1);
  });
});

import { describe, expect, it } from 'vitest';
import { shouldRetryQuery } from '../queryClient';

const httpError = (status: number, code = `HTTP_${status}`) =>
  Object.assign(new Error(`HTTP ${status}`), { status, code });

describe('shouldRetryQuery', () => {
  it.each([401, 403, 404, 429])('does not retry HTTP %i', (status) => {
    expect(shouldRetryQuery(0, httpError(status))).toBe(false);
  });

  it('does not retry an expired session', () => {
    expect(shouldRetryQuery(0, Object.assign(new Error('Session expired'), { code: 'AUTH_EXPIRED' }))).toBe(false);
  });

  it.each([500, 502, 503])('retries HTTP %i up to 2 times', (status) => {
    expect(shouldRetryQuery(0, httpError(status))).toBe(true);
    expect(shouldRetryQuery(1, httpError(status))).toBe(true);
    expect(shouldRetryQuery(2, httpError(status))).toBe(false);
  });

  it('retries network errors and errors without a status up to 2 times', () => {
    const network = Object.assign(new Error('offline'), { code: 'NETWORK_ERROR' });
    expect(shouldRetryQuery(0, network)).toBe(true);
    expect(shouldRetryQuery(2, network)).toBe(false);
    expect(shouldRetryQuery(1, new Error('boom'))).toBe(true);
    expect(shouldRetryQuery(1, null)).toBe(true);
    expect(shouldRetryQuery(2, null)).toBe(false);
  });
});

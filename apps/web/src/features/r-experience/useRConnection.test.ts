import { describe, expect, it } from 'vitest';
import { rConnectionBanner } from './useRConnection';

const at = '2026-11-08T02:52:00Z';

describe('app connection banner', () => {
  it('shows offline first, from the browser state', () => {
    expect(rConnectionBanner({ last_updated: at, auth_state: 'needs_reauth' }, false)).toEqual({ kind: 'offline', since: at });
    expect(rConnectionBanner(null, false)).toEqual({ kind: 'offline', since: null });
  });
  it('reports a lost Rivian connection when Rivian asks for a new sign-in', () => {
    expect(rConnectionBanner({ last_updated: at, auth_state: 'needs_reauth' }, true)).toEqual({ kind: 'rivian', since: at, reason: 'credentials' });
    expect(rConnectionBanner({ last_updated: at, renewal_state: 'reauth_required' }, true)?.kind).toBe('rivian');
  });
  it('reports a lost Rivian connection when the collector fails or the Rivian socket goes silent', () => {
    expect(rConnectionBanner({ last_updated: at, worker_health: 'error' }, true)).toMatchObject({ kind: 'rivian', reason: 'feed' });
    expect(rConnectionBanner({ last_updated: at, last_ws_received_at: '2026-11-08T01:00:00Z', telemetry_stale: true, telemetry_stale_reason: 'ws_silent' }, true))
      .toEqual({ kind: 'rivian', since: '2026-11-08T01:00:00Z', reason: 'feed' });
  });
  it('does not alarm when only the battery reading is old (the car sleeps)', () => {
    expect(rConnectionBanner({ last_updated: at, telemetry_stale: true, telemetry_stale_reason: 'battery_stale', worker_health: 'stale' }, true)).toBeNull();
    expect(rConnectionBanner({ last_updated: at, worker_health: 'healthy' }, true)).toBeNull();
  });
});

import { describe, expect, it } from 'vitest';
import { normalizeChargeSession, normalizeTrip } from '../../../../packages/hooks/src/api/normalization';

describe('API normalization keeps additive server fields', () => {
  it('keeps trip odometer and outside temperature fields', () => {
    const trip = normalizeTrip({ id: 't1', vehicle_id: 'v1', started_at: '2026-11-07T08:12:00Z', start_odometer_mi: 12452.1, end_odometer_mi: 12467.3, outside_temp_c: 8, outside_temp_source: 'vehicle' });
    expect(trip.start_odometer_mi).toBe(12452.1);
    expect(trip.end_odometer_mi).toBe(12467.3);
    expect(trip.outside_temp_c).toBe(8);
  });
  it('leaves the odometer fields absent for older servers and null for missing readings', () => {
    expect('start_odometer_mi' in normalizeTrip({ id: 't2' })).toBe(false);
    expect(normalizeTrip({ id: 't3', start_odometer_mi: null }).start_odometer_mi).toBeNull();
  });
  it('keeps the derived charge rate', () => {
    const session = normalizeChargeSession({ id: 'c1', vehicle_id: 'v1', started_at: '2026-11-06T16:08:00Z', cost_usd: 14.3, cost_method: 'user_manual', cost_rate_per_kwh: 0.5 });
    expect(session.cost_method).toBe('user_manual');
    expect(session.cost_rate_per_kwh).toBe(0.5);
  });
});

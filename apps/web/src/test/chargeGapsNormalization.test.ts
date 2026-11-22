import { describe, expect, it } from 'vitest';
import { normalizeChargeGaps } from '../../../../packages/hooks/src/useCarDataHealth';

describe('charge-gaps response', () => {
  it('reads the API list form', () => {
    expect(normalizeChargeGaps('veh-1', [
      { start: '2026-11-01T01:00:00Z', end: '2026-11-01T03:00:00Z', battery_from: 41, battery_to: 63.5 },
      { start: '2026-11-02T01:00:00Z', end: null, battery_from: 41, battery_to: 50 },
    ])).toEqual({
      vehicle_id: 'veh-1', lookback_days: 30, parked_periods_checked: null, daily_samples: null,
      gaps: [{ id: '2026-11-01T01:00:00Z', started_at: '2026-11-01T01:00:00Z', ended_at: '2026-11-01T03:00:00Z',
        soc_start: 41, soc_end: 63.5, latitude: null, longitude: null }],
    });
  });

  it('reads the object form with checked periods and daily samples', () => {
    const result = normalizeChargeGaps('veh-1', {
      vehicle_id: 'veh-1', lookback_days: 30, parked_periods_checked: 69, gaps: [],
      daily_samples: [{ day: '2026-11-07', samples: 1234 }, { day: 'bad' }],
    });
    expect(result.parked_periods_checked).toBe(69);
    expect(result.gaps).toEqual([]);
    expect(result.daily_samples).toEqual([{ day: '2026-11-07', samples: 1234 }]);
  });

  it('treats an unknown body as an empty list', () => {
    expect(normalizeChargeGaps('veh-1', {}).gaps).toEqual([]);
  });
});

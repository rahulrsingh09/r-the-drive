import { describe, expect, it } from 'vitest';
import type { VehicleStatus } from '@panda/types';
import { vehicleLockReading } from './vehicleLockReading';

const snapshot = (data: Partial<VehicleStatus>) => data as VehicleStatus;

describe('vehicle lock reading', () => {
  it('uses the reported overall state independently of door positions', () => {
    expect(vehicleLockReading(snapshot({ doors_locked: true, door_front_left_closed: false }))).toBe(true);
    expect(vehicleLockReading(snapshot({ doors_locked: false, door_front_left_closed: true }))).toBe(false);
  });
  it('does not infer an overall state from individual locks', () => {
    expect(vehicleLockReading(snapshot({ door_front_left_locked: true }))).toBeNull();
    expect(vehicleLockReading(snapshot({ door_front_left_locked: false }))).toBeNull();
  });
  it('keeps missing snapshots and readings unknown', () => {
    expect(vehicleLockReading(undefined)).toBeNull();
    expect(vehicleLockReading(null)).toBeNull();
    expect(vehicleLockReading(snapshot({ doors_locked: null }))).toBeNull();
  });
  for (const reason of ['never_seen', 'invalid_sensor'] as const) {
    it(`ignores stale values when ${reason}`, () => {
      expect(vehicleLockReading(snapshot({ doors_locked: true, field_availability: {
        doors_locked: { availability: 'never_seen', ever_seen: false, last_seen_at: null, latest_event_at: null, reason_code: reason },
      } }))).toBeNull();
    });
  }
});

import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatDriveMode, getDriveModeBadgeClass } from '@panda/ui/lib/driveMode';
import { formatMiles, formatDistanceKm, formatMph, formatMassKg, formatPressure, formatAltitude, formatEnergyPerDistance } from '@panda/ui/lib/utils';
import { summarizeStatusAvailability } from '@panda/ui/lib/vehicleStatus';
import type { VehicleStatus } from '@panda/types';

afterEach(() => vi.restoreAllMocks());

describe('foundation input boundaries', () => {
  it.each([['constructor', 'Constructor'], ['__proto__', '  Proto  ']])('formats the unknown drive value %s without prototype lookup', (value, label) => {
    expect(formatDriveMode(value)).toBe(label);
    expect(getDriveModeBadgeClass(value)).toBe('bg-bg-elevated text-fg-secondary border border-border');
  });

  it('does not read or migrate preferences for missing measurements', () => {
    const read = vi.spyOn(Storage.prototype, 'getItem');
    for (const format of [formatMiles, formatDistanceKm, formatMph, formatMassKg, formatPressure, formatAltitude, formatEnergyPerDistance]) {
      expect(format(null)).toBe('-'); expect(format(undefined)).toBe('-'); expect(format(NaN)).toBe('-');
    }
    expect(read).not.toHaveBeenCalled();
  });

  it('keeps incomplete availability metadata Boolean and retains the first event timestamp', () => {
    const vehicle = { field_availability: {
      first: { availability: 'historical', last_seen_at: '2026-01-01T00:00:00Z', latest_event_at: '2026-01-02T00:00:00Z' },
      second: { availability: 'current', last_seen_at: '2026-02-01T00:00:00Z', latest_event_at: '2026-02-02T00:00:00Z' },
    } } as unknown as VehicleStatus;
    expect(summarizeStatusAvailability(vehicle, ['first', 'second'])).toEqual({ availability: 'current', reasonCode: null, lastSeenAt: '2026-02-01T00:00:00Z', latestEventAt: '2026-01-02T00:00:00Z', everSeen: false });
  });
});

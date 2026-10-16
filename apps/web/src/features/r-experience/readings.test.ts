import { describe, expect, it } from 'vitest';
import type { VehicleStatus } from '@panda/types';
import { finiteReading, isConfiguredR2, vehicleReading, vehicleState } from './readings';

describe('R reading contracts', () => {
  it('keeps missing, invalid and unseen readings distinct from zero', () => {
    expect(finiteReading(0)).toBe(0);
    expect(finiteReading(null)).toBeNull();
    expect(finiteReading(NaN)).toBeNull();
    expect(finiteReading(Infinity)).toBeNull();
    const status = { battery_level: 0, tire_fl_psi: 0, field_availability: {
      tire_fl_psi: { availability: 'never_seen' },
    } } as unknown as VehicleStatus;
    expect(vehicleReading(status, 'battery_level')).toBe(0);
    expect(vehicleReading(status, 'tire_fl_psi')).toBeNull();
  });
  it('uses the owner-configured Catalina Cove artwork even when Rivian omits paint metadata', () => {
    expect(isConfiguredR2({ model: 'R2', trim: 'Performance', color: 'Catalina Cove' })).toBe(true);
    expect(isConfiguredR2({ model: 'R2', trim: 'R2 Performance', color: null })).toBe(true);
    expect(isConfiguredR2({ model: 'R2', trim: 'Performance', color: 'Other paint' })).toBe(true);
    expect(isConfiguredR2({ model: 'R1T', trim: 'Performance', color: 'Catalina Cove' })).toBe(false);
    expect(isConfiguredR2({ model: 'R2', trim: 'Standard', color: 'Catalina Cove' })).toBe(false);
  });
  it('does not call unknown or stale vehicle telemetry parked', () => {
    expect(vehicleState(undefined)).toBe('Waiting for telemetry');
    expect(vehicleState({} as VehicleStatus)).toBe('Vehicle snapshot');
    expect(vehicleState({ power_state: 'ready', telemetry_stale: true } as VehicleStatus)).toBe('Last known readings');
    expect(vehicleState({ charger_status: 'chrgr_sts_connected_charging' } as VehicleStatus)).toBe('Charging');
  });
});

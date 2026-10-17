import { afterEach, describe, expect, it, vi } from 'vitest';
import type { VehicleStatus } from '@panda/types';
import { getDefaultBySlug } from '@panda/dashboards';
import { usesBundledLayout } from '../features/r-experience/defaultLayout';
import { finiteReading, isConfiguredR2, vehicleReading, vehicleState } from '../features/r-experience/readings';
import { vehicleLockReading } from '../features/r-experience/vehicleLockReading';
import { readRDataPalette } from '../features/r-experience/dataPalette';

afterEach(() => vi.restoreAllMocks());
const status = (value: object) => value as VehicleStatus;

describe('R reading and layout characterization', () => {
  it.each([undefined, null, true, false, '', '2', NaN, Infinity, -Infinity, {}, []])('rejects nonfinite reading %j', value => {
    expect(finiteReading(value)).toBeNull();
  });
  it.each([0, -0, -12.5, Number.MAX_VALUE])('preserves finite reading %s', value => {
    expect(Object.is(finiteReading(value), value)).toBe(true);
  });
  it('applies availability and tire validity independently', () => {
    for (const field of ['battery_level', 'tire_fl_psi'] as const) {
      for (const availability of [{ availability: 'never_seen' }, { reason_code: 'invalid_sensor' }]) {
        expect(vehicleReading(status({ [field]: 42, field_availability: { [field]: availability } }), field)).toBeNull();
      }
    }
    expect(vehicleReading(status({ tire_fr_psi: 42, tire_fr_valid: false }), 'tire_fr_psi')).toBeNull();
    expect(vehicleReading(status({ tire_fr_psi: 42, tire_fr_valid: null }), 'tire_fr_psi')).toBe(42);
    expect(vehicleLockReading(status({ doors_locked: false, field_availability: { doors_locked: { availability: 'stale' } } }))).toBe(false);
    expect(vehicleLockReading(status({ doors_locked: 'false' }))).toBeNull();
  });
  it.each([
    [undefined, 'Waiting for telemetry'],
    [{ telemetry_stale: true, charger_status: 'chrgr_sts_connected_charging' }, 'Last known readings'],
    [{ power_state: 'drive_sleep', charger_status: 'chrgr_sts_connected_charging' }, 'Charging'],
    [{ power_state: 'GO_SLEEP' }, 'Driving'],
    [{ power_state: 'Sleep' }, 'Asleep'],
    [{ power_state: 'ready' }, 'Parked'],
    [{ power_state: 'READY' }, 'Vehicle snapshot'],
    [{}, 'Vehicle snapshot'],
  ])('keeps vehicle state precedence for %j', (value, label) => {
    expect(vehicleState(value as VehicleStatus | undefined)).toBe(label);
  });
  it('normalizes only ASCII model and trim characters', () => {
    expect(isConfiguredR2({ model: ' R-2! ', trim: 'pre-PERFORMANCE-edition', color: null })).toBe(true);
    expect(isConfiguredR2({ model: 'R２', trim: 'performance', color: null })).toBe(false);
    expect(isConfiguredR2(undefined)).toBe(false);
  });
  it('admits reordered layouts and ignores titles/options, but compares identity and geometry', () => {
    const original = getDefaultBySlug('dashboard')!;
    const changed = structuredClone(original);
    changed.widgets.reverse();
    changed.widgets[0]!.title = 'Synthetic title';
    changed.widgets[0]!.options = {};
    expect(usesBundledLayout(changed)).toBe(true);
    changed.widgets[0]!.definitionId = 'different';
    expect(usesBundledLayout(changed)).toBe(false);
    expect(usesBundledLayout({ ...original, slug: 'not-bundled' })).toBe(false);
    expect(usesBundledLayout(undefined)).toBe(false);
    const repeated = structuredClone(original);
    repeated.widgets = original.widgets.map(() => original.widgets[0]!);
    expect(usesBundledLayout(repeated)).toBe(true);
  });
  it('reads both theme scopes and removes palette probes', () => {
    vi.spyOn(window, 'getComputedStyle').mockImplementation(element => ({
      getPropertyValue: (key: string) => ` ${element.classList.contains('dark') ? 'dark' : 'light'}:${key} `,
    }) as CSSStyleDeclaration);
    const result = readRDataPalette();
    expect(result.palette).toBe('rad');
    expect(result.chartColorPairs?.['series-01']).toEqual({ light: 'light:--rm-series-01', dark: 'dark:--rm-series-01' });
    expect(result.chartColorPairs?.orange).toEqual({ light: 'light:--rm-chart-orange', dark: 'dark:--rm-chart-orange' });
    expect(document.querySelector('.r-palette-probe')).toBeNull();
  });
});

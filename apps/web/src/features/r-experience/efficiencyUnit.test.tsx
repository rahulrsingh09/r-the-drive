import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import {
  EFFICIENCY_UNIT_STORAGE_KEY, formatEfficiencyNumber, formatEfficiencyValue, formatEfficiencyWithUnit,
  measuredEfficiencyWhMi, readEfficiencyUnit, useEfficiencyUnit,
} from './efficiencyUnit';

beforeEach(() => localStorage.clear());

describe('efficiency unit', () => {
  it('converts Wh/mi into each unit with its own digits', () => {
    expect(formatEfficiencyWithUnit(351, 'wh_mi')).toBe('351 Wh/mi');
    expect(formatEfficiencyWithUnit(351, 'mi_kwh')).toBe('2.85 mi/kWh');
    expect(formatEfficiencyWithUnit(351, 'kwh_100mi')).toBe('35.1 kWh/100 mi');
    expect(formatEfficiencyNumber(null, 'wh_mi')).toBe('–');
    expect(formatEfficiencyValue(0, 'mi_kwh')).toBeNull();
  });

  it('defaults to Wh/mi and reads older stored names', () => {
    expect(readEfficiencyUnit()).toBe('wh_mi');
    localStorage.setItem(EFFICIENCY_UNIT_STORAGE_KEY, 'mikwh');
    expect(readEfficiencyUnit()).toBe('mi_kwh');
    localStorage.setItem(EFFICIENCY_UNIT_STORAGE_KEY, 'nonsense');
    expect(readEfficiencyUnit()).toBe('wh_mi');
  });

  it('keeps the choice in local storage and updates every hook', () => {
    const first = renderHook(() => useEfficiencyUnit());
    const second = renderHook(() => useEfficiencyUnit());
    act(() => first.result.current.setUnit('kwh_100mi'));
    expect(localStorage.getItem(EFFICIENCY_UNIT_STORAGE_KEY)).toBe('kwh_100mi');
    expect(second.result.current.unit).toBe('kwh_100mi');
    expect(second.result.current.formatWithUnit(320)).toBe('32.0 kWh/100 mi');
    expect(second.result.current.format(null)).toBeNull();
    expect(second.result.current.lowerIsBetter).toBe(true);
  });

  it('leaves trips under 3 mi out of the average and weights by distance', () => {
    expect(measuredEfficiencyWhMi([
      { distance_mi: 10, efficiency_wh_mi: 300 }, { distance_mi: 30, efficiency_wh_mi: 400 }, { distance_mi: 2, efficiency_wh_mi: 900 },
      { distance_mi: 5, efficiency_wh_mi: null, energy_used_kwh: null },
    ])).toBeCloseTo(375);
    expect(measuredEfficiencyWhMi([{ distance_mi: 1, efficiency_wh_mi: 300 }])).toBeNull();
  });
});

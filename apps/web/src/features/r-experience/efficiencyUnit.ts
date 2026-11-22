import React from 'react';

/**
 * The owner's efficiency unit, shared by Now, Activity, Insights and Car.
 * Values are stored and computed in Wh/mi; this file only converts them for display.
 * The choice is kept in local storage under one key, and every hook instance follows changes
 * (same tab through a window event, other tabs through the storage event).
 */
export type EfficiencyUnit = 'wh_mi' | 'mi_kwh' | 'kwh_100mi';

export const EFFICIENCY_UNIT_STORAGE_KEY = 'r-efficiency-unit';
export const EFFICIENCY_UNIT_EVENT = 'r-efficiency-unit-change';
export const DEFAULT_EFFICIENCY_UNIT: EfficiencyUnit = 'wh_mi';

/** Switch order: Wh/mi · mi/kWh · kWh/100 mi. */
export const EFFICIENCY_UNIT_ORDER: readonly EfficiencyUnit[] = ['wh_mi', 'mi_kwh', 'kwh_100mi'];

export interface EfficiencyUnitSpec {
  label: string;
  /** Fraction digits shown. `digits` is the same value. */
  decimals: number;
  digits: number;
  /** True when a lower number is better (energy per distance). */
  lowerIsBetter: boolean;
  /** Converts Wh/mi into this unit. `fromWhPerMile` is the same function. */
  fromWhMi: (whPerMile: number) => number;
  fromWhPerMile: (whPerMile: number) => number;
}

const spec = (label: string, decimals: number, lowerIsBetter: boolean, from: (whPerMile: number) => number): EfficiencyUnitSpec =>
  ({ label, decimals, digits: decimals, lowerIsBetter, fromWhMi: from, fromWhPerMile: from });

export const EFFICIENCY_UNITS: Record<EfficiencyUnit, EfficiencyUnitSpec> = {
  wh_mi: spec('Wh/mi', 0, true, value => value),
  mi_kwh: spec('mi/kWh', 2, false, value => 1000 / value),
  kwh_100mi: spec('kWh/100 mi', 1, true, value => value / 10),
};

/** Older stored names for the same units. */
const LEGACY_UNITS: Record<string, EfficiencyUnit> = { whmi: 'wh_mi', mikwh: 'mi_kwh', kwh100: 'kwh_100mi' };

/** Trips shorter than this have coarse battery readings and stay out of efficiency averages ("Short trip"). */
export const SHORT_TRIP_MILES = 3;

export function isEfficiencyUnit(value: unknown): value is EfficiencyUnit {
  return value === 'wh_mi' || value === 'mi_kwh' || value === 'kwh_100mi';
}

export function readEfficiencyUnit(): EfficiencyUnit {
  try {
    const stored = window.localStorage.getItem(EFFICIENCY_UNIT_STORAGE_KEY);
    if (isEfficiencyUnit(stored)) return stored;
    return (stored && LEGACY_UNITS[stored]) || DEFAULT_EFFICIENCY_UNIT;
  } catch {
    return DEFAULT_EFFICIENCY_UNIT;
  }
}

export function writeEfficiencyUnit(unit: EfficiencyUnit) {
  try { window.localStorage.setItem(EFFICIENCY_UNIT_STORAGE_KEY, unit); } catch { /* storage can be unavailable */ }
  window.dispatchEvent(new CustomEvent(EFFICIENCY_UNIT_EVENT, { detail: unit }));
}

/** Wh/mi in the unit. Null for a missing, zero or negative reading. */
export function convertEfficiency(whPerMile: number | null | undefined, unit: EfficiencyUnit): number | null {
  if (whPerMile == null || !Number.isFinite(whPerMile) || whPerMile <= 0) return null;
  return EFFICIENCY_UNITS[unit].fromWhMi(whPerMile);
}

/** The number only, or null without a reading. */
export function formatEfficiencyValue(whPerMile: number | null | undefined, unit: EfficiencyUnit): string | null {
  const value = convertEfficiency(whPerMile, unit);
  const { decimals } = EFFICIENCY_UNITS[unit];
  return value == null ? null : value.toLocaleString('en-US', { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
}

/** The number only, for example "351", "2.85" or "35.1". "–" without a reading. */
export function formatEfficiencyNumber(whPerMile: number | null | undefined, unit: EfficiencyUnit): string {
  return formatEfficiencyValue(whPerMile, unit) ?? '–';
}

/** The number and unit, for example "351 Wh/mi". "–" without a reading. */
export function formatEfficiencyWithUnit(whPerMile: number | null | undefined, unit: EfficiencyUnit): string {
  const text = formatEfficiencyNumber(whPerMile, unit);
  return text === '–' ? text : `${text} ${EFFICIENCY_UNITS[unit].label}`;
}

/** Distance-weighted Wh/mi over trips of at least {@link SHORT_TRIP_MILES} with an energy reading. Null without one. */
export function measuredEfficiencyWhMi(trips: ReadonlyArray<{ distance_mi: number; efficiency_wh_mi: number | null; energy_used_kwh?: number | null }>) {
  let miles = 0;
  let wh = 0;
  for (const trip of trips) {
    if (!(trip.distance_mi >= SHORT_TRIP_MILES)) continue;
    const perMile = trip.efficiency_wh_mi ?? (trip.energy_used_kwh != null && trip.energy_used_kwh > 0 ? trip.energy_used_kwh * 1000 / trip.distance_mi : null);
    if (perMile == null || !Number.isFinite(perMile) || perMile <= 0) continue;
    miles += trip.distance_mi;
    wh += perMile * trip.distance_mi;
  }
  return miles > 0 ? wh / miles : null;
}

/**
 * The owner's efficiency unit and helpers bound to it:
 * - `format(whPerMile)`: the number only, or null without a reading ("351");
 * - `formatValue(whPerMile)`: the number only, or "–" without a reading;
 * - `formatWithUnit(whPerMile)`: the number and unit, or "–" ("351 Wh/mi").
 */
export function useEfficiencyUnit() {
  const [unit, setUnitState] = React.useState<EfficiencyUnit>(() => readEfficiencyUnit());
  React.useEffect(() => {
    const sync = () => setUnitState(readEfficiencyUnit());
    const onStorage = (event: StorageEvent) => { if (event.key === null || event.key === EFFICIENCY_UNIT_STORAGE_KEY) sync(); };
    window.addEventListener(EFFICIENCY_UNIT_EVENT, sync);
    window.addEventListener('storage', onStorage);
    return () => {
      window.removeEventListener(EFFICIENCY_UNIT_EVENT, sync);
      window.removeEventListener('storage', onStorage);
    };
  }, []);
  const setUnit = React.useCallback((next: EfficiencyUnit) => { setUnitState(next); writeEfficiencyUnit(next); }, []);
  return React.useMemo(() => ({
    unit,
    setUnit,
    spec: EFFICIENCY_UNITS[unit],
    label: EFFICIENCY_UNITS[unit].label,
    lowerIsBetter: EFFICIENCY_UNITS[unit].lowerIsBetter,
    convert: (whPerMile: number | null | undefined) => convertEfficiency(whPerMile, unit),
    format: (whPerMile: number | null | undefined) => formatEfficiencyValue(whPerMile, unit),
    formatValue: (whPerMile: number | null | undefined) => formatEfficiencyNumber(whPerMile, unit),
    formatWithUnit: (whPerMile: number | null | undefined) => formatEfficiencyWithUnit(whPerMile, unit),
  }), [unit, setUnit]);
}

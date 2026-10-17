import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
import { browserStorage } from './browserRuntime';

export type UnitSystem = 'imperial' | 'metric';
export type EfficiencyDisplay = 'distance_per_energy' | 'energy_per_distance';
export type UnitMode = 'imperial' | 'metric' | 'custom';
export type DistanceUnit = 'miles' | 'kilometers';
export type SpeedUnit = 'mph' | 'kmh';
export type TemperatureUnit = 'fahrenheit' | 'celsius';
export type PressureUnit = 'psi' | 'kpa';
export type AltitudeUnit = 'feet' | 'meters';
export type PlaceRadiusUnit = 'feet' | 'meters';

export interface UnitPreferences {
  mode: UnitMode;
  distance_unit: DistanceUnit;
  speed_unit: SpeedUnit;
  temperature_unit: TemperatureUnit;
  pressure_unit: PressureUnit;
  altitude_unit: AltitudeUnit;
  place_radius_unit: PlaceRadiusUnit;
  efficiency_display: EfficiencyDisplay;
}

const UNIT_SYSTEM_STORAGE_KEY = 'rm-units';
const EFFICIENCY_DISPLAY_STORAGE_KEY = 'rm-efficiency-display';
const UNIT_PREFERENCES_STORAGE_KEY = 'rm-unit-preferences';
const MILES_TO_KM = 1.609344;
const MPH_TO_KMH = 1.609344;
const METERS_TO_FEET = 3.28084;
const PSI_TO_KPA = 6.894757293168361;

const IMPERIAL_PREFS: UnitPreferences = {
  mode: 'imperial',
  distance_unit: 'miles',
  speed_unit: 'mph',
  temperature_unit: 'fahrenheit',
  pressure_unit: 'psi',
  altitude_unit: 'feet',
  place_radius_unit: 'feet',
  efficiency_display: 'distance_per_energy',
};

const METRIC_PREFS: UnitPreferences = {
  mode: 'metric',
  distance_unit: 'kilometers',
  speed_unit: 'kmh',
  temperature_unit: 'celsius',
  pressure_unit: 'kpa',
  altitude_unit: 'meters',
  place_radius_unit: 'meters',
  efficiency_display: 'distance_per_energy',
};

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const PREFERENCE_VALUES = {
  mode: ['imperial', 'metric', 'custom'],
  distance_unit: ['miles', 'kilometers'],
  speed_unit: ['mph', 'kmh'],
  temperature_unit: ['fahrenheit', 'celsius'],
  pressure_unit: ['psi', 'kpa'],
  altitude_unit: ['feet', 'meters'],
  place_radius_unit: ['feet', 'meters'],
  efficiency_display: ['distance_per_energy', 'energy_per_distance'],
} satisfies Record<keyof UnitPreferences, readonly string[]>;

function isUnitPreferences(value: unknown): value is UnitPreferences {
  if (typeof value !== 'object' || value === null) return false;
  return Object.entries(PREFERENCE_VALUES).every(([field, choices]) =>
    choices.includes((value as Record<string, string>)[field]!),
  );
}

export function getUnitPreferences(): UnitPreferences {
  const defaults = { ...IMPERIAL_PREFS };
  return browserStorage(defaults, (storage) => {
    let stored: unknown;
    try { stored = JSON.parse(storage.getItem(UNIT_PREFERENCES_STORAGE_KEY) ?? 'null'); } catch { /* Try legacy keys. */ }
    if (isUnitPreferences(stored)) return stored;
    const migrated = { ...defaults };
    try {
      if (storage.getItem(UNIT_SYSTEM_STORAGE_KEY) === 'metric') Object.assign(migrated, METRIC_PREFS);
      migrated.efficiency_display = storage.getItem(EFFICIENCY_DISPLAY_STORAGE_KEY) === 'energy_per_distance'
        ? 'energy_per_distance' : 'distance_per_energy';
    } catch { /* Preserve any preference read before storage failed. */ }
    setUnitPreferences(migrated, { suppressEvent: true });
    return migrated;
  });
}

export function setUnitPreferences(preferences: UnitPreferences, options: { suppressEvent?: boolean } = {}) {
  browserStorage(undefined, (storage, browser) => {
    const writes = [
      [UNIT_PREFERENCES_STORAGE_KEY, JSON.stringify(preferences)],
      [UNIT_SYSTEM_STORAGE_KEY, preferences.distance_unit === 'kilometers' ? 'metric' : 'imperial'],
      [EFFICIENCY_DISPLAY_STORAGE_KEY, preferences.efficiency_display],
    ] as const;
    for (const [key, value] of writes) storage.setItem(key, value);
    if (options.suppressEvent) return;
    for (const name of ['rm-units-change', 'rm-efficiency-display-change']) browser.dispatchEvent(new Event(name));
  });
}

export function getUnitSystem(): UnitSystem {
  return getUnitPreferences().distance_unit === 'kilometers' ? 'metric' : 'imperial';
}

export function setUnitSystem(system: UnitSystem) {
  setUnitPreferences({ ...(system === 'metric' ? METRIC_PREFS : IMPERIAL_PREFS) });
}

export function getEfficiencyDisplay(): EfficiencyDisplay {
  return getUnitPreferences().efficiency_display;
}

export function setEfficiencyDisplay(display: EfficiencyDisplay) {
  mergeUnitPreferences({ efficiency_display: display });
}

export function mergeUnitPreferences(partial: Partial<UnitPreferences>) {
  setUnitPreferences(Object.assign(getUnitPreferences(), partial));
}

type Numeric = number | null | undefined;
const missingNumber = (value: Numeric): value is null | undefined => value == null || Number.isNaN(value);
function numberText(value: Numeric, options: Intl.NumberFormatOptions): string {
  return missingNumber(value) ? '-' : new Intl.NumberFormat('en-US', options).format(value);
}
export function formatNumber(value: Numeric, decimals = 1): string {
  return numberText(value, { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
}
export function formatSmartNumber(value: Numeric, decimals = 1): string {
  return numberText(value, { minimumFractionDigits: 0, maximumFractionDigits: decimals });
}

type Measurement = readonly [label: string, digits: number, factor: number, divide?: boolean];
const MEASUREMENTS = {
  miles: ['mi', 0, 1], kilometers: ['km', 1, MILES_TO_KM],
  mph: ['mph', 0, 1], kmh: ['km/h', 0, MPH_TO_KMH],
  psi: ['psi', 0, 1], kpa: ['kPa', 0, PSI_TO_KPA],
  feet: ['ft', 0, METERS_TO_FEET], meters: ['m', 0, 1],
} satisfies Record<string, Measurement>;

function measurement(value: Numeric, specification: Measurement | (() => Measurement), decimals?: number): string {
  if (missingNumber(value)) return '-';
  const [label, digits, factor, divide] = typeof specification === 'function' ? specification() : specification;
  const converted = divide ? value / factor : value * factor;
  return `${formatNumber(converted, decimals ?? digits)} ${label}`;
}
export function formatMiles(value: Numeric, decimals?: number): string {
  return measurement(value, () => MEASUREMENTS[getUnitPreferences().distance_unit], decimals);
}
export function formatDistanceKm(value: Numeric): string {
  return measurement(value, () => getUnitPreferences().distance_unit === 'kilometers' ? ['km', 1, 1] : ['mi', 0, MILES_TO_KM, true]);
}
export function formatPercent(value: number, decimals = 0): string {
  return `${formatNumber(value, decimals)}%`;
}
export function formatSmartPercent(value: number, decimals = 0): string {
  return `${formatSmartNumber(value, decimals)}%`;
}
export function formatKwh(value: Numeric): string {
  return measurement(value, ['kWh', 1, 1]);
}
export function formatMph(value: Numeric): string {
  return measurement(value, () => MEASUREMENTS[getUnitPreferences().speed_unit]);
}
export function formatEfficiency(value: Numeric): string {
  const text = formatEfficiencyValue(value);
  return text === '-' ? text : [text, getEfficiencyUnitLabel()].join(' ');
}
export function formatEfficiencyFromWhPerKm(value: Numeric): string {
  return formatEfficiency(missingNumber(value) ? value : value * MILES_TO_KM);
}
export function formatMassKg(value: Numeric): string {
  return measurement(value, () => getUnitPreferences().distance_unit === 'kilometers' ? ['kg', 0, 1] : ['lb', 0, 2.2046226218]);
}
export function getEfficiencyUnitLabel(): string {
  const { distance_unit, efficiency_display } = getUnitPreferences();
  const distance = distance_unit === 'kilometers' ? 'km' : 'mi';
  return efficiency_display === 'energy_per_distance' ? `Wh/${distance}` : `${distance}/kWh`;
}
export function formatEfficiencyValue(value: Numeric): string {
  if (missingNumber(value)) return '-';
  const { distance_unit, efficiency_display } = getUnitPreferences();
  const metric = distance_unit === 'kilometers';
  const reciprocal = efficiency_display !== 'energy_per_distance';
  const converted = reciprocal
    ? (metric ? whPerMileToKmPerKwh(value) : whPerMileToMiPerKwh(value))
    : (metric ? value / MILES_TO_KM : value);
  return formatNumber(converted, reciprocal ? 1 : 0);
}
export function formatEnergyPerDistance(value: Numeric): string {
  return measurement(value, () => getUnitPreferences().distance_unit === 'kilometers' ? ['Wh/km', 0, MILES_TO_KM, true] : ['Wh/mi', 0, 1]);
}
function efficiencyConversion(value: Numeric, reciprocal: boolean, metric: boolean): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  if (!reciprocal) return value / MILES_TO_KM;
  if (!(value > 0)) return null;
  const distance = 1000 / value;
  return metric ? distance * MILES_TO_KM : distance;
}
export function whPerMileToMiPerKwh(value: Numeric): number | null {
  return efficiencyConversion(value, true, false);
}
export function whPerMileToKmPerKwh(value: Numeric): number | null {
  return efficiencyConversion(value, true, true);
}
export function whPerMileToWhPerKm(value: Numeric): number | null {
  return efficiencyConversion(value, false, true);
}
export function formatPressure(value: Numeric): string {
  return measurement(value, () => MEASUREMENTS[getUnitPreferences().pressure_unit]);
}
export function formatAltitude(valueMeters: Numeric): string {
  return measurement(valueMeters, () => MEASUREMENTS[getUnitPreferences().altitude_unit]);
}
export function formatCurrency(value: Numeric): string {
  return numberText(value, { style: 'currency', currency: 'USD' });
}
export function formatDuration(minutes: Numeric): string {
  if (missingNumber(minutes)) return '-';
  const hours = minutes < 60 ? null : Math.floor(minutes / 60);
  const remainder = Math.round(hours === null ? minutes : minutes % 60);
  const pieces = hours === null ? [] : [`${hours}h`];
  if (hours === null || remainder !== 0) pieces.push(`${remainder}m`);
  return pieces.join(' ');
}
export function formatTemp(celsius: Numeric): string {
  if (missingNumber(celsius)) return '-';
  const fahrenheit = getUnitPreferences().temperature_unit !== 'celsius';
  return `${Math.round(fahrenheit ? celsius * 9 / 5 + 32 : celsius)} ${fahrenheit ? 'F' : 'C'}`;
}

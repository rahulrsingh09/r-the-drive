import { CHART_COLORS } from './ChartProvider';
import { formatKwh, formatPercent } from '../lib/utils';

export function thresholdColor(value: number, bands: ReadonlyArray<readonly [number, string]>, direction: 'above' | 'below', fallback = CHART_COLORS.danger as string) {
  const match = bands.find(([threshold]) => direction === 'above' ? value > threshold : value < threshold);
  return match?.[1] ?? fallback;
}

export function nullableProjection<T, M extends object, K extends string>(items: T[], field: (item: T) => number | null, metadata: (item: T) => M, key: K, factor: number): Array<M & Record<K, number | null>> {
  return items.map(item => {
    const value = field(item);
    return { ...metadata(item), [key]: value === null || factor === 1 ? value : value * factor } as M & Record<K, number | null>;
  });
}

const CAPACITY_FIELDS = new Map([['capacity_pct', { suffix: '%', label: 'Capacity' }]]);
export function capacityTooltip(value: number | undefined, name: string): [string, string] {
  const field = CAPACITY_FIELDS.get(name) ?? { suffix: ' kWh', label: 'Usable' };
  return [`${(value ?? 0).toFixed(1)}${field.suffix}`, field.label];
}

const CHARGE_FIELDS = new Map([['power_kw', { format: formatKwh, label: 'Power' }]]);
export function chargeTooltip(value: number | undefined, name: string): [string, string] {
  const field = CHARGE_FIELDS.get(name) ?? { format: formatPercent, label: 'SoC' };
  return [field.format(value ?? 0), field.label];
}

function elapsedLabel(seconds: number, compact: boolean) {
  const fields = [Math.floor(seconds / 60), compact ? seconds % 60 : Math.max(0, Math.floor(seconds % 60))];
  if (compact && fields[1] === 0) return `${fields[0]}m`;
  return fields.map((value, index) => index ? String(value).padStart(2, '0') : String(value)).join(':');
}

export function elapsedTripLabel(seconds: number) {
  return elapsedLabel(seconds, false);
}

export function elapsedProfileLabel(seconds: number) {
  return elapsedLabel(seconds, true);
}

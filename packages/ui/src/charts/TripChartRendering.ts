import * as React from 'react';
import { getUnitPreferences } from '../lib/utils';

type HasElapsed = { elapsed_s: number };

type SampleDotStyle = {
  fill: string;
  r: number;
  strokeWidth: number;
};

type SampleDotProps = {
  cx?: number;
  cy?: number;
  payload?: { elapsed_s?: number | null };
  value?: number | null;
};

export const TRIP_MARKER_FULL_THRESHOLD = 120;
export const TRIP_MARKER_TARGET_COUNT = 75;

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function displayTemperature(celsius: number, fahrenheit: boolean) {
  return fahrenheit ? celsius * 9 / 5 + 32 : celsius;
}

export function getVisibleSampleElapsedSet<T extends HasElapsed>(
  data: readonly T[],
  hasValue: (point: T) => boolean,
  fullMarkerThreshold = TRIP_MARKER_FULL_THRESHOLD,
  targetMarkerCount = TRIP_MARKER_TARGET_COUNT,
) {
  const elapsed: number[] = [];
  data.forEach(point => { if (hasValue(point)) elapsed.push(point.elapsed_s); });
  const count = elapsed.length;
  const stride = count <= fullMarkerThreshold ? 1 : Math.max(1, Math.ceil(count / targetMarkerCount));
  const selected = new Set<number>();
  for (let index = 0; index < count; index += stride) selected.add(elapsed[index]!);
  if (count) selected.add(elapsed[count - 1]!);
  return selected;
}

export function createSampleDotRenderer(
  visibleElapsedSet: ReadonlySet<number>,
  style: SampleDotStyle,
) {
  return function SampleDot(props: SampleDotProps) {
    const coordinates = [props.cx, props.cy, props.value, props.payload?.elapsed_s];
    if (!coordinates.every(isFiniteNumber) || !visibleElapsedSet.has(coordinates[3]!)) return null;
    return React.createElement('circle', { cx: props.cx, cy: props.cy, fill: style.fill, r: style.r, strokeWidth: style.strokeWidth });
  };
}

export function getTripTemperatureDomain(
  valuesCelsius: readonly (number | null | undefined)[],
  temperatureUnit: 'fahrenheit' | 'celsius' = getUnitPreferences().temperature_unit,
) {
  const fahrenheit = temperatureUnit === 'fahrenheit';
  let minimum = Infinity;
  let maximum = -Infinity;
  let measured = false;
  for (const value of valuesCelsius) {
    if (!isFiniteNumber(value)) continue;
    measured = true;
    const display = displayTemperature(value, fahrenheit);
    minimum = Math.min(minimum, display);
    maximum = Math.max(maximum, display);
  }
  if (!measured) return null;
  const step = fahrenheit ? 5 : 2;
  const headroom = Math.max(Math.max(1, maximum - minimum) * 0.15, step);
  const bounds = [Math.floor(minimum / step) * step, Math.ceil((maximum + headroom) / step) * step];
  return [
    fahrenheit ? (bounds[0]! - 32) * 5 / 9 : bounds[0]!,
    fahrenheit ? (bounds[1]! - 32) * 5 / 9 : bounds[1]!,
  ] as const;
}

import * as React from 'react';
import { createSampleDotRenderer, getVisibleSampleElapsedSet } from './TripChartRendering';
import { getActiveElapsedSFromChartState, type TripChartMouseState } from './TripChartSync';

type Marker = { fill: string; r: number; strokeWidth: number };

/** Index every measured column once so all trip overlays select the same observations. */
export function useTripMeasurements<T extends { elapsed_s: number }>(
  data: T[], fields: Partial<Record<keyof T, Marker>>,
  active: number | null, onChange?: (elapsed: number | null) => void,
) {
  const indexed = React.useMemo(() => {
    const keys = Object.keys(fields) as Array<keyof T>;
    const columns = new Map<keyof T, T[]>();
    for (const key of keys) columns.set(key, []);
    const measured: T[] = [];
    for (const point of data) {
      let observed = false;
      for (const key of keys) {
        if (point[key] == null) continue;
        columns.get(key)!.push(point);
        observed = true;
      }
      if (observed) measured.push(point);
    }
    const dots = {} as Record<keyof T, ReturnType<typeof createSampleDotRenderer>>;
    for (const key of keys) dots[key] = createSampleDotRenderer(
      getVisibleSampleElapsedSet(columns.get(key)!, () => true), fields[key]!,
    );
    return { measured, dots };
  }, [data, fields]);
  const onMouseMove = React.useCallback((state: unknown) => {
    if (onChange) onChange(getActiveElapsedSFromChartState(state as TripChartMouseState<T>, indexed.measured, active));
  }, [onChange, indexed, active]);
  const onMouseLeave = React.useCallback(() => { onChange?.(null); }, [onChange]);
  return { ...indexed, onMouseMove, onMouseLeave };
}

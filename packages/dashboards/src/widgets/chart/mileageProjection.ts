import type { ChartDefinitionV1 } from '@panda/types';

type MileagePoint = {
  ts: string;
  x: number | null;
  y?: number | null;
  rangeMi: number | null;
  projectedMaxRangeMi: number | null;
  degradationPct?: number | null;
};
const FIELD_KEYS = {
  odometer_miles: 'x',
  usable_kwh: 'y',
  range_mi: 'rangeMi',
  projected_max_range_mi: 'projectedMaxRangeMi',
  degradation_pct: 'degradationPct',
} as const;
export function mileageProjection<T extends MileagePoint>(
  points: T[],
  definition: ChartDefinitionV1,
  mode: 'capacity' | 'range'
) {
  const domainKey = FIELD_KEYS[definition.x.field.field as keyof typeof FIELD_KEYS];
  const time = definition.x.kind === 'time';
  const rows: Array<
    T & { domain: number; projectedRangeMi: number | null; odometerMi: number | null }
  > = [];
  for (const point of points) {
    if (point.x == null && (mode === 'range' || point.y == null)) continue;
    const raw = time
      ? definition.x.field.field === 'timestamp'
        ? Date.parse(point.ts) / 1000
        : NaN
      : point[domainKey];
    const domain = raw == null ? NaN : Number(raw);
    if (!Number.isFinite(domain)) continue;
    rows.push({
      ...point,
      domain,
      projectedRangeMi: point.projectedMaxRangeMi ?? point.rangeMi,
      odometerMi: point.x,
    });
  }
  return rows.sort((left, right) => left.domain - right.domain);
}

import {
  getEfficiencyDisplay,
  getUnitSystem,
  whPerMileToWhPerKm,
  whPerMileToKmPerKwh,
  whPerMileToMiPerKwh,
} from '@panda/ui/lib/utils';

export function efficiencyProjection() {
  const metric = getUnitSystem() === 'metric';
  const energy = getEfficiencyDisplay() === 'energy_per_distance';
  const unit = energy ? (metric ? 'Wh/km' : 'Wh/mi') : metric ? 'km/kWh' : 'mi/kWh';
  const convert = energy
    ? metric
      ? whPerMileToWhPerKm
      : (value: number) => value
    : metric
      ? whPerMileToKmPerKwh
      : whPerMileToMiPerKwh;
  return {
    unit,
    convert(value: number | null | undefined) {
      return value == null ? null : convert(value);
    },
  };
}
export function efficiencyRows<T, P extends object>(
  rows: T[],
  number: (row: T) => number | null,
  present: (row: T) => P,
  order?: (left: T, right: T) => number
) {
  const projection = efficiencyProjection();
  const selected = rows.flatMap((row) => {
    const raw = number(row);
    const value = raw == null ? null : projection.convert(raw);
    return value == null ? [] : [{ row, value }];
  });
  if (order) selected.sort((a, b) => order(a.row, b.row));
  return selected.map(({ row, value }) => ({ ...present(row), value }));
}

export function efficiencyTimeline(
  rows: Array<{ ts: string; trip_efficiency_wh_mi: number | null; rolling_24h_wh_mi: number | null }>
) {
  const projection = efficiencyProjection();
  const points: Array<{ ts: string }> = [];
  const values = { trip_efficiency_wh_mi: [] as Array<number | null>, rolling_24h_wh_mi: [] as Array<number | null> };
  for (const row of rows) {
    points.push({ ts: row.ts });
    for (const field of Object.keys(values) as Array<keyof typeof values>)
      values[field].push(projection.convert(row[field]));
  }
  return { unit: projection.unit, points, values };
}

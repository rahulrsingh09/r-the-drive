import type { ChargeCurvePoint } from '@panda/types';

export function prepareSessionCurve(
  data: ChargeCurvePoint[],
  startedAt: string | null,
  totalEnergy: number | null
) {
  const valid = data.filter(
    (point) => Number.isFinite(point.soc_pct) && Number.isFinite(point.power_kw)
  );
  const start = startedAt ? new Date(startedAt).getTime() : null;
  const useTime = start != null && valid.some((point) => point.minutes_elapsed != null);
  let end = valid.length;
  for (let index = valid.length - 1; index >= 0; index--) {
    if ((valid[index]!.power_kw ?? 0) <= 0.1) continue;
    end = Math.min(valid.length, index + 2);
    break;
  }
  const rows = valid.slice(0, end);
  const points: Array<{ ts: string | number }> = [];
  const rateValues: number[] = [];
  const energyValues: number[] = [];
  let energy = 0;
  for (let index = 0; index < rows.length; index++) {
    const row = rows[index]!,
      previous = rows[index - 1];
    if (previous?.minutes_elapsed != null && row.minutes_elapsed != null)
      energy +=
        Math.max(0, row.power_kw ?? 0) * ((row.minutes_elapsed - previous.minutes_elapsed) / 60);
    points.push({
      ts: useTime ? new Date(start! + row.minutes_elapsed! * 60000).toISOString() : row.soc_pct,
    });
    rateValues.push(row.power_kw);
    energyValues.push(Math.max(0, energy));
  }
  const lastEnergy = energyValues.at(-1) ?? 0;
  if (typeof totalEnergy === 'number' && Number.isFinite(totalEnergy) && lastEnergy > 0) {
    const scale = Math.max(0, totalEnergy) / lastEnergy;
    for (let index = 0; index < energyValues.length; index++)
      energyValues[index] = energyValues[index]! * scale;
  }
  if (useTime && start! + (rows[0]?.minutes_elapsed ?? 0) * 60000 - start! > 300000) {
    points.unshift({ ts: startedAt! });
    rateValues.unshift(0);
    energyValues.unshift(0);
  }
  return { points, rateValues, energyValues, useTime };
}

export function sessionHourSplits(points: Array<{ ts: string | number }>, useTime: boolean) {
  if (!useTime) return undefined;
  const seconds = points
    .map((point) => new Date(String(point.ts)).getTime() / 1000)
    .filter(Number.isFinite);
  if (seconds.length < 2) return undefined;
  const first = Math.ceil(seconds[0]! / 3600) * 3600;
  const count = Math.floor((seconds.at(-1)! - first) / 3600) + 1;
  return count < 2 ? undefined : Array.from({ length: count }, (_, index) => first + index * 3600);
}

import type { DailyChargeSessionsDay, DailyChargeSessionsSession } from './DailyChargeSessionsChart';
import { CHART_COLORS } from './ChartProvider';

export type ChargerGroupKey = 'ac' | 'dc' | 'unknown';
export const GROUP_ORDER: ChargerGroupKey[] = ['ac', 'dc', 'unknown'];
export const GROUP_META = {
  ac: { key: 'ac', label: 'AC', color: CHART_COLORS.emerald },
  dc: { key: 'dc', label: 'DC', color: CHART_COLORS.amber },
  unknown: { key: 'unknown', label: 'Unknown', color: CHART_COLORS.muted },
} as const;
type GroupMeta = { key: ChargerGroupKey; label: string; color: string };
export interface ChargerGroup extends GroupMeta {
  energyKwh: number; costUsd: number | null; sessionCount: number;
  sessions: DailyChargeSessionsSession[];
}
export interface PreparedDay extends DailyChargeSessionsDay {
  totalEnergyKwh: number; groups: ChargerGroup[];
}
const CHARGER_KEYS = new Map<string, ChargerGroupKey>([['ac', 'ac'], ['ac_l2', 'ac'], ['dc', 'dc'], ['dcfc', 'dc']]);

/** Join daily totals against a chronological, per-day session index. */
export function prepareChargingDays(daily: DailyChargeSessionsDay[], sessions: DailyChargeSessionsSession[], color?: string): PreparedDay[] {
  const byDay = new Map<string, DailyChargeSessionsSession[]>();
  for (const session of sessions) {
    const bucket = byDay.get(session.day_local);
    if (bucket) bucket.push(session); else byDay.set(session.day_local, [session]);
  }
  const aggregates = new Map<string, { energy: number; groups: ChargerGroup[] }>();
  for (const [day, bucket] of byDay) {
    bucket.sort((a, b) => new Date(a.started_at).getTime() - new Date(b.started_at).getTime());
    const groups = new Map<ChargerGroupKey, ChargerGroup>();
    let energy = 0;
    for (const session of bucket) {
      const key = CHARGER_KEYS.get(session.charger_type?.trim().toLowerCase() ?? '') ?? 'unknown';
      let group = groups.get(key);
      if (!group) {
        group = { ...GROUP_META[key], color: color ?? GROUP_META[key].color, energyKwh: 0, costUsd: null, sessionCount: 0, sessions: [] };
        groups.set(key, group);
      }
      const value = Math.max(0, session.energy_added_kwh ?? 0);
      energy += value;
      group.energyKwh += value;
      if (Number.isFinite(session.cost_usd)) group.costUsd = group.sessionCount ? (group.costUsd ?? 0) + session.cost_usd! : session.cost_usd;
      group.sessionCount += 1;
      group.sessions.push(session);
    }
    aggregates.set(day, { energy, groups: GROUP_ORDER.flatMap(key => groups.has(key) ? [groups.get(key)!] : []) });
  }
  const result: PreparedDay[] = [];
  for (const day of daily) {
    const aggregate = aggregates.get(day.day_local);
    const totalEnergyKwh = day.total_energy_kwh > 0 ? day.total_energy_kwh : aggregate?.energy ?? 0;
    const groups = aggregate?.groups ?? [];
    if (totalEnergyKwh > 0 || groups.length) result.push({ ...day, totalEnergyKwh, groups });
  }
  return result;
}

export function chargingDayGeometry(days: PreparedDay[], left: number, top: number, innerHeight: number, slotWidth: number, barWidth: number, axisSpan: number, renderHeight: number) {
  return days.map((day, index) => {
    const x = left + slotWidth * index + (slotWidth - barWidth) / 2;
    const barHeight = (day.totalEnergyKwh / axisSpan) * renderHeight;
    const clippedBarHeight = Math.max(0, barHeight);
    const y = top + innerHeight - barHeight;
    let offset = 0;
    let sessionCount = 0;
    const segments = day.groups.map(group => {
      const height = day.totalEnergyKwh > 0 ? (group.energyKwh / axisSpan) * renderHeight : 0;
      const segment = { group, y: top + innerHeight - clippedBarHeight + offset, height };
      offset += height;
      sessionCount += group.sessionCount;
      return segment;
    });
    return { ...day, x, y, barHeight, clippedBarHeight, totalLabelY: top + innerHeight - (day.totalEnergyKwh / axisSpan) * renderHeight - 8, labelX: left + slotWidth * index + slotWidth / 2, sessionCount, segments };
  });
}

import { formatKwh, formatPercent } from '@panda/ui/lib/utils';
import type { PhantomDrainPeriod } from '@panda/types';
import type { SensorDataAccent } from '@panda/dashboards';

export interface PhantomDrainSummary {
  maxDrainPctPerHour: number | null;
  avgSleepPct: number | null;
  avgStateCoveragePct: number | null;
  totalEnergyDrainedKwh: number;
  avgDrainPctPerHour: number | null;
}

export interface PhantomDrainSummaryCard {
  key: string;
  title: string;
  value: string;
  icon: string;
  dataAccent: SensorDataAccent;
  accentBorder?: boolean;
  secondary?: string;
}

export function summarizePhantomDrainPeriods(periods: PhantomDrainPeriod[]): PhantomDrainSummary {
  const totals = { energy: 0, hours: 0, loss: 0 };
  const weighted = {
    sleep: { sum: 0, hours: 0 },
    coverage: { sum: 0, hours: 0 },
  };
  let maximum: number | null = null;
  for (const period of periods) {
    const duration = finiteNumber(period.duration_hours);
    const rate = finiteNumber(period.drain_pct_per_hour);
    if (rate !== null) maximum = maximum === null ? rate : Math.max(maximum, rate);
    totals.hours += duration ?? 0;
    totals.loss += finiteNumber(period.soc_lost_pct) ?? 0;
    const energy = period.energy_drained_kwh;
    if (energy != null && !Number.isNaN(energy)) totals.energy += energy;
    if (duration === null) continue;
    const observations = { sleep: period.sleep_share_pct, coverage: period.state_coverage_pct };
    for (const key of ['sleep', 'coverage'] as const) {
      const value = finiteNumber(observations[key]);
      if (value === null) continue;
      weighted[key].sum += duration * value;
      weighted[key].hours += duration;
    }
  }
  const average = ({ sum, hours }: { sum: number; hours: number }) => hours > 0 ? sum / hours : null;
  return {
    maxDrainPctPerHour: maximum,
    avgSleepPct: average(weighted.sleep),
    avgStateCoveragePct: average(weighted.coverage),
    totalEnergyDrainedKwh: totals.energy,
    avgDrainPctPerHour: average({ sum: totals.loss, hours: totals.hours }),
  };
}

export function buildPhantomDrainSummaryCards(summary: PhantomDrainSummary): PhantomDrainSummaryCard[] {
  const rows: Array<[keyof PhantomDrainSummary, string, string, string, SensorDataAccent, number, number, string]> = [
    ['maxDrainPctPerHour', 'max-drain-rate', 'Max drain rate', 'activity', 'series-02', 1, 2, ' / h'],
    ['avgSleepPct', 'avg-sleep', 'Avg sleep', 'moon-star', 'series-03', 100, 1, ''],
    ['totalEnergyDrainedKwh', 'total-energy-drained', 'Total energy drained', 'bolt', 'series-01', 1, 1, ''],
    ['avgDrainPctPerHour', 'avg-drain-per-hour', 'Avg drain per hour', 'timer', 'series-04', 1, 2, ' / h'],
  ];
  return rows.map(([field, key, title, icon, dataAccent, scale, digits, suffix]) => {
    const reading = summary[field];
    let value = '-';
    if (field === 'totalEnergyDrainedKwh') value = formatKwh(reading);
    else if (reading != null) value = formatPercent(reading * scale, digits) + suffix;
    const card: PhantomDrainSummaryCard = { key, title, value, icon: `lucide:${icon}`, dataAccent };
    if (field === 'maxDrainPctPerHour') card.accentBorder = true;
    if (field === 'avgSleepPct') {
      const coverage = summary.avgStateCoveragePct;
      card.secondary = `State coverage ${coverage == null ? 'unknown' : formatPercent(coverage * 100, 0)}`;
    }
    return card;
  });
}

function finiteNumber(value: number | null | undefined) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

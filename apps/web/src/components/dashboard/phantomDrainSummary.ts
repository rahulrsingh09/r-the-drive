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

function formatRatioPercent(value: number | null, decimals = 0) {
  return value == null ? '-' : formatPercent(value * 100, decimals);
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
  return [
    {
      key: 'max-drain-rate',
      title: 'Max drain rate',
      value: summary.maxDrainPctPerHour == null ? '-' : `${formatPercent(summary.maxDrainPctPerHour, 2)} / h`,
      icon: 'lucide:activity',
      dataAccent: 'series-02',
      accentBorder: true,
    },
    {
      key: 'avg-sleep',
      title: 'Avg sleep',
      value: formatRatioPercent(summary.avgSleepPct, 1),
      icon: 'lucide:moon-star',
      dataAccent: 'series-03',
      secondary: `State coverage ${summary.avgStateCoveragePct == null ? 'unknown' : formatRatioPercent(summary.avgStateCoveragePct, 0)}`,
    },
    {
      key: 'total-energy-drained',
      title: 'Total energy drained',
      value: formatKwh(summary.totalEnergyDrainedKwh),
      icon: 'lucide:bolt',
      dataAccent: 'series-01',
    },
    {
      key: 'avg-drain-per-hour',
      title: 'Avg drain per hour',
      value: summary.avgDrainPctPerHour == null ? '-' : `${formatPercent(summary.avgDrainPctPerHour, 2)} / h`,
      icon: 'lucide:timer',
      dataAccent: 'series-04',
    },
  ];
}

function finiteNumber(value: number | null | undefined) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

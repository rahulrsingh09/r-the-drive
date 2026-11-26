import type { ParkedEnergySample, PhantomDrainPeriod, StateTimelinePeriod } from '@panda/types';
import { cToF, dayKeyOf, n0, n1, n2, sum, type DayKey } from './insightsModel';

/** Days losing at least this much SoC while parked open full diagnostic detail. */
export const SPIKE_LOSS_PCT = 1.5;

export interface ParkedDaySummary {
  day: DayKey;
  hours: number;
  loss: number;
  kwh: number;
  periods: PhantomDrainPeriod[];
  isSpike: boolean;
  sleepPct: number | null;
  drainPctPerHour: number | null;
  avgWatts: number | null;
  socStart: number | null;
  socEnd: number | null;
  avgOutsideTempC: number | null;
  avgCabinTempC: number | null;
  wakeCount: number | null;
  maxSleepStreakMinutes: number | null;
  gearGuardActive: boolean;
  climateActive: boolean;
  otaActive: boolean;
  openClosureDetected: boolean | null;
  doorsUnlocked: boolean | null;
  twelveVoltAttention: boolean | null;
  pluggedIn: boolean | null;
}

export interface ParkedTimelineSlice {
  kind: 'sleep' | 'awake';
  startMs: number;
  endMs: number;
  minutes: number;
  label: string;
}

export interface ParkedTimelineSummary {
  slices: ParkedTimelineSlice[];
  sleepMinutes: number;
  awakeMinutes: number;
  unreportedMinutes: number;
  totalMinutes: number;
  sleepPct: number | null;
  awakeTriggers: string[];
}

export interface ParkedEnergyCategoryRow {
  key: 'vehicle_systems' | 'climate' | 'gear_guard' | 'outlets' | 'other';
  label: string;
  kwh: number;
  sharePct: number;
  color: string;
}

export interface ParkedEnergyBreakdown {
  totalKwh: number;
  sampleCount: number;
  categories: ParkedEnergyCategoryRow[];
}

function weightedAverage(
  periods: readonly PhantomDrainPeriod[],
  readValue: (p: PhantomDrainPeriod) => number | null | undefined,
): number | null {
  let weighted = 0;
  let totalWeight = 0;
  for (const period of periods) {
    const val = readValue(period);
    const weight = period.duration_hours ?? 0;
    if (val != null && Number.isFinite(val) && weight > 0) {
      weighted += val * weight;
      totalWeight += weight;
    }
  }
  return totalWeight > 0 ? weighted / totalWeight : null;
}

function anyTrue(periods: readonly PhantomDrainPeriod[], read: (p: PhantomDrainPeriod) => boolean | null | undefined): boolean {
  return periods.some((p) => read(p) === true);
}

function nullableFlag(periods: readonly PhantomDrainPeriod[], read: (p: PhantomDrainPeriod) => boolean | null | undefined): boolean | null {
  let sawValue = false;
  for (const p of periods) {
    const val = read(p);
    if (val === true) return true;
    if (val === false) sawValue = true;
  }
  return sawValue ? false : null;
}

/** Group validated parked periods by the day parking started with aggregated diagnostics. */
export function buildParkedDaySummaries(
  periods: readonly PhantomDrainPeriod[],
  timezone: string,
): Map<DayKey, ParkedDaySummary> {
  const byDay = new Map<DayKey, PhantomDrainPeriod[]>();
  for (const period of periods) {
    if (period.validation_status !== 'validated' || !period.period_start || period.duration_hours == null) continue;
    const day = dayKeyOf(period.period_start, timezone);
    byDay.set(day, [...(byDay.get(day) ?? []), period]);
  }

  const out = new Map<DayKey, ParkedDaySummary>();
  for (const [day, list] of byDay.entries()) {
    const ordered = [...list].sort((a, b) => Date.parse(a.period_start!) - Date.parse(b.period_start!));
    const hours = sum(ordered, (p) => Math.max(0, p.duration_hours ?? 0));
    const loss = sum(ordered, (p) => {
      const l = p.soc_lost_pct ?? (p.soc_start != null && p.soc_end != null ? p.soc_start - p.soc_end : null);
      return l != null ? Math.max(0, l) : 0;
    });
    const kwh = sum(ordered, (p) => (p.energy_drained_kwh != null && p.energy_drained_kwh > 0 ? p.energy_drained_kwh : 0));
    const sleepShare = weightedAverage(ordered, (p) => p.sleep_share_pct);
    const wakeValues = ordered.map((p) => p.wake_count).filter((v): v is number => v != null && Number.isFinite(v));
    const streakValues = ordered
      .map((p) => p.max_sleep_streak_minutes)
      .filter((v): v is number => v != null && Number.isFinite(v));

    out.set(day, {
      day,
      hours,
      loss,
      kwh,
      periods: ordered,
      isSpike: loss >= SPIKE_LOSS_PCT,
      sleepPct: sleepShare != null ? Math.round(sleepShare * 100) : null,
      drainPctPerHour: hours > 0 ? loss / hours : null,
      avgWatts: weightedAverage(ordered, (p) => p.avg_power_w),
      socStart: ordered[0]?.soc_start ?? null,
      socEnd: ordered.at(-1)?.soc_end ?? null,
      avgOutsideTempC: weightedAverage(ordered, (p) => p.avg_outside_temp_c),
      avgCabinTempC: weightedAverage(ordered, (p) => p.avg_cabin_temp_c),
      wakeCount: wakeValues.length ? wakeValues.reduce((a, b) => a + b, 0) : null,
      maxSleepStreakMinutes: streakValues.length ? Math.max(...streakValues) : null,
      gearGuardActive: anyTrue(ordered, (p) => p.gear_guard_active),
      climateActive: anyTrue(ordered, (p) => p.climate_active),
      otaActive: anyTrue(ordered, (p) => p.ota_active),
      openClosureDetected: nullableFlag(ordered, (p) => p.open_closure_detected),
      doorsUnlocked: nullableFlag(ordered, (p) => p.doors_unlocked),
      twelveVoltAttention: nullableFlag(ordered, (p) => p.twelve_volt_attention),
      pluggedIn: nullableFlag(ordered, (p) => p.plugged_in),
    });
  }
  return out;
}

/** Pick default bar to highlight: newest spike day if any, otherwise newest day with parked loss data. */
export function pickDefaultParkedDay(
  days30: readonly DayKey[],
  summaries: ReadonlyMap<DayKey, ParkedDaySummary>,
): DayKey | null {
  for (let i = days30.length - 1; i >= 0; i -= 1) {
    const day = days30[i]!;
    if (summaries.get(day)?.isSpike) return day;
  }
  for (let i = days30.length - 1; i >= 0; i -= 1) {
    const day = days30[i]!;
    if (summaries.has(day)) return day;
  }
  return null;
}

const SLEEP_STATES = new Set(['sleep']);
const AWAKE_STATES = new Set(['idle', 'awake', 'standby', 'ready']);

/**
 * Build a parked-only Sleep vs Awake timeline strictly scoped to one day's parked periods.
 * Drive/charge intervals are never included; triggers attach only to the awake summary.
 */
export function buildParkedOnlyTimeline(
  summary: ParkedDaySummary,
  statePeriods: readonly StateTimelinePeriod[],
  fallbackNowMs = Date.now(),
): ParkedTimelineSummary {
  const awakeTriggers: string[] = [];
  if (summary.gearGuardActive) awakeTriggers.push('Gear Guard');
  if (summary.climateActive) awakeTriggers.push('Climate');
  if (summary.otaActive) awakeTriggers.push('OTA update');

  const rawSlices: ParkedTimelineSlice[] = [];
  let totalMinutes = 0;

  for (const park of summary.periods) {
    if (!park.period_start) continue;
    const pStart = Date.parse(park.period_start);
    const pEnd = park.period_end ? Date.parse(park.period_end) : fallbackNowMs;
    if (!Number.isFinite(pStart) || !Number.isFinite(pEnd) || pEnd <= pStart) continue;
    totalMinutes += (pEnd - pStart) / 60_000;

    for (const sp of statePeriods) {
      const state = sp.state.trim().toLowerCase();
      const kind = SLEEP_STATES.has(state) ? 'sleep' : AWAKE_STATES.has(state) ? 'awake' : null;
      if (!kind) continue;
      const sStart = Date.parse(sp.started_at);
      const sEnd = sp.ended_at ? Date.parse(sp.ended_at) : fallbackNowMs;
      if (!Number.isFinite(sStart) || !Number.isFinite(sEnd)) continue;
      const startMs = Math.max(pStart, sStart);
      const endMs = Math.min(pEnd, sEnd);
      if (endMs <= startMs) continue;
      rawSlices.push({
        kind,
        startMs,
        endMs,
        minutes: (endMs - startMs) / 60_000,
        label: kind === 'sleep' ? 'Deep sleep' : 'Kept awake',
      });
    }
  }

  rawSlices.sort((a, b) => a.startMs - b.startMs);
  let sleepMinutes = rawSlices.filter((s) => s.kind === 'sleep').reduce((acc, s) => acc + s.minutes, 0);
  let awakeMinutes = rawSlices.filter((s) => s.kind === 'awake').reduce((acc, s) => acc + s.minutes, 0);

  // Fall back to period sleep_share_pct if state-timeline rows have not loaded yet.
  if (sleepMinutes === 0 && awakeMinutes === 0 && summary.sleepPct != null && totalMinutes > 0) {
    sleepMinutes = (summary.sleepPct / 100) * totalMinutes;
    awakeMinutes = Math.max(0, totalMinutes - sleepMinutes);
  }

  const coveredMinutes = sleepMinutes + awakeMinutes;
  const unreportedMinutes = Math.max(0, totalMinutes - coveredMinutes);
  const denom = coveredMinutes > 0 ? coveredMinutes : totalMinutes;
  const sleepPct = denom > 0 ? Math.round((sleepMinutes / denom) * 100) : summary.sleepPct;

  return {
    slices: rawSlices,
    sleepMinutes,
    awakeMinutes,
    unreportedMinutes,
    totalMinutes,
    sleepPct,
    awakeTriggers,
  };
}

/** Format temperature reading respecting unit preference. */
export function formatTempReading(tempC: number, fahrenheit: boolean): string {
  return fahrenheit ? `${n0(cToF(tempC))}°F` : `${n1(tempC)}°C`;
}

/** Format minutes into concise "Xh Ym" or "Y min". */
export function formatDurationMinutes(minutes: number): string {
  const rounded = Math.max(0, Math.round(minutes));
  const h = Math.floor(rounded / 60);
  const m = rounded % 60;
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/**
 * Summarize Rivian-reported parked energy into flat category rows matching CostTab's Where you charge.
 * Prefers distinct daily `24h` samples (one latest sample per day), falling back to latest `since_parked` sample.
 */
export function summarizeParkedEnergy(
  samples: readonly ParkedEnergySample[],
  timezone: string,
): ParkedEnergyBreakdown {
  if (!samples.length) return { totalKwh: 0, sampleCount: 0, categories: [] };

  const daily24h = new Map<DayKey, ParkedEnergySample>();
  let latestSinceParked: ParkedEnergySample | null = null;
  for (const sample of samples) {
    if (sample.window === '24h') {
      const day = dayKeyOf(sample.source_at, timezone);
      const prev = daily24h.get(day);
      if (!prev || Date.parse(sample.source_at) >= Date.parse(prev.source_at)) {
        daily24h.set(day, sample);
      }
    } else if (sample.window === 'since_parked') {
      if (!latestSinceParked || Date.parse(sample.source_at) >= Date.parse(latestSinceParked.source_at)) {
        latestSinceParked = sample;
      }
    }
  }

  const chosen = daily24h.size ? [...daily24h.values()] : latestSinceParked ? [latestSinceParked] : [samples[0]!];
  const systems = sum(chosen, (s) => Math.max(0, s.vehicle_systems_kwh ?? 0));
  const climate = sum(chosen, (s) => Math.max(0, s.climate_kwh ?? 0));
  const gearGuard = sum(chosen, (s) => Math.max(0, s.gear_guard_kwh ?? 0));
  const outlets = sum(chosen, (s) => Math.max(0, s.outlets_kwh ?? 0));
  const partSum = systems + climate + gearGuard + outlets;
  const reportedTotal = sum(chosen, (s) => Math.max(0, s.total_kwh ?? 0));
  const totalKwh = Math.max(partSum, reportedTotal);
  if (totalKwh <= 0) return { totalKwh: 0, sampleCount: chosen.length, categories: [] };

  const other = Math.max(0, totalKwh - partSum);
  const raw: Array<Omit<ParkedEnergyCategoryRow, 'sharePct'>> = [
    { key: 'vehicle_systems', label: 'Vehicle systems', kwh: systems, color: 'var(--ri-home)' },
    { key: 'climate', label: 'Cabin climate', kwh: climate, color: 'var(--ri-office)' },
    { key: 'gear_guard', label: 'Gear Guard', kwh: gearGuard, color: 'var(--ri-acc)' },
    { key: 'outlets', label: 'Outlets & accessories', kwh: outlets, color: 'var(--ri-other)' },
  ];
  if (other > 0.05) {
    raw.push({ key: 'other', label: 'Other / thermal care', kwh: other, color: 'var(--ri-t3)' });
  }

  const categories = raw
    .filter((row) => row.kwh > 0)
    .map((row) => ({
      ...row,
      sharePct: (row.kwh / totalKwh) * 100,
    }));

  return {
    totalKwh,
    sampleCount: chosen.length,
    categories,
  };
}

export function formatParkedRateCopy(day: ParkedDaySummary): string {
  const parts = [`${n1(day.loss)}% over ${n0(day.hours)} h parked`];
  if (day.sleepPct != null) parts.push(`${day.sleepPct}% deep sleep`);
  if (day.avgWatts != null) parts.push(`${n0(day.avgWatts)} W avg`);
  return parts.join(' · ');
}

export { n2 };

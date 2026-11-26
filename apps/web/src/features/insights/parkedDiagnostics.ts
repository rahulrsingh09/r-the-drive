import type { ParkedEnergySample, PhantomDrainPeriod, StateTimelinePeriod } from '@panda/types';
import { cToF, dayKeyOf, n0, n1, n2, sum, type DayKey } from './insightsModel';

/** Days losing at least this much SoC while parked open full diagnostic detail. */
export const SPIKE_LOSS_PCT = 1.5;

export interface ParkedDaySummary {
  day: DayKey;
  hours: number;
  loss: number;
  kwh: number;
  periodCount: number;
  primaryLoss: number | null;
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
  window: ParkedEnergySample['window'] | null;
  windowLabel: string | null;
  sourceAt: string | null;
  durationMinutes: number | null;
  rangeImpactMi: number | null;
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

function periodLoss(period: PhantomDrainPeriod): number | null {
  const loss = period.soc_lost_pct ?? (
    period.soc_start != null && period.soc_end != null ? period.soc_start - period.soc_end : null
  );
  return loss != null && Number.isFinite(loss) ? Math.max(0, loss) : null;
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
    const loss = sum(ordered, (p) => periodLoss(p) ?? 0);
    const kwh = sum(ordered, (p) => (p.energy_drained_kwh != null && p.energy_drained_kwh > 0 ? p.energy_drained_kwh : 0));
    const sleepShare = weightedAverage(ordered, (p) => p.sleep_share_pct);
    const wakeValues = ordered.map((p) => p.wake_count).filter((v): v is number => v != null && Number.isFinite(v));
    const streakValues = ordered
      .map((p) => p.max_sleep_streak_minutes)
      .filter((v): v is number => v != null && Number.isFinite(v));

    // For SoC start -> end, pick the single largest-loss (tie-break longest) parked interval
    // so multi-park days never imply intermediate driving/charging SoC drops were parked loss.
    const primary = [...ordered].sort((a, b) => (
      (periodLoss(b) ?? 0) - (periodLoss(a) ?? 0)
      || (b.duration_hours ?? 0) - (a.duration_hours ?? 0)
    ))[0] ?? ordered[0]!;

    out.set(day, {
      day,
      hours,
      loss,
      kwh,
      periodCount: ordered.length,
      primaryLoss: periodLoss(primary),
      periods: ordered,
      isSpike: loss >= SPIKE_LOSS_PCT,
      sleepPct: sleepShare != null ? Math.round(sleepShare * 100) : null,
      drainPctPerHour: hours > 0 ? loss / hours : null,
      avgWatts: weightedAverage(ordered, (p) => p.avg_power_w),
      socStart: primary.soc_start ?? null,
      socEnd: primary.soc_end ?? null,
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

// VehicleState::Offline occurs whenever cellular/cloud telemetry is asleep (is_online = false),
// alongside explicit PowerState::Sleep. Drive/charging states are excluded from parked breakdowns.
const SLEEP_STATES = new Set(['sleep', 'offline']);
const AWAKE_STATES = new Set(['idle', 'awake', 'standby', 'ready', 'updating']);

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

  // Use server sleep_share_pct fallback only when timeline rows didn't cover known states AND sleepPct > 0;
  // never synthesize "100% Kept awake" just because sleep/offline states were unrecorded.
  if (sleepMinutes === 0 && awakeMinutes === 0 && summary.sleepPct != null && summary.sleepPct > 0 && totalMinutes > 0) {
    sleepMinutes = (summary.sleepPct / 100) * totalMinutes;
    awakeMinutes = Math.max(0, totalMinutes - sleepMinutes);
  }

  const coveredMinutes = sleepMinutes + awakeMinutes;
  const unreportedMinutes = Math.max(0, totalMinutes - coveredMinutes);
  const reliableCoverage = totalMinutes > 0 && coveredMinutes / totalMinutes >= 0.5;
  const sleepPct = reliableCoverage && coveredMinutes > 0
    ? Math.round((sleepMinutes / coveredMinutes) * 100)
    : summary.sleepPct != null && summary.sleepPct > 0
      ? summary.sleepPct
      : null;

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

const WINDOW_PRIORITY: readonly ParkedEnergySample['window'][] = ['24h', 'since_parked', '8h'];
const WINDOW_LABELS: Record<ParkedEnergySample['window'], string> = {
  '24h': '24h',
  since_parked: 'since parked',
  '8h': '8h',
};
const KM_TO_MI = 0.621371;

function sampleEnergyKwh(sample: ParkedEnergySample): number {
  const systems = Math.max(0, sample.vehicle_systems_kwh ?? 0);
  const climate = Math.max(0, sample.climate_kwh ?? 0);
  const gearGuard = Math.max(0, sample.gear_guard_kwh ?? 0);
  const outlets = Math.max(0, sample.outlets_kwh ?? 0);
  const partSum = systems + climate + gearGuard + outlets;
  const reportedTotal = Math.max(0, sample.total_kwh ?? 0);
  return Math.max(partSum, reportedTotal);
}

/**
 * Summarize the latest Rivian-reported parked energy snapshot (`24h`, `since_parked`, or `8h`).
 * `/parked-energy` returns only the freshest snapshot per rolling window (not a 30-day cumulative sum).
 */
export function summarizeParkedEnergy(
  samples: readonly ParkedEnergySample[],
  _timezone?: string,
): ParkedEnergyBreakdown {
  if (!samples.length) {
    return {
      totalKwh: 0,
      sampleCount: 0,
      window: null,
      windowLabel: null,
      sourceAt: null,
      durationMinutes: null,
      rangeImpactMi: null,
      categories: [],
    };
  }

  const latestByWindow = new Map<ParkedEnergySample['window'], ParkedEnergySample>();
  for (const sample of samples) {
    const prev = latestByWindow.get(sample.window);
    if (!prev || Date.parse(sample.source_at) >= Date.parse(prev.source_at)) {
      latestByWindow.set(sample.window, sample);
    }
  }

  const ranked = WINDOW_PRIORITY
    .map((win) => latestByWindow.get(win))
    .filter((sample): sample is ParkedEnergySample => sample != null);
  const chosen = ranked.find((sample) => sampleEnergyKwh(sample) > 0) ?? ranked[0] ?? samples[0]!;

  const systems = Math.max(0, chosen.vehicle_systems_kwh ?? 0);
  const climate = Math.max(0, chosen.climate_kwh ?? 0);
  const gearGuard = Math.max(0, chosen.gear_guard_kwh ?? 0);
  const outlets = Math.max(0, chosen.outlets_kwh ?? 0);
  const partSum = systems + climate + gearGuard + outlets;
  const reportedTotal = Math.max(0, chosen.total_kwh ?? 0);
  const totalKwh = Math.max(partSum, reportedTotal);
  const rangeImpactMi = chosen.total_range_impact_km != null && Number.isFinite(chosen.total_range_impact_km) && chosen.total_range_impact_km > 0
    ? chosen.total_range_impact_km * KM_TO_MI
    : null;
  const baseMeta = {
    window: chosen.window,
    windowLabel: WINDOW_LABELS[chosen.window] ?? chosen.window,
    sourceAt: chosen.source_at,
    durationMinutes: chosen.duration_minutes != null && Number.isFinite(chosen.duration_minutes)
      ? chosen.duration_minutes
      : null,
    rangeImpactMi,
  };
  if (totalKwh <= 0) return { ...baseMeta, totalKwh: 0, sampleCount: 1, categories: [] };

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
    ...baseMeta,
    totalKwh,
    sampleCount: 1,
    categories,
  };
}

export function formatParkedRateCopy(day: ParkedDaySummary): string {
  const parts = [`${n1(day.loss)}% over ${n0(day.hours)} h parked`];
  if (day.sleepPct != null && day.sleepPct > 0) parts.push(`${day.sleepPct}% deep sleep`);
  if (day.avgWatts != null) parts.push(`${n0(day.avgWatts)} W avg`);
  return parts.join(' · ');
}

export { n2 };

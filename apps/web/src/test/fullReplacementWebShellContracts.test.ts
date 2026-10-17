import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PhantomDrainPeriod } from '@panda/types';
import { summarizePhantomDrainPeriods, buildPhantomDrainSummaryCards } from '../components/dashboard/phantomDrainSummary';
import { getFeedHealthIssue, resolveVehicleOnlineState, shouldEmitFeedHealthToast } from '../components/layout/vehicleConnection';
import { loadDashboardTimeframe, serializeDashboardTimeframe, timeframeToQuery, presetToRange, parseTimeframeInput } from '../lib/dates';
import { fetchLatestStableRelease, isNewerRelease } from '../lib/releaseCheck';

beforeEach(() => { sessionStorage.clear(); localStorage.clear(); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

function period(values: Partial<PhantomDrainPeriod>): PhantomDrainPeriod {
  return values as PhantomDrainPeriod;
}

describe('web shell aggregate contracts', () => {
  it('uses separate finite duration weights and includes every finite loss in the rate numerator', () => {
    const summary = summarizePhantomDrainPeriods([
      period({ duration_hours: 2, sleep_share_pct: 0.5, state_coverage_pct: 0.9, soc_lost_pct: 1, drain_pct_per_hour: 0.5, energy_drained_kwh: 2 }),
      period({ duration_hours: 6, sleep_share_pct: null, state_coverage_pct: 0.3, soc_lost_pct: 3, drain_pct_per_hour: 0.8, energy_drained_kwh: -1 }),
      period({ duration_hours: NaN, sleep_share_pct: 1, state_coverage_pct: Infinity, soc_lost_pct: 4, drain_pct_per_hour: Infinity, energy_drained_kwh: NaN }),
    ]);
    expect(summary).toEqual({ maxDrainPctPerHour: 0.8, avgSleepPct: 0.5, avgStateCoveragePct: 0.44999999999999996, totalEnergyDrainedKwh: 1, avgDrainPctPerHour: 1 });
    expect(buildPhantomDrainSummaryCards(summary).map(card => [card.key, card.value])).toEqual([
      ['max-drain-rate', '0.80% / h'], ['avg-sleep', '50.0%'], ['total-energy-drained', '1.0 kWh'], ['avg-drain-per-hour', '1.00% / h'],
    ]);
  });

  it('retains negative weights and the energy infinity contract', () => {
    expect(summarizePhantomDrainPeriods([period({ duration_hours: -1, sleep_share_pct: 0.8, soc_lost_pct: 1, energy_drained_kwh: Infinity })])).toEqual({
      maxDrainPctPerHour: null, avgSleepPct: null, avgStateCoveragePct: null, totalEnergyDrainedKwh: Infinity, avgDrainPctPerHour: null,
    });
  });
});

describe('web shell time and persisted ranges', () => {
  it.each([
    [{ kind: 'lifetime', preset: '7d' }, { kind: 'lifetime' }],
    [{ kind: 'preset', preset: 'lifetime' }, { kind: 'lifetime' }],
    [{ kind: 'other', preset: '7d' }, { kind: 'preset', preset: '7d' }],
    [{ kind: 'custom', from: 'bad', to: 'bad', preset: '7d' }, null],
    [{ kind: 'preset', preset: 'invalid', from: '2026-10-02', to: '2026-10-01' }, { kind: 'custom', from: new Date('2026-10-01'), to: new Date('2026-10-02') }],
    [null, null], [[], null],
  ])('decodes current and legacy payload %j', (payload, expected) => {
    sessionStorage.setItem('rm-dashboard-timeframe', JSON.stringify(payload));
    expect(loadDashboardTimeframe()).toEqual(expected);
  });

  it('normalizes custom bounds when persisted but labels preset queries by preset', () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date('2026-10-17T12:34:56Z'));
    expect(presetToRange('6h')).toEqual({ from: new Date('2026-10-17T06:34:56Z'), to: new Date('2026-10-17T12:34:56Z') });
    expect(timeframeToQuery({ kind: 'preset', preset: '6h' }).cacheKey).toBe('preset:6h');
    expect(serializeDashboardTimeframe({ kind: 'custom', from: new Date('2026-10-02T00:00:00Z'), to: new Date('2026-10-01T00:00:00Z') })).toEqual({ kind: 'custom', from: '2026-10-01T00:00:00Z', to: '2026-10-02T00:00:00Z' });
    expect(parseTimeframeInput(' ', new Date())).toBeNull();
    expect(parseTimeframeInput('nonsense', new Date())).toBeNull();
  });
});

describe('web shell connection and releases', () => {
  it('prioritizes auth trouble but browser readiness precedes server feed health', () => {
    const issue = getFeedHealthIssue({ auth_state: 'needs_reauth', auth_reason_code: '', worker_health: 'error', worker_health_msg: '' });
    expect(issue).toEqual({ key: 'needs_reauth:', title: 'Rivian feed disconnected', message: '' });
    const input = { liveVehicleId: 'synthetic', vehicleSelectionReady: true, connectionState: 'connected', connected: true, feedHealthIssue: issue };
    expect(resolveVehicleOnlineState(input)).toBe('unhealthy');
    expect(resolveVehicleOnlineState({ ...input, connected: false })).toBe('connecting');
    expect(resolveVehicleOnlineState({ ...input, liveVehicleId: null })).toBe('offline');
    expect(getFeedHealthIssue({ telemetry_stale: true, worker_health: 'healthy' })).toBeNull();
  });

  it('retains cooldown behavior for missing timestamps, future clocks, and exact boundaries', () => {
    expect(shouldEmitFeedHealthToast(localStorage, 'health', 100)).toBe(false);
    expect(shouldEmitFeedHealthToast(localStorage, 'health', 900_000)).toBe(true);
    expect(shouldEmitFeedHealthToast(localStorage, 'health', 800_000)).toBe(false);
    expect(shouldEmitFeedHealthToast(localStorage, 'health', 1_800_000)).toBe(true);
  });

  it.each(['2026.00.1', '2026.13.1', '2026.10.9007199254740992', '2026.10.1-rc', '2026.10.1+dev.bad'])('rejects unsupported release %s', version => {
    expect(isNewerRelease(version, '2026.10.2')).toBeNull();
  });

  it('cleans the release timeout after invalid payload and aborts hung fetches without credentials', async () => {
    vi.useFakeTimers();
    const payload = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ tag_name: '2026.10.1', draft: 'false', prerelease: false }) });
    await expect(fetchLatestStableRelease(payload)).rejects.toThrow('invalid stable release response');
    expect(vi.getTimerCount()).toBe(0);
    const fetcher = vi.fn((_url, options) => new Promise<Response>((_resolve, reject) => options.signal.addEventListener('abort', () => reject(new Error('aborted')))));
    const promise = fetchLatestStableRelease(fetcher as typeof fetch);
    const rejected = expect(promise).rejects.toThrow('aborted');
    await vi.advanceTimersByTimeAsync(15_000);
    await rejected;
    expect(fetcher.mock.calls[0]?.[1]).toMatchObject({ credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store' });
    expect(vi.getTimerCount()).toBe(0);
  });
});

import { describe, expect, it } from 'vitest';
import type { ParkedEnergySample, PhantomDrainPeriod } from '@panda/types';
import {
  buildParkedDaySummaries,
  buildParkedOnlyTimeline,
  pickDefaultParkedDay,
  summarizeParkedEnergy,
} from './parkedDiagnostics';

function makePeriod(overrides: Partial<PhantomDrainPeriod>): PhantomDrainPeriod {
  return {
    period_start: '2026-10-14T08:00:00Z',
    period_end: '2026-10-14T20:00:00Z',
    duration_hours: 12,
    sleep_share_pct: 0.9,
    state_coverage_pct: 1,
    soc_start: 70,
    soc_end: 69.5,
    soc_lost_pct: 0.5,
    drain_pct_per_hour: 0.04,
    range_start_mi: 240,
    range_end_mi: 238,
    range_lost_mi: 2,
    range_lost_per_hour_mi: 0.16,
    energy_drained_kwh: 0.45,
    avg_power_w: 38,
    has_reduced_range: false,
    validation_status: 'validated',
    validation_reason: null,
    sample_count: 10,
    start_sample_at: '2026-10-14T08:00:00Z',
    end_sample_at: '2026-10-14T20:00:00Z',
    movement_detected: false,
    overlaps_trip: false,
    overlaps_charge: false,
    ...overrides,
  };
}

describe('parkedDiagnostics', () => {
  it('picks newest spike day by default and clips timeline strictly to parked states', () => {
    const summaries = buildParkedDaySummaries([
      makePeriod({
        period_start: '2026-10-12T08:00:00Z',
        period_end: '2026-10-12T20:00:00Z',
        soc_lost_pct: 2.5,
        ota_active: true,
        gear_guard_active: true,
      }),
      makePeriod({
        period_start: '2026-10-14T08:00:00Z',
        period_end: '2026-10-14T20:00:00Z',
        soc_lost_pct: 0.4,
      }),
    ], 'America/Los_Angeles');

    expect(pickDefaultParkedDay(['2026-10-12', '2026-10-13', '2026-10-14'], summaries)).toBe('2026-10-12');

    const spike = summaries.get('2026-10-12')!;
    const timeline = buildParkedOnlyTimeline(spike, [
      { id: 1, state: 'driving', started_at: '2026-10-12T07:00:00Z', ended_at: '2026-10-12T10:00:00Z', duration_seconds: 10800 },
      { id: 2, state: 'sleep', started_at: '2026-10-12T08:00:00Z', ended_at: '2026-10-12T16:00:00Z', duration_seconds: 28800 },
      { id: 3, state: 'awake', started_at: '2026-10-12T16:00:00Z', ended_at: '2026-10-12T22:00:00Z', duration_seconds: 21600 },
    ]);

    expect(timeline.slices).toHaveLength(2);
    expect(timeline.sleepMinutes).toBeCloseTo(480);
    expect(timeline.awakeMinutes).toBeCloseTo(240);
    expect(timeline.awakeTriggers).toEqual(['Gear Guard', 'OTA update']);
  });

  it('deduplicates 24h parked energy samples per day and computes shares', () => {
    const samples: ParkedEnergySample[] = [
      {
        window: '24h',
        source_at: '2026-10-14T10:00:00Z',
        received_at: '2026-10-14T10:01:00Z',
        parked_started_at: '2026-10-14T02:00:00Z',
        duration_minutes: 480,
        total_kwh: 1,
        vehicle_systems_kwh: 0.5,
        climate_kwh: 0.5,
        gear_guard_kwh: 0,
        outlets_kwh: 0,
        total_range_impact_km: null,
        vehicle_systems_range_impact_km: null,
        outlets_range_impact_km: null,
        climate_range_impact_km: null,
        gear_guard_range_impact_km: null,
      },
      {
        window: '24h',
        source_at: '2026-10-14T18:00:00Z',
        received_at: '2026-10-14T18:01:00Z',
        parked_started_at: '2026-10-14T02:00:00Z',
        duration_minutes: 960,
        total_kwh: 2.0,
        vehicle_systems_kwh: 1.0,
        climate_kwh: 0.5,
        gear_guard_kwh: 0.5,
        outlets_kwh: 0,
        total_range_impact_km: null,
        vehicle_systems_range_impact_km: null,
        outlets_range_impact_km: null,
        climate_range_impact_km: null,
        gear_guard_range_impact_km: null,
      },
    ];
    const breakdown = summarizeParkedEnergy(samples, 'America/Los_Angeles');
    expect(breakdown.sampleCount).toBe(1);
    expect(breakdown.totalKwh).toBeCloseTo(2);
    expect(breakdown.categories.map((c) => [c.key, c.kwh])).toEqual([
      ['vehicle_systems', 1],
      ['climate', 0.5],
      ['gear_guard', 0.5],
    ]);
  });

  it('treats offline vehicle state as deep sleep and avoids false 100% awake on unknown gaps', () => {
    const summaries = buildParkedDaySummaries([
      makePeriod({
        period_start: '2026-10-14T08:00:00Z',
        period_end: '2026-10-14T16:00:00Z',
        duration_hours: 8,
        sleep_share_pct: 0,
        soc_start: 70,
        soc_end: 69.5,
        soc_lost_pct: 0.5,
      }),
      makePeriod({
        period_start: '2026-10-14T20:00:00Z',
        period_end: '2026-10-15T04:00:00Z',
        duration_hours: 8,
        sleep_share_pct: 0.9,
        soc_start: 68,
        soc_end: 66.5,
        soc_lost_pct: 1.5,
      }),
    ], 'America/Los_Angeles');

    const day = summaries.get('2026-10-14')!;
    expect(day.periodCount).toBe(2);
    expect(day.socStart).toBe(68);
    expect(day.socEnd).toBe(66.5);
    expect(day.primaryLoss).toBe(1.5);

    const timeline = buildParkedOnlyTimeline(day, [
      { id: 1, state: 'offline', started_at: '2026-10-14T08:00:00Z', ended_at: '2026-10-14T16:00:00Z', duration_seconds: 28800 },
      { id: 2, state: 'sleep', started_at: '2026-10-14T20:00:00Z', ended_at: '2026-10-15T04:00:00Z', duration_seconds: 28800 },
    ]);
    expect(timeline.sleepPct).toBe(100);
    expect(timeline.awakeMinutes).toBe(0);

    const unreported = buildParkedOnlyTimeline(
      { ...day, sleepPct: 0 },
      [{ id: 3, state: 'unknown', started_at: '2026-10-14T08:00:00Z', ended_at: '2026-10-15T04:00:00Z', duration_seconds: 57600 }],
    );
    expect(unreported.awakeMinutes).toBe(0);
    expect(unreported.sleepPct).toBeNull();
  });
});

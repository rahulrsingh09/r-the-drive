import React from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { PhantomDrainPeriodCard, type PhantomDrainRow } from '@panda/ui/tables';
import { setUnitSystem } from '@panda/ui/lib/utils';

const period: PhantomDrainRow = {
  period_start: '2026-10-01T00:00:00Z', period_end: '2026-10-01T08:00:00Z',
  duration_hours: 8, sleep_share_pct: null, state_coverage_pct: .3,
  soc_start: 72, soc_end: 71.4, soc_lost_pct: .6, drain_pct_per_hour: .075,
  range_start_mi: 218, range_end_mi: 216, range_lost_mi: 2, range_lost_per_hour_mi: .25,
  energy_drained_kwh: .6, avg_power_w: 75, has_reduced_range: true,
  validation_status: 'validated', validation_reason: null, sample_count: 12,
  start_sample_at: null, end_sample_at: null, movement_detected: false, overlaps_trip: false, overlaps_charge: false,
};

afterEach(() => { cleanup(); setUnitSystem('imperial'); });

describe('mobile parked-period readings', () => {
  it('preserves fractional range loss and distinguishes missing sleep from zero', () => {
    setUnitSystem('imperial');
    render(<PhantomDrainPeriodCard period={period} />);
    expect(screen.getByText('0.25 mi/h')).toBeInTheDocument();
    expect(screen.getByText('Unknown')).toBeInTheDocument();
    expect(screen.getByText('30%')).toBeInTheDocument();
    expect(screen.getByText(/Sleep is unknown/)).toBeInTheDocument();
    expect(screen.getByText(/Reduced-range conditions/)).toBeInTheDocument();
  });
  it('uses selected distance units and retains a genuine zero sleep reading', () => {
    setUnitSystem('metric');
    render(<PhantomDrainPeriodCard period={{ ...period, sleep_share_pct: 0 }} />);
    expect(screen.getByText('0.40 km/h')).toBeInTheDocument();
    expect(screen.getByText('0%')).toBeInTheDocument();
    expect(screen.queryByText('Unknown')).not.toBeInTheDocument();
  });
});

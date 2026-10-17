import { describe, expect, it } from 'vitest';
import {
  buildChargeCurvePlot,
  distributeChargeCurveSoc,
  normalizeChartSettingsMap,
  getManualAxisRange,
} from '../../../../packages/dashboards/src/widgets/chart/DashboardChartWidget';
import {
  scheduleToForm,
  formToInput as chargingInput,
  formatDays,
} from '../../../../packages/dashboards/src/widgets/charging/ChargingScheduleEditorWidget';
import {
  formToInput as departureInput,
  formatOccurrence,
} from '../../../../packages/dashboards/src/widgets/charging/DepartureSchedulesWidget';
import { buildPhantomDrainDailySeries } from '../../../../packages/dashboards/src/widgets/chart/PhantomDrainChart';
import type { ChargeCurveAnalysisPoint, PhantomDrainPeriod } from '@panda/types';
import type { DepartureSchedule } from '@panda/hooks';

describe('dashboard widget algorithms', () => {
  it('keeps canonical settings over legacy aliases and rejects non-finite axis bounds', () => {
    expect(
      normalizeChartSettingsMap({
        'range-history': { timeFilter: 'raw', axes: { y: { mode: 'manual', min: 0, max: 200 } } },
        'soc-history': {
          smoothness: 'straight',
          axes: { y: { min: NaN, max: Infinity, mode: 'auto' }, garbage: { min: 3 } },
        },
        empty: {},
        bad: 'ignored',
      })
    ).toEqual({ 'soc-history': { smoothness: 'straight', axes: { y: { mode: 'auto' } } } });
    expect(getManualAxisRange({ mode: 'manual', min: 5, max: 5 })).toBeUndefined();
    expect(getManualAxisRange({ mode: 'manual', min: 5, max: 10 })).toEqual([5, 10]);
    expect(getManualAxisRange({ mode: 'auto', min: 5, max: 10 })).toBeUndefined();
  });
  it('keeps historical curve samples out of the verified regression', () => {
    const observed = Array.from({ length: 5 }, (_, i) => ({
      soc_pct: 40 + i,
      charge_rate_kw: 100 + 10 * i,
      charger_type: 'dc',
      sample_source: 'telemetry',
      power_method: 'recorded',
    })) as ChargeCurveAnalysisPoint[];
    const rows = [
      ...observed,
      { ...observed[0]!, charge_rate_kw: 900, sample_source: 'rivian_charge_curve_points' },
      { ...observed[0]!, charger_type: 'ac' },
    ] as ChargeCurveAnalysisPoint[];
    const plotted = buildChargeCurvePlot(rows, 'observed');
    expect(plotted.hasEstimatedHistory).toBe(true);
    expect(plotted.rows.filter((row) => row.observedKw != null)).toHaveLength(5);
    expect(plotted.rows.filter((row) => row.estimatedKw != null)).toHaveLength(1);
    const summary = plotted.rows.filter((row) => row.summaryKw != null);
    expect(summary.map((row) => row.socPct)).toEqual([40, 41, 42, 43, 44]);
    summary.forEach((row) => expect(row.summaryKw).toBeCloseTo(100 + (row.socPct - 40) * 10, 8));
    expect(buildChargeCurvePlot([], 'off')).toEqual({ rows: [], hasEstimatedHistory: false });
  });
  it('spreads coincident SoC samples symmetrically without mutating input', () => {
    const rows = [3, 1, 1, 1].map((socPct) => ({
      socPct,
      plotSoc: socPct,
      observedKw: 1,
      observedDetail: null,
      estimatedKw: null,
      estimatedDetail: null,
      summaryKw: null,
      summaryDetail: null,
    }));
    const result = distributeChargeCurveSoc(rows);
    expect(result.map((row) => row.plotSoc)).toEqual([0.96, 1, 1.04, 3]);
    expect(rows.map((row) => row.socPct)).toEqual([3, 1, 1, 1]);
  });
  it('splits validated drain over local midnights and preserves proportional loss', () => {
    const start = new Date(2026, 0, 2, 22).toISOString(),
      end = new Date(2026, 0, 3, 2).toISOString();
    const period = {
      period_start: start,
      period_end: end,
      soc_lost_pct: 4,
      validation_status: 'validated',
    } as PhantomDrainPeriod;
    const rows = buildPhantomDrainDailySeries([
      period,
      { ...period, validation_status: 'rejected' } as unknown as PhantomDrainPeriod,
    ]);
    expect(rows).toHaveLength(2);
    expect(
      rows.map((row) => [row.socLost, row.parkedHours, row.drainRate, row.periodCount])
    ).toEqual([
      [2, 2, 1, 1],
      [2, 2, 1, 1],
    ]);
  });
  it('preserves schedule parse, rounding, empty and weekday wire contracts', () => {
    expect(
      scheduleToForm({
        enabled: true,
        start_time_minutes: 1500,
        duration_minutes: 95,
        amperage: 0,
        week_days: null,
      })
    ).toEqual({
      enabled: true,
      startTime: '01:00',
      durationHours: '1.6',
      amperage: '0',
      weekDays: [],
    });
    expect(
      chargingInput({
        enabled: true,
        startTime: '22:30',
        durationHours: '1.25',
        amperage: 'bad',
        weekDays: [],
      })
    ).toEqual({
      enabled: true,
      start_time_minutes: 1350,
      duration_minutes: 75,
      amperage: null,
      week_days: null,
    });
    expect(
      chargingInput({
        enabled: false,
        startTime: '1:2:bad',
        durationHours: '',
        amperage: '',
        weekDays: ['Sunday'],
      })
    ).toEqual({
      enabled: false,
      start_time_minutes: null,
      duration_minutes: null,
      amperage: null,
      week_days: ['Sunday'],
    });
    expect(formatDays(['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday'])).toBe('Weekdays');
    expect(formatDays(['Sunday', 'Saturday'])).toBe('Weekends');
    expect(
      departureInput({
        name: '',
        enabled: true,
        days: [],
        time: '',
        seatFlHeat: 0,
        seatFrHeat: 2,
        cabinTempC: 'not a number',
        defrost: true,
      })
    ).toEqual({
      name: null,
      enabled: true,
      occurrence: { type: 'RepeatsWeekly', days: [], time_minutes: 0 },
      comfort_settings: { seat_fr_heat: 2, defrost: true },
    });
    expect(
      formatOccurrence({ occurrence: { days: ['Friday'], time_minutes: 450 } } as DepartureSchedule)
    ).toBe('Fri at 07:30');
  });
});

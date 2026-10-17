import { describe, expect, it } from 'vitest';
import type { ChargeSessionRow, TripRow } from '@panda/ui/tables';
import type { DepartureSchedule } from '@panda/hooks';
import type { ChartRecord } from '@panda/types';
import { getChartDefinition } from '../../../../packages/dashboards/src/charts/catalog';
import { widgetChartCapabilities } from '../../../../packages/dashboards/src/widgets/chart/chartCapabilities';
import { scatterBuckets, scatterPalette } from '../../../../packages/dashboards/src/widgets/chart/scatterProjection';
import { chargingScheduleDraft, departurePresentation } from '../../../../packages/dashboards/src/widgets/charging/schedulePresentation';
import { sensorSamples, sensorSourceLoading } from '../../../../packages/dashboards/src/widgets/sensor/sensorReadings';
import { pageNavigation, tripBatchActions } from '../../../../packages/dashboards/src/widgets/table/tableControls';
import { chargingSessionValues, selectedTripRows, tripTagBatch } from '../../../../packages/dashboards/src/widgets/table/tableProjection';
import { computeTripStat } from '../../../../packages/dashboards/src/widgets/table/tripStatMetrics';

describe('dashboard widget recovery boundaries', () => {
  it('keeps scatter boundaries, invalid exclusions, flat ranges and rounded color channels', () => {
    expect(scatterBuckets([null, NaN, Infinity, 0, 1, 2, 3, 4, 5, 6], 6)).toEqual([
      [3], [4], [5], [6], [7], [8, 9],
    ]);
    expect(scatterBuckets([10, null, 10], 6)).toEqual([[], [], [], [], [], [0, 2]]);
    expect(scatterPalette('#102030', '#718293', 3)).toEqual(['#102030', '#415162', '#718293']);
  });

  it('keeps bundled capability policy ahead of user-defined axis and series declarations', () => {
    const bundled = getChartDefinition('soc-history')!;
    const record = { config: {
      ...bundled.config,
      axes: { x: { label: '', unit: '', domain: { mode: 'auto' } }, y: { domain: { mode: 'auto' } } },
      x: { ...bundled.config.x, kind: 'number' },
      series: [],
      timeframe: { mode: 'lifetime' },
    } } as unknown as ChartRecord;
    expect(widgetChartCapabilities(bundled, record)).toEqual(widgetChartCapabilities(bundled));
    expect(widgetChartCapabilities(undefined, record)).toEqual({
      timeFilter: false, smoothness: false, xDomainSource: 'chart-local',
      axes: { x: { label: '' }, y: { label: 'Y axis' } },
    });
    expect(widgetChartCapabilities()).toEqual({ timeFilter: false, axes: {}, xDomainSource: 'dashboard-timeframe' });
  });

  it('retains zero schedule fields and the exact seat/cabin/defrost order', () => {
    expect(chargingScheduleDraft({
      enabled: false, start_time_minutes: 0, duration_minutes: 0, amperage: 0, week_days: [],
    })).toEqual({ enabled: false, startTime: '00:00', durationHours: '0', amperage: '0', weekDays: [] });
    const schedule = {
      occurrence: { days: ['Friday', 'Monday'], time_minutes: 450 },
      comfort_settings: { seat_fl_heat: 1, seat_fr_heat: 3, cabin_temp_c: 0, defrost: true },
    } as DepartureSchedule;
    const result = departurePresentation(schedule);
    expect(result.occurrence).toBe('Fri, Mon at 07:30');
    expect(result.pills.map((pill) => pill.label)).toEqual(['FL Seat: Low', 'FR Seat: High', 'Cabin: 0°C', 'Defrost']);
    expect(departurePresentation({} as DepartureSchedule).occurrence).toBe('Not configured');
  });

  it('preserves sample identities, latest fallback, and source-specific loading', () => {
    const point = { ts: 'synthetic-time', value: 0 };
    const samples = sensorSamples([{ value: null }, { value: Infinity }, point]);
    expect(samples.available).toBe(true);
    expect(samples.sprite(99, 'now')[0]).toBe(point);
    expect(sensorSamples([]).sprite(0, 'now')).toEqual([{ value: 0, ts: 'now' }]);
    expect(sensorSamples([]).sprite(NaN, 'now')).toEqual([]);
    const pending = { health: false, charging: false, status: true, needsStatus: true };
    expect(sensorSourceLoading('batteryHealth', false, false, pending)).toBe(true);
    expect(sensorSourceLoading('batteryHealth', true, false, pending)).toBe(false);
    expect(sensorSourceLoading('metric', true, true, pending)).toBe(false);
  });

  it('preserves page boundaries and add/replace/clear command distinctions', () => {
    expect(pageNavigation(1, { total: 0, per_page: 15 })).toEqual({
      pages: 1, total: 0, previous: { page: 0, disabled: true }, next: { page: 2, disabled: true },
    });
    expect(pageNavigation(2, { total: 31, per_page: 15 }).next).toEqual({ page: 3, disabled: false });
    const actions = tripBatchActions(2, ['common', 'new'], ['common'], null, false);
    expect(actions.map(({ key, mode, tags, disabled }) => ({ key, mode, tags, disabled }))).toEqual([
      { key: 'clear', mode: 'replace', tags: [], disabled: false },
      { key: 'add', mode: 'add', tags: ['new'], disabled: false },
      { key: 'save', mode: 'replace', tags: ['common', 'new'], disabled: false },
    ]);
    expect(tripBatchActions(2, [], [], 'add', false).map((action) => [action.disabled, action.loading])).toEqual([
      [true, false], [true, true], [true, false],
    ]);
  });

  it('retains cross-page selection order, registry priority, and missing-row tag exclusion', () => {
    const a = { id: 'a', tags: [{ id: 'shared' }, { id: 'other' }] } as TripRow;
    const b = { id: 'b', tags: [{ id: 'shared' }] } as TripRow;
    expect(selectedTripRows(['b', 'a'], { a }, [{ ...a, distance_mi: 99 }, b])).toEqual([b, a]);
    expect(tripTagBatch(['a', 'b'], { a }, [b])).toMatchObject({ commonTagIds: ['shared'], hasMixedTags: true });
    expect(tripTagBatch(['a', 'missing'], { a }, [b])).toMatchObject({ commonTagIds: [], hasMixedTags: false, allSelectedTripsLoaded: false });
  });

  it('keeps live zero readings ahead of recorded values, and finished sessions ignore live data', () => {
    const session = {
      ended_at: null, charger_type: null, network_vendor: ' Tesla ', peak_power_kw: 50,
      energy_added_kwh: 10, duration_min: 60, soc_end: 80, range_added_km: 100,
      live_total_charged_kwh: 0, live_time_elapsed_seconds: 0, live_power_kw: 0,
      live_soc_pct: 0, live_range_added_km: 0,
    } as ChargeSessionRow;
    expect(chargingSessionValues(session)).toEqual({ active: true, energy: 0, duration: 0, power: 0, socEnd: 0, range: 0, type: 'dc' });
    expect(chargingSessionValues({ ...session, ended_at: 'finished' })).toEqual({
      active: false, energy: 10, duration: 60, power: 50, socEnd: 80, range: 100, type: 'dc',
    });
  });

  it('preserves aggregation order and IEEE nonfinite results without inventing measurements', () => {
    const trips = [1e16, 1, -1e16].map((distance_mi) => ({ distance_mi, duration_min: 0, efficiency_wh_mi: null })) as TripRow[];
    expect(computeTripStat('miles', trips)).toBe(0);
    expect(computeTripStat('efficiency', trips)).toBeNull();
    expect(computeTripStat('duration', [{ ...trips[0]!, duration_min: NaN }])).toBeNaN();
  });
});

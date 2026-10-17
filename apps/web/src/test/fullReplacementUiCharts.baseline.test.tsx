import { EfficiencyVsTempChart } from '../../../../packages/ui/src/charts/EfficiencyVsTempChart';
import { EfficiencyTrendChart } from '../../../../packages/ui/src/charts/EfficiencyTrendChart';
import { DegradationChart } from '../../../../packages/ui/src/charts/DegradationChart';
import React from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createHash } from 'node:crypto';
import * as Charts from '../../../../packages/ui/src/charts';
import { filterTimeSeriesValues, bucketTimeSeriesValues } from '../../../../packages/ui/src/charts/timeFilter';
import { clampedControlPoints, splitCurveSegments } from '../../../../packages/ui/src/charts/curveSmoothness';
import { selectAdaptiveAxisLabelIndices, selectValueLabelIndices } from '../../../../packages/ui/src/charts/chartLabelLayout';

let width = 960;
const operations: unknown[][] = [];
const normalize = (text: string) => text.replace(/«r[^»]*»/g, 'react-id').replace(/recharts\d+/g, 'recharts-id');
const digest = (text: string) => createHash('sha256').update(text).digest('hex');
const daily = [0, 1, 2, 3].map(i => ({ day_local: `2024-01-0${i + 1}`, day_start: `2024-01-0${i + 1}T00:00:00Z`, total_energy_kwh: [32, 0, 64, 8][i]!, session_count: 2 }));
const dailySessions = daily.flatMap((d, i) => ['ac_l2', 'dcfc', null].map((charger_type, j) => ({ ...d, session_id: `${i}-${j}`, started_at: d.day_start, energy_added_kwh: j ? 12 : 20, cost_usd: j ? null : 2, charger_type, location_name: null })));
const trip = [0, 30, 60, 120].map((elapsed_s, i) => ({ elapsed_s, speed_mph: [0, null, 40, 15][i]!, power_kw: i * 12, regen_kw: i ? null : 0, altitude_m: 100 + i * 6, outside_temp_c: 10 + i, cabin_temp_c: 21, driver_temp_c: i ? 22 : null, hvac_active: i % 2 === 0, tire_fl_psi: 40 + i, tire_fr_psi: 42, tire_rl_psi: i ? null : 40, tire_rr_psi: 41 }));
const samples = trip.map((p, i) => ({ ...p, ts: `2024-01-0${i + 1}T00:00:00Z`, value: i ? p.altitude_m : null }));
const fixtures: Record<string, React.ReactElement> = {
  daily: <Charts.DailyChargeSessionsChart daily={daily} dailySessions={dailySessions} />,
  energy: <Charts.DailyEnergyBarChart daily={daily} />,
  distribution: <Charts.ChargeSessionDistributionChart bands={[{ label: 'AC', count: 3, averageRateKw: 12 }, { label: 'DC', count: 1, averageRateKw: 120 }, { label: 'None', count: 0, averageRateKw: null }]} />,
  pills: <Charts.EfficiencyPillBarChart valueUnit="Wh/mi" data={[{ label: 'Drive', value: 320, count: 4, distance: 13, coverage: 0.8 }, { label: 'Unknown', value: 420, speed: 45, tone: 'neutral' }]} />,
  charge: <Charts.ChargeCurveChart data={[{ soc: 10, power_kw: 150 }, { soc: 50, power_kw: 80 }, { soc: 90, power_kw: 20 }]} />,
  degradation: <DegradationChart data={samples.map(p => ({ ...p, usable_kwh: 120, rated_kwh: 135, capacity_pct: 94 }))} />,
  trend: <EfficiencyTrendChart showBrush data={samples.map((p, i) => ({ ...p, trip_efficiency_wh_mi: 300 + i * 10, rolling_24h_wh_mi: i ? 340 : null }))} />,
  temperature: <EfficiencyVsTempChart data={[{ temp_c_low: 10, temp_c_high: 20, avg_efficiency_wh_mi: 300, trip_count: 3 }, { temp_c_low: 20, temp_c_high: 30, avg_efficiency_wh_mi: null, trip_count: 0 }]} />,
  elevation: <Charts.ElevationProfileChart data={samples} />,
  speed: <Charts.SpeedProfileChart data={trip.map(p => ({ elapsed_s: p.elapsed_s, speed_mph: p.speed_mph ?? 0 }))} />,
  drive: <Charts.TripDriveChart data={trip} activeElapsedS={30} />,
  tripElevation: <Charts.TripElevationChart data={trip} activeElapsedS={30} />,
  tripTemperature: <Charts.TripTemperatureChart data={trip} activeElapsedS={30} />,
  tripTires: <Charts.TripTirePressureChart data={trip} activeElapsedS={30} />,
  histogram: <Charts.SpeedHistogramChart bins={[{ label: '0-10', min: 0, max: 10, count: 3, duration_seconds: 60, sample_elapsed_s: 0 }, { label: '10-20', min: 10, max: 20, count: 1, duration_seconds: 20, sample_elapsed_s: 30 }]} />,
  rich: <Charts.RichTimeSeriesChart points={samples} series={[{ key: 'a', label: 'Power', values: [1, null, 3, 4] }, { key: 'b', label: 'Capacity', values: [20, 30, 40, 30], yScale: 'y2' }]} yUnit="kW" yRightUnit="%" connectGaps intervals={[{ id: 'trip', start: samples[0]!.ts, end: samples[2]!.ts, label: 'Trip' }]} referenceLines={[{ value: 2, label: 'Target' }]} />,
  sparkline: <Charts.MiniSparkline data={samples} type="area" timeFilter="raw" />,
  bars: <Charts.MiniSparkline data={samples} type="bar" timeFilter="24h" />,
};

beforeEach(() => {
  operations.length = 0;
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(() => width);
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(() => ({ x: 0, y: 0, left: 0, top: 0, right: width, bottom: 280, width, height: 280, toJSON: () => ({}) }));
  const originalContext = HTMLCanvasElement.prototype.getContext;
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement, ...args) {
    const context = originalContext.apply(this, args as never) as CanvasRenderingContext2D;
    return new Proxy(context, { get(target, key) { const value = Reflect.get(target, key); return typeof value === 'function' ? (...values: unknown[]) => { operations.push([String(key), ...values.map(v => typeof v === 'object' ? String(v) : v)]); return value.apply(target, values); } : value; } });
  });
  vi.stubGlobal('ResizeObserver', class { constructor(private cb: ResizeObserverCallback) {} observe(target: Element) { this.cb([{ target, contentRect: { width, height: 280 } } as ResizeObserverEntry], this as unknown as ResizeObserver); } disconnect() {} unobserve() {} });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('canonical chart characterization', () => {
  for (const viewport of [960, 375]) {
    for (const [name, element] of Object.entries(fixtures)) {
      it(`${name} preserves DOM and SVG at ${viewport}px`, async () => {
        width = viewport;
        const view = render(element);
        await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
        const markup = normalize(view.container.innerHTML);
        expect(markup).toMatchSnapshot();
        if (name === 'sparkline' || name === 'bars') expect(operations).toMatchSnapshot('canvas commands');
        if (name === 'daily') {
          const button = view.getAllByRole('button')[0]!;
          fireEvent.mouseEnter(button, { clientX: 100, clientY: 50 });
          expect(view.getByRole('tooltip').textContent).toMatchSnapshot('hover');
        }
        if (name === 'pills') {
          fireEvent.click(view.getAllByRole('button')[0]!);
          expect(view.getAllByRole('button')[0]).toHaveAttribute('aria-pressed', 'true');
        }
      });
    }
  }
  it('characterizes irregular filters and geometric selection', () => {
    const cases = Array.from({ length: 30 }, (_, seed) => {
      const times = Array.from({ length: 12 }, (_, i) => (i === 6 ? 2 : i) * (seed + 1) * 120000);
      const values = times.map((_, i) => (i + seed) % 5 === 0 ? null : Math.sin(i + seed) * 10);
      const points = values.map((y, x) => y == null ? null : ({ x, y }));
      const segments = splitCurveSegments(points);
      return { filtered: filterTimeSeriesValues(times, values, '15m'), buckets: bucketTimeSeriesValues(times, values, '15m'), segments, controls: segments.flatMap(s => s.slice(1).map((_, i) => clampedControlPoints(s, i, 'gentle'))), labels: selectAdaptiveAxisLabelIndices(times.map(String), times.map((_, i) => i * 12)), valueLabels: selectValueLabelIndices(times.map((_, i) => ({ index: i, x: i * 4, y: i % 2, width: 12, value: i % 3 }))) };
    });
    expect(digest(JSON.stringify(cases))).toMatchSnapshot();
  });
});

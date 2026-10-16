import React from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  BatteryHealthSummary,
  ChargingSummary,
  EfficiencySummary,
  MetricBatchResponse,
  VehicleStatus,
} from '@panda/types';
import type { DashboardDataRequirements, WidgetCtx } from '@panda/dashboards';

type QueryResult<T> = { data: T | null | undefined; isFetching: boolean };

const hooks = vi.hoisted(() => ({
  metricBatch: vi.fn<(...args: unknown[]) => QueryResult<MetricBatchResponse>>(),
  status: vi.fn<(...args: unknown[]) => QueryResult<VehicleStatus>>(),
  batteryHealth: vi.fn<(...args: unknown[]) => QueryResult<BatteryHealthSummary>>(),
  chargingSummary: vi.fn<(...args: unknown[]) => QueryResult<ChargingSummary>>(),
  efficiencySummary: vi.fn<(...args: unknown[]) => QueryResult<EfficiencySummary>>(),
}));

vi.mock('@panda/hooks', () => ({
  useMetricBatch: (...args: unknown[]) => hooks.metricBatch(...args),
  useCurrentVehicleStatus: (...args: unknown[]) => hooks.status(...args),
  useBatteryHealth: (...args: unknown[]) => hooks.batteryHealth(...args),
  useChargingSummary: (...args: unknown[]) => hooks.chargingSummary(...args),
  useEfficiencySummary: (...args: unknown[]) => hooks.efficiencySummary(...args),
}));

import {
  DashboardDataProvider,
  useDashboardDataAvailable,
  useDashboardDataSelector,
  useDashboardMetric,
} from '../../../../packages/dashboards/src/dashboardData';

const CTX: WidgetCtx = {
  vehicleId: 'synthetic-vehicle',
  from: '2026-10-01T00:00:00Z',
  to: '2026-10-08T00:00:00Z',
};
const SUMMARIES = ['status', 'batteryHealth', 'chargingSummary', 'efficiencySummary'] as const;
const ALL_SUMMARIES: DashboardDataRequirements = {
  status: true, batteryHealth: true, chargingSummary: true, efficiencySummary: true,
};
const CACHED_SUMMARIES = {
  status: {
    vehicle_id: CTX.vehicleId!,
    battery_level: null,
    range_miles: 0,
    power_state: null,
    charger_state: null,
    speed_mph: 0,
    latitude: null,
    longitude: null,
    is_online: false,
    last_updated: null,
  } satisfies VehicleStatus,
  batteryHealth: {
    usable_now_kwh: null,
    usable_new_kwh: null,
    battery_health_pct: null,
    estimated_degradation_pct: null,
    charging_cycles: null,
    charge_count: 0,
    total_energy_added_kwh: null,
    total_energy_used_kwh: null,
    charging_efficiency_pct: null,
  } satisfies BatteryHealthSummary,
  chargingSummary: {
    total_energy_kwh: 0, total_cost_usd: null, session_count: 0, weekly: [],
  } satisfies ChargingSummary,
  efficiencySummary: {
    avg: 300, p10: 250, p90: 400, total_miles: 100, efficiency_miles: 80, coverage_percent: 80,
  } satisfies EfficiencySummary,
};
const EMPTY_SNAPSHOT = {
  hasProvider: true, values: {}, series: {}, metrics: {}, isRefreshing: false,
};
const BATCH: MetricBatchResponse = {
  bucket: 'raw',
  density: 'full',
  values: [
    { metric: 'trip_miles', value: 0, unit: 'mi', label: 'Miles', ts: CTX.from },
    { metric: 'avg_efficiency', value: null, unit: 'Wh/mi', label: 'Efficiency', ts: null },
  ],
  series: [
    { metric: 'trip_miles', points: [{ ts: CTX.from!, value: 0 }] },
    { metric: 'energy_charged', points: [{ ts: CTX.from!, value: null }, { ts: CTX.to!, value: 12 }] },
  ],
};

// Memoization makes updates depend on the real external-store subscription,
// rather than a consumer rerender accidentally forced by its parent's props.
const SnapshotConsumer = React.memo(function SnapshotConsumer() {
  const snapshot = useDashboardDataSelector((value) => value);
  const available = useDashboardDataAvailable();
  return (
    <>
      <output data-testid="available">{String(available)}</output>
      <output data-testid="snapshot">{JSON.stringify(snapshot ? {
        ...snapshot,
        values: Object.fromEntries(snapshot.values),
        series: Object.fromEntries(snapshot.series),
        metrics: Object.fromEntries(snapshot.metrics),
      } : null)}</output>
    </>
  );
});

function MetricConsumer({ metric }: { metric: string | null }) {
  const data = useDashboardMetric(metric);
  return <output data-testid="metric">{JSON.stringify(data ?? null)}</output>;
}

function readSnapshot(): Record<string, unknown> {
  return JSON.parse(screen.getByTestId('snapshot').textContent!);
}

function provider(requirements: DashboardDataRequirements = {}, ctx = CTX) {
  return <DashboardDataProvider ctx={ctx} requirements={requirements}><SnapshotConsumer /></DashboardDataProvider>;
}

beforeEach(() => {
  vi.clearAllMocks();
  hooks.metricBatch.mockReturnValue({ data: undefined, isFetching: false });
  // Disabled queries can still expose cached results. The provider must filter
  // these explicitly; this fixture deliberately does not filter on vehicle ID.
  hooks.status.mockReturnValue({ data: CACHED_SUMMARIES.status, isFetching: false });
  hooks.batteryHealth.mockReturnValue({ data: CACHED_SUMMARIES.batteryHealth, isFetching: false });
  hooks.chargingSummary.mockReturnValue({ data: CACHED_SUMMARIES.chargingSummary, isFetching: false });
  hooks.efficiencySummary.mockReturnValue({ data: CACHED_SUMMARIES.efficiencySummary, isFetching: false });
});

afterEach(cleanup);

describe('DashboardDataProvider consumer contract', () => {
  it('lets widgets detect an absent provider without inventing data', () => {
    render(<><SnapshotConsumer /><MetricConsumer metric="trip_miles" /></>);

    expect(screen.getByTestId('available')).toHaveTextContent('false');
    expect(readSnapshot()).toBeNull();
    expect(screen.getByTestId('metric')).toHaveTextContent('null');
    expect(hooks.metricBatch).not.toHaveBeenCalled();
  });

  it.each(SUMMARIES)('exposes only requested %s while other hooks retain cached results', (summary) => {
    render(provider({ [summary]: true }));

    expect(screen.getByTestId('available')).toHaveTextContent('true');
    expect(readSnapshot()).toEqual({ ...EMPTY_SNAPSHOT, [summary]: CACHED_SUMMARIES[summary] });
    expect(hooks.status).toHaveBeenLastCalledWith(summary === 'status' ? CTX.vehicleId : null);
    expect(hooks.batteryHealth).toHaveBeenLastCalledWith(summary === 'batteryHealth' ? CTX.vehicleId : null);
    expect(hooks.chargingSummary).toHaveBeenLastCalledWith(
      summary === 'chargingSummary' ? CTX.vehicleId : null, CTX.from, CTX.to,
    );
    expect(hooks.efficiencySummary).toHaveBeenLastCalledWith(
      summary === 'efficiencySummary' ? CTX.vehicleId : null, CTX.from, CTX.to, undefined,
    );
  });

  it('drops disabled summaries immediately when only requirements change, and can enable them again', () => {
    const view = render(provider(ALL_SUMMARIES));
    expect(readSnapshot()).toEqual({ ...EMPTY_SNAPSHOT, ...CACHED_SUMMARIES });

    view.rerender(provider({
      status: false, batteryHealth: false, chargingSummary: false, efficiencySummary: false,
    }));
    expect(readSnapshot()).toEqual(EMPTY_SNAPSHOT);
    expect(hooks.status).toHaveBeenLastCalledWith(null);
    expect(hooks.batteryHealth).toHaveBeenLastCalledWith(null);
    expect(hooks.chargingSummary).toHaveBeenLastCalledWith(null, CTX.from, CTX.to);
    expect(hooks.efficiencySummary).toHaveBeenLastCalledWith(null, CTX.from, CTX.to, undefined);

    view.rerender(provider({ chargingSummary: true }));
    expect(readSnapshot()).toEqual({ ...EMPTY_SNAPSHOT, chargingSummary: CACHED_SUMMARIES.chargingSummary });
    view.rerender(provider());
    expect(readSnapshot()).toEqual(EMPTY_SNAPSHOT);
  });

  it('removes requested summary data when a later hook result is missing', () => {
    const view = render(provider(ALL_SUMMARIES));
    expect(readSnapshot()).toEqual({ ...EMPTY_SNAPSHOT, ...CACHED_SUMMARIES });

    hooks.status.mockReturnValue({ data: null, isFetching: false });
    hooks.batteryHealth.mockReturnValue({ data: undefined, isFetching: false });
    hooks.chargingSummary.mockReturnValue({ data: undefined, isFetching: false });
    hooks.efficiencySummary.mockReturnValue({ data: undefined, isFetching: false });
    view.rerender(provider(ALL_SUMMARIES));

    expect(readSnapshot()).toEqual(EMPTY_SNAPSHOT);
  });

  it('joins latest values and series by metric without losing zero, null or series-only data', () => {
    hooks.metricBatch.mockReturnValue({ data: BATCH, isFetching: false });
    const requirements = { metrics: BATCH.values.map(({ metric }) => ({ metric })) };
    const view = render(
      <DashboardDataProvider ctx={CTX} requirements={requirements}>
        <SnapshotConsumer />
        <MetricConsumer metric="trip_miles" />
      </DashboardDataProvider>,
    );

    const expectedMetrics = {
      trip_miles: { value: BATCH.values[0], series: BATCH.series[0]!.points },
      avg_efficiency: { value: BATCH.values[1], series: [] },
      energy_charged: { series: BATCH.series[1]!.points },
    };
    expect(readSnapshot()).toEqual({
      ...EMPTY_SNAPSHOT,
      values: { trip_miles: BATCH.values[0], avg_efficiency: BATCH.values[1] },
      series: { trip_miles: BATCH.series[0]!.points, energy_charged: BATCH.series[1]!.points },
      metrics: expectedMetrics,
    });
    expect(JSON.parse(screen.getByTestId('metric').textContent!)).toEqual(expectedMetrics.trip_miles);

    for (const metric of ['energy_charged', 'unknown_metric', null]) {
      view.rerender(
        <DashboardDataProvider ctx={CTX} requirements={requirements}>
          <SnapshotConsumer />
          <MetricConsumer metric={metric} />
        </DashboardDataProvider>,
      );
      expect(JSON.parse(screen.getByTestId('metric').textContent!)).toEqual(
        metric === 'energy_charged' ? expectedMetrics.energy_charged : null,
      );
    }
  });

  it.each([
    ['missing', undefined],
    ['empty', { values: [], series: [], bucket: 'raw', density: 'full' } satisfies MetricBatchResponse],
  ] as const)('clears previously displayed metrics when the next response is %s', (_label, data) => {
    hooks.metricBatch.mockReturnValue({ data: BATCH, isFetching: false });
    const view = render(provider({ metrics: [{ metric: 'trip_miles' }] }));
    expect(readSnapshot().metrics).toHaveProperty('trip_miles');

    hooks.metricBatch.mockReturnValue({ data, isFetching: false });
    view.rerender(provider({ metrics: [{ metric: 'avg_efficiency' }] }));

    expect(readSnapshot()).toEqual(EMPTY_SNAPSHOT);
    expect(hooks.metricBatch).toHaveBeenLastCalledWith(
      CTX.vehicleId, [{ metric: 'avg_efficiency' }], CTX.from, CTX.to, false, undefined,
    );
  });

  it.each(['metricBatch', ...SUMMARIES] as const)('notifies consumers while %s refreshes and settles', (source) => {
    const view = render(provider(ALL_SUMMARIES));
    expect(readSnapshot().isRefreshing).toBe(false);

    hooks[source].mockReturnValue({ data: undefined, isFetching: true });
    view.rerender(provider(ALL_SUMMARIES));
    expect(readSnapshot().isRefreshing).toBe(true);

    hooks[source].mockReturnValue({ data: undefined, isFetching: false });
    view.rerender(provider(ALL_SUMMARIES));
    expect(readSnapshot().isRefreshing).toBe(false);
  });

  it.each([
    [null, null, true],
    [CTX.from, null, false],
    [null, CTX.to, false],
    [CTX.from, CTX.to, false],
  ])('forwards range %s to %s with lifetime=%s and the shared trip cohort', (from, to, lifetime) => {
    const tripTagFilter = { tagIds: ['synthetic-tag'], tagMatch: 'any' as const, untagged: false };
    const ctx = { ...CTX, from, to, tripTagFilter };
    const metrics = [{ metric: 'avg_efficiency', include_latest: true, include_series: false }];
    render(provider({ metrics, chargingSummary: true, efficiencySummary: true }, ctx));

    expect(hooks.metricBatch).toHaveBeenLastCalledWith(CTX.vehicleId, metrics, from, to, lifetime, tripTagFilter);
    expect(hooks.chargingSummary).toHaveBeenLastCalledWith(CTX.vehicleId, from, to);
    expect(hooks.efficiencySummary).toHaveBeenLastCalledWith(CTX.vehicleId, from, to, tripTagFilter);
  });

  it('passes a missing vehicle to every query without substituting another vehicle', () => {
    render(provider(ALL_SUMMARIES, { vehicleId: null, from: null, to: null }));

    expect(hooks.metricBatch).toHaveBeenLastCalledWith(null, [], null, null, true, undefined);
    expect(hooks.status).toHaveBeenLastCalledWith(null);
    expect(hooks.batteryHealth).toHaveBeenLastCalledWith(null);
    expect(hooks.chargingSummary).toHaveBeenLastCalledWith(null, null, null);
    expect(hooks.efficiencySummary).toHaveBeenLastCalledWith(null, null, null, undefined);
  });
});

import React from 'react';
import { act, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WidgetCtx } from '@panda/dashboards';

type QueryMock = {
  data: unknown;
  isFetching: boolean;
  isError?: boolean;
  refetch?: () => unknown;
};

const hooks = vi.hoisted(() => {
  const ok = () => ({ data: null, isFetching: false }) as QueryMock;
  return {
    metricBatch: vi.fn<(...args: unknown[]) => QueryMock>((..._args) => ({ data: { values: [], series: [] }, isFetching: false })),
    status: vi.fn<(...args: unknown[]) => QueryMock>((..._args) => ok()),
    batteryHealth: vi.fn<(...args: unknown[]) => QueryMock>((..._args) => ok()),
    chargingSummary: vi.fn<(...args: unknown[]) => QueryMock>((..._args) => ok()),
    efficiencySummary: vi.fn<(...args: unknown[]) => QueryMock>((..._args) => ok()),
  };
});

vi.mock('@panda/hooks', () => ({
  useMetricBatch: (...args: unknown[]) => hooks.metricBatch(...args),
  useCurrentVehicleStatus: (...args: unknown[]) => hooks.status(...args),
  useBatteryHealth: (...args: unknown[]) => hooks.batteryHealth(...args),
  useChargingSummary: (...args: unknown[]) => hooks.chargingSummary(...args),
  useEfficiencySummary: (...args: unknown[]) => hooks.efficiencySummary(...args),
}));

import {
  DashboardDataProvider,
  useDashboardDataSelector,
  type DashboardDataSnapshot,
} from '../../../../packages/dashboards/src/dashboardData';

describe('DashboardDataProvider trip-tag filters', () => {
  it('passes the same normalized cohort into metric batch and efficiency summary requests', () => {
    const filter = { tagIds: ['tag-bike', 'tag-rack'], tagMatch: 'any' as const, untagged: false };
    const ctx: WidgetCtx = {
      vehicleId: 'vehicle-1',
      from: '2024-01-01T00:00:00Z',
      to: '2024-01-31T23:59:59Z',
      tripTagFilter: filter,
    };

    render(
      <DashboardDataProvider ctx={ctx} requirements={{ metrics: [{ metric: 'avg_efficiency' }], efficiencySummary: true }}>
        <div />
      </DashboardDataProvider>,
    );

    expect(hooks.metricBatch).toHaveBeenLastCalledWith(
      'vehicle-1',
      [{ metric: 'avg_efficiency' }],
      ctx.from,
      ctx.to,
      false,
      filter,
    );
    expect(hooks.efficiencySummary).toHaveBeenLastCalledWith('vehicle-1', ctx.from, ctx.to, filter);
  });
});

describe('DashboardDataProvider read failures', () => {
  const ctx: WidgetCtx = { vehicleId: 'vehicle-1', from: '2024-01-01T00:00:00Z', to: '2024-01-31T23:59:59Z' };
  const requirements = {
    metrics: [{ metric: 'total_miles' }],
    status: true,
    batteryHealth: true,
    chargingSummary: true,
    efficiencySummary: true,
  };

  afterEach(() => {
    for (const hook of Object.values(hooks)) hook.mockReset();
    hooks.metricBatch.mockImplementation(() => ({ data: { values: [], series: [] }, isFetching: false }));
    for (const hook of [hooks.status, hooks.batteryHealth, hooks.chargingSummary, hooks.efficiencySummary]) {
      hook.mockImplementation(() => ({ data: null, isFetching: false }));
    }
  });

  function renderSnapshot() {
    const latest: { current: DashboardDataSnapshot | undefined } = { current: undefined };
    function Probe() {
      latest.current = useDashboardDataSelector((snapshot) => snapshot);
      return null;
    }
    render(
      <DashboardDataProvider ctx={ctx} requirements={requirements}>
        <Probe />
      </DashboardDataProvider>,
    );
    return latest;
  }

  it('flags a source as failed only when it has an error and no data, and retry refetches only failed sources', () => {
    const refetchBatch = vi.fn();
    const refetchHealth = vi.fn();
    const refetchCharging = vi.fn();
    const refetchStatus = vi.fn();
    hooks.metricBatch.mockImplementation(() => ({ data: undefined, isFetching: false, isError: true, refetch: refetchBatch }));
    hooks.batteryHealth.mockImplementation(() => ({ data: undefined, isFetching: false, isError: true, refetch: refetchHealth }));
    // An error with earlier data is not a failure: the tile keeps the last value.
    hooks.chargingSummary.mockImplementation(() => ({ data: { session_count: 1 }, isFetching: false, isError: true, refetch: refetchCharging }));
    hooks.status.mockImplementation(() => ({ data: null, isFetching: false, isError: false, refetch: refetchStatus }));

    const latest = renderSnapshot();

    expect(latest.current?.failed).toEqual({
      batch: true,
      status: false,
      batteryHealth: true,
      chargingSummary: false,
      efficiencySummary: false,
    });

    act(() => latest.current?.retry());
    expect(refetchBatch).toHaveBeenCalledTimes(1);
    expect(refetchHealth).toHaveBeenCalledTimes(1);
    expect(refetchCharging).not.toHaveBeenCalled();
    expect(refetchStatus).not.toHaveBeenCalled();
  });

  it('reports no failures when every source succeeds', () => {
    const latest = renderSnapshot();

    expect(Object.values(latest.current?.failed ?? {})).toEqual([false, false, false, false, false]);
  });
});

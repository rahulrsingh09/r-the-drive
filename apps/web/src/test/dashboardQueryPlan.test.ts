import { describe, expect, it, vi } from 'vitest';
import type { DashboardDataRequirements, WidgetCtx } from '@panda/dashboards';
import { createDashboardQueryPlan } from '../../../../packages/dashboards/src/dashboardQueryPlan';

describe('dashboard query plan', () => {
  it('owns immutable copies without freezing or normalizing caller declarations', () => {
    const setFilter = vi.fn();
    const ctx: WidgetCtx = {
      vehicleId: 'synthetic-vehicle',
      from: null,
      to: '2026-10-08T00:00:00Z',
      tripTagFilter: { tagIds: ['b', 'a', 'a'], tagMatch: 'any', untagged: false, setFilter },
    };
    const requirements: DashboardDataRequirements = {
      metrics: [{ metric: 'trip_miles' }, { metric: 'energy_charged', include_latest: false, include_series: true }],
      status: true,
      chargingSummary: true,
    };

    const plan = createDashboardQueryPlan(ctx, requirements);

    expect(plan).toEqual({
      vehicleId: ctx.vehicleId,
      from: ctx.from,
      to: ctx.to,
      lifetime: false,
      metrics: requirements.metrics,
      summaryVehicles: {
        status: ctx.vehicleId, batteryHealth: null, chargingSummary: ctx.vehicleId, efficiencySummary: null,
      },
      tripTagFilter: ctx.tripTagFilter,
    });
    for (const value of [plan, plan.metrics, plan.metrics[0], plan.summaryVehicles, plan.tripTagFilter, plan.tripTagFilter?.tagIds]) {
      expect(Object.isFrozen(value)).toBe(true);
    }
    expect(plan.tripTagFilter?.setFilter).toBe(setFilter);

    requirements.metrics![0]!.metric = 'changed-caller';
    ctx.tripTagFilter!.tagIds.push('new-tag');
    requirements.status = false;
    expect(plan.metrics[0]).toEqual({ metric: 'trip_miles' });
    expect(plan.tripTagFilter?.tagIds).toEqual(['b', 'a', 'a']);
    expect(plan.summaryVehicles.status).toBe('synthetic-vehicle');
  });

  it('keeps every summary disabled without a selected vehicle', () => {
    const plan = createDashboardQueryPlan({ vehicleId: null, from: null, to: null }, {
      status: true, batteryHealth: true, chargingSummary: true, efficiencySummary: true,
    });

    expect(plan.lifetime).toBe(true);
    expect(plan.metrics).toEqual([]);
    expect(plan.summaryVehicles).toEqual({
      status: null, batteryHealth: null, chargingSummary: null, efficiencySummary: null,
    });
  });
});

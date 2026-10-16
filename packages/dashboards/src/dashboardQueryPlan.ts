import type { MetricBatchMetricRequest } from '@panda/types';
import type { DashboardDataRequirements, WidgetCtx } from './registry';

export const DASHBOARD_SUMMARY_SOURCES = [
  'status', 'batteryHealth', 'chargingSummary', 'efficiencySummary',
] as const;

type SummarySource = typeof DASHBOARD_SUMMARY_SOURCES[number];

/** A detached request description. Building a plan never freezes caller data. */
export interface DashboardQueryPlan {
  readonly vehicleId: string | null;
  readonly from: string | null;
  readonly to: string | null;
  readonly lifetime: boolean;
  readonly metrics: readonly Readonly<MetricBatchMetricRequest>[];
  readonly summaryVehicles: Readonly<Record<SummarySource, string | null>>;
  readonly tripTagFilter: WidgetCtx['tripTagFilter'];
}

/**
 * Widget declarations are independent demands: requesting a value or series
 * anywhere is sufficient, regardless of declarations that disable that facet.
 */
export function mergeDashboardRequirements(
  declarations: readonly DashboardDataRequirements[],
): DashboardDataRequirements {
  if (declarations.length === 0) return {};

  const names = new Set<string>();
  const latest = new Set<string>();
  const series = new Set<string>();
  const summaries: Record<SummarySource, boolean> = {
    status: false, batteryHealth: false, chargingSummary: false, efficiencySummary: false,
  };

  for (const declaration of declarations) {
    for (const source of DASHBOARD_SUMMARY_SOURCES) {
      if (declaration[source] === true) summaries[source] = true;
    }
    for (const request of declaration.metrics ?? []) {
      names.add(request.metric);
      if (request.include_latest !== false) latest.add(request.metric);
      if (request.include_series === true) series.add(request.metric);
    }
  }

  if (names.size === 0) return summaries;
  return {
    ...summaries,
    metrics: [...names].sort((a, b) => a.localeCompare(b)).map((metric) => ({
      metric,
      include_latest: latest.has(metric),
      include_series: series.has(metric),
    })),
  };
}

export function createDashboardQueryPlan(
  ctx: WidgetCtx,
  requirements: DashboardDataRequirements,
): DashboardQueryPlan {
  const summaryVehicles: Record<SummarySource, string | null> = {
    status: null, batteryHealth: null, chargingSummary: null, efficiencySummary: null,
  };
  for (const source of DASHBOARD_SUMMARY_SOURCES) {
    if (requirements[source]) summaryVehicles[source] = ctx.vehicleId;
  }

  let tripTagFilter: WidgetCtx['tripTagFilter'];
  if (ctx.tripTagFilter) {
    const tagIds = [...ctx.tripTagFilter.tagIds];
    Object.freeze(tagIds);
    tripTagFilter = Object.freeze({ ...ctx.tripTagFilter, tagIds });
  }

  return Object.freeze({
    vehicleId: ctx.vehicleId,
    from: ctx.from,
    to: ctx.to,
    lifetime: !ctx.from && !ctx.to,
    // Direct provider callers retain their omitted flags. The existing batch
    // hook, rather than this adapter, owns defaults at that public boundary.
    metrics: Object.freeze((requirements.metrics ?? []).map((request) => Object.freeze({ ...request }))),
    summaryVehicles: Object.freeze(summaryVehicles),
    tripTagFilter,
  });
}

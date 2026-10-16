import React from 'react';
import {
  useBatteryHealth,
  useChargingSummary,
  useCurrentVehicleStatus,
  useEfficiencySummary,
  useMetricBatch,
} from '@panda/hooks';
import type {
  BatteryHealthSummary,
  ChargingSummary,
  EfficiencySummary,
  MetricBatchResponse,
  MetricSeriesPoint,
  MetricValueResponse,
  VehicleStatus,
} from '@panda/types';
import { getWidgetForInstance } from './registry';
import type { DashboardDataRequirements, WidgetInstance, WidgetCtx } from './registry';
import { createDashboardQueryPlan, mergeDashboardRequirements } from './dashboardQueryPlan';

export type { DashboardDataRequirements } from './registry';

export interface DashboardMetricData {
  value?: MetricValueResponse;
  series: MetricSeriesPoint[];
}

export interface DashboardDataSnapshot {
  hasProvider: true;
  values: ReadonlyMap<string, MetricValueResponse>;
  series: ReadonlyMap<string, MetricSeriesPoint[]>;
  metrics: ReadonlyMap<string, DashboardMetricData>;
  status?: VehicleStatus | null;
  batteryHealth?: BatteryHealthSummary;
  chargingSummary?: ChargingSummary;
  efficiencySummary?: EfficiencySummary;
  isRefreshing: boolean;
}

class DashboardDataStore {
  private readonly listeners = new Set<() => void>();

  constructor(private snapshot: DashboardDataSnapshot) {}

  getSnapshot = () => this.snapshot;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  publish(snapshot: DashboardDataSnapshot) {
    if (this.snapshot === snapshot) return;
    this.snapshot = snapshot;
    this.listeners.forEach((listener) => listener());
  }
}

const DashboardDataContext = React.createContext<DashboardDataStore | null>(null);

export function collectDashboardDataRequirements(widgets: readonly WidgetInstance[]): DashboardDataRequirements {
  const declarations = widgets.flatMap((instance) => {
    const requirements = getWidgetForInstance(instance)?.dataRequirements?.(instance);
    return requirements ? [requirements] : [];
  });
  return mergeDashboardRequirements(declarations);
}

interface QueryObservation<T> {
  data: T | null | undefined;
  isPlaceholderData?: boolean;
}

/**
 * Query observers may carry the preceding key's result while fetching. Only a
 * committed real result establishes which vehicle that source belongs to.
 * Range/filter placeholders for that vehicle remain usable.
 */
function useVehicleSource<T>(vehicleId: string | null, observation: QueryObservation<T>): T | undefined {
  const confirmedVehicle = React.useRef<string | null>(null);
  const { data, isPlaceholderData } = observation;
  React.useLayoutEffect(() => {
    if (vehicleId && data != null && !isPlaceholderData) confirmedVehicle.current = vehicleId;
  }, [vehicleId, data, isPlaceholderData]);

  if (!vehicleId || (isPlaceholderData && confirmedVehicle.current !== vehicleId)) return undefined;
  return data ?? undefined;
}

/** Build once per batch response, independently of high-frequency live status. */
function projectMetrics(batch: MetricBatchResponse | undefined) {
  const values = new Map<string, MetricValueResponse>();
  const series = new Map<string, MetricSeriesPoint[]>();
  const metrics = new Map<string, DashboardMetricData>();

  for (const value of batch?.values ?? []) {
    values.set(value.metric, value);
    metrics.set(value.metric, { value, series: [] });
  }
  for (const entry of batch?.series ?? []) {
    series.set(entry.metric, entry.points);
    const value = values.get(entry.metric);
    metrics.set(entry.metric, value ? { value, series: entry.points } : { series: entry.points });
  }
  return { values, series, metrics };
}

export function DashboardDataProvider({
  ctx,
  requirements,
  children,
}: {
  ctx: WidgetCtx;
  requirements: DashboardDataRequirements;
  children: React.ReactNode;
}) {
  const plan = React.useMemo(() => createDashboardQueryPlan(ctx, requirements), [
    ctx.vehicleId, ctx.from, ctx.to, ctx.tripTagFilter,
    requirements.metrics, requirements.status, requirements.batteryHealth,
    requirements.chargingSummary, requirements.efficiencySummary,
  ]);
  const batch = useMetricBatch(plan.vehicleId, plan.metrics, plan.from, plan.to, plan.lifetime, plan.tripTagFilter);
  const status = useCurrentVehicleStatus(plan.summaryVehicles.status);
  const batteryHealth = useBatteryHealth(plan.summaryVehicles.batteryHealth);
  const chargingSummary = useChargingSummary(
    plan.summaryVehicles.chargingSummary, plan.from, plan.to,
  );
  const efficiencySummary = useEfficiencySummary(
    plan.summaryVehicles.efficiencySummary, plan.from, plan.to, plan.tripTagFilter,
  );

  const batchData = useVehicleSource(plan.metrics.length ? plan.vehicleId : null, batch);
  const statusData = useVehicleSource(plan.summaryVehicles.status, status);
  const selectedStatus = statusData?.vehicle_id && statusData.vehicle_id !== plan.vehicleId
    ? undefined
    : statusData;
  const healthData = useVehicleSource(plan.summaryVehicles.batteryHealth, batteryHealth);
  const chargingData = useVehicleSource(plan.summaryVehicles.chargingSummary, chargingSummary);
  const efficiencyData = useVehicleSource(plan.summaryVehicles.efficiencySummary, efficiencySummary);
  const metricData = React.useMemo(() => projectMetrics(batchData), [batchData]);

  const snapshot = React.useMemo<DashboardDataSnapshot>(() => Object.freeze({
    hasProvider: true,
    ...metricData,
    ...(selectedStatus ? { status: selectedStatus } : {}),
    ...(healthData ? { batteryHealth: healthData } : {}),
    ...(chargingData ? { chargingSummary: chargingData } : {}),
    ...(efficiencyData ? { efficiencySummary: efficiencyData } : {}),
    isRefreshing: batch.isFetching || status.isFetching || batteryHealth.isFetching
      || chargingSummary.isFetching || efficiencySummary.isFetching,
  }), [
    metricData, selectedStatus, healthData, chargingData, efficiencyData,
    batch.isFetching, status.isFetching, batteryHealth.isFetching,
    chargingSummary.isFetching, efficiencySummary.isFetching,
  ]);

  // Changing vehicles supplies a new, already-scoped store without remounting
  // widgets. Existing stores only publish committed renders, before paint.
  const store = React.useMemo(() => new DashboardDataStore(snapshot), [plan.vehicleId]);
  React.useLayoutEffect(() => {
    store.publish(snapshot);
  }, [snapshot, store]);

  return <DashboardDataContext.Provider value={store}>{children}</DashboardDataContext.Provider>;
}

export function useDashboardDataSelector<T>(selector: (snapshot: DashboardDataSnapshot) => T): T | undefined {
  const store = React.useContext(DashboardDataContext);
  const subscribe = React.useCallback(
    (listener: () => void) => store?.subscribe(listener) ?? (() => {}),
    [store],
  );
  const getSelection = React.useMemo(() => {
    let previous: DashboardDataSnapshot | undefined;
    let selected: T | undefined;
    return () => {
      const snapshot = store?.getSnapshot();
      if (snapshot !== previous) {
        previous = snapshot;
        selected = snapshot ? selector(snapshot) : undefined;
      }
      return selected;
    };
  }, [store, selector]);
  return React.useSyncExternalStore(subscribe, getSelection, getSelection);
}

export function useDashboardMetric(metric: string | null) {
  return useDashboardDataSelector((snapshot) => metric ? snapshot.metrics.get(metric) : undefined);
}

export function useDashboardDataAvailable() {
  return React.useContext(DashboardDataContext) !== null;
}

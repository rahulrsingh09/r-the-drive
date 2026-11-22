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

/** The provider queries that a tile can read from. */
export type DashboardDataSource = 'batch' | 'status' | 'batteryHealth' | 'chargingSummary' | 'efficiencySummary';

/**
 * A source has failed when its read returned an error and no earlier data
 * exists. Background reads do not toast, so the tile shows this error itself.
 */
export type DashboardDataFailures = Readonly<Record<DashboardDataSource, boolean>>;

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
  failed: DashboardDataFailures;
  /** Refetches only the sources that have failed. */
  retry: () => void;
}

const NO_FAILURES: DashboardDataFailures = {
  batch: false,
  status: false,
  batteryHealth: false,
  chargingSummary: false,
  efficiencySummary: false,
};

interface ProviderQueryState {
  data?: unknown;
  isError?: boolean;
  refetch?: () => unknown;
}

function hasFailed(query: ProviderQueryState, required: boolean) {
  return required && query.isError === true && query.data == null;
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

  const failedBatch = hasFailed(batch, plan.metrics.length > 0);
  const failedStatus = hasFailed(status, Boolean(plan.summaryVehicles.status));
  const failedBatteryHealth = hasFailed(batteryHealth, Boolean(plan.summaryVehicles.batteryHealth));
  const failedChargingSummary = hasFailed(chargingSummary, Boolean(plan.summaryVehicles.chargingSummary));
  const failedEfficiencySummary = hasFailed(efficiencySummary, Boolean(plan.summaryVehicles.efficiencySummary));
  const { refetch: refetchBatch } = batch;
  const { refetch: refetchStatus } = status;
  const { refetch: refetchBatteryHealth } = batteryHealth;
  const { refetch: refetchChargingSummary } = chargingSummary;
  const { refetch: refetchEfficiencySummary } = efficiencySummary;
  const retry = React.useCallback(() => {
    const sources: Array<[boolean, (() => unknown) | undefined]> = [
      [failedBatch, refetchBatch],
      [failedStatus, refetchStatus],
      [failedBatteryHealth, refetchBatteryHealth],
      [failedChargingSummary, refetchChargingSummary],
      [failedEfficiencySummary, refetchEfficiencySummary],
    ];
    for (const [sourceFailed, refetch] of sources) {
      if (sourceFailed) void refetch?.();
    }
  }, [
    failedBatch,
    failedStatus,
    failedBatteryHealth,
    failedChargingSummary,
    failedEfficiencySummary,
    refetchBatch,
    refetchStatus,
    refetchBatteryHealth,
    refetchChargingSummary,
    refetchEfficiencySummary,
  ]);
  const failed = React.useMemo<DashboardDataFailures>(
    () => failedBatch || failedStatus || failedBatteryHealth || failedChargingSummary || failedEfficiencySummary
      ? {
          batch: failedBatch,
          status: failedStatus,
          batteryHealth: failedBatteryHealth,
          chargingSummary: failedChargingSummary,
          efficiencySummary: failedEfficiencySummary,
        }
      : NO_FAILURES,
    [failedBatch, failedStatus, failedBatteryHealth, failedChargingSummary, failedEfficiencySummary],
  );

  const snapshot = React.useMemo<DashboardDataSnapshot>(() => Object.freeze({
    hasProvider: true,
    ...metricData,
    ...(selectedStatus ? { status: selectedStatus } : {}),
    ...(healthData ? { batteryHealth: healthData } : {}),
    ...(chargingData ? { chargingSummary: chargingData } : {}),
    ...(efficiencyData ? { efficiencySummary: efficiencyData } : {}),
    isRefreshing: batch.isFetching || status.isFetching || batteryHealth.isFetching
      || chargingSummary.isFetching || efficiencySummary.isFetching,
    failed,
    retry,
  }), [
    metricData, selectedStatus, healthData, chargingData, efficiencyData,
    batch.isFetching, status.isFetching, batteryHealth.isFetching,
    chargingSummary.isFetching, efficiencySummary.isFetching,
    failed, retry,
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

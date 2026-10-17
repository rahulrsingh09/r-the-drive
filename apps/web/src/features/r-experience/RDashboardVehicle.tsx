import React from 'react';
import { useCurrentVehicleStatus, useMetricBatch, useResolvedVehicleSelection } from '@panda/hooks';
import type { WidgetPresentationProps } from '@panda/dashboards';
import { finiteReading, isConfiguredR2 } from './readings';
import { RVehicleStage } from './RVehicleStage';
import { currentVehicleRecord } from './telemetryModel';
import './r-full-dashboard.css';

export const HomeVehicleHeadingContext = React.createContext<string | undefined>(undefined);
const metrics = [
  { metric: 'trip_miles', include_latest: true, include_series: false },
  { metric: 'total_trips', include_latest: true, include_series: false },
  { metric: 'avg_efficiency', include_latest: true, include_series: false },
];

export function RDashboardVehicle({ instance, ctx, children }: WidgetPresentationProps) {
  const headingWidget = React.useContext(HomeVehicleHeadingContext);
  const { vehicles, effectiveVehicleId } = useResolvedVehicleSelection();
  const vehicleId = ctx.vehicleId ?? effectiveVehicleId;
  const vehicle = vehicles.find(item => item.id === vehicleId);
  const snapshot = useCurrentVehicleStatus(vehicleId);
  const status = currentVehicleRecord(snapshot, vehicleId);
  const showStage = isConfiguredR2(vehicle) || headingWidget === instance.id;
  const batch = useMetricBatch(showStage ? vehicleId : null, metrics, ctx.from ?? null, ctx.to ?? null, !ctx.from && !ctx.to);
  const values = new Map<string, number | null>();
  if (!batch.isPlaceholderData) {
    for (const item of batch.data?.values ?? []) {
      if (!values.has(item.metric)) values.set(item.metric, finiteReading(item.value));
    }
  }
  const retrySnapshot = () => { void snapshot.refetch(); };
  const retryTotals = () => { void batch.refetch(); };
  if (!vehicle) return <p role="status">Loading vehicle…</p>;
  if (!showStage) return <>{children}</>;
  return (
    <div className="r-dashboard-vehicle">
      <RVehicleStage vehicle={vehicle} status={status} headingLevel={headingWidget === instance.id ? 'h1' : 'h2'}
        distance={values.get('trip_miles') ?? null} trips={values.get('total_trips') ?? null} efficiency={values.get('avg_efficiency') ?? null} />
      {snapshot.isError && <p className="r-reading-note" role="alert">Vehicle readings could not be refreshed. <button type="button" onClick={retrySnapshot}>Retry</button></p>}
      {batch.isError && <p className="r-reading-note" role="alert">Period totals could not be loaded. <button type="button" onClick={retryTotals}>Retry</button></p>}
    </div>
  );
}

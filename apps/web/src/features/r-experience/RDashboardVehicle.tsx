import React from 'react';
import { useCurrentVehicleStatus, useMetricBatch, useResolvedVehicleSelection } from '@panda/hooks';
import type { WidgetPresentationProps } from '@panda/dashboards';
import { finiteReading, isConfiguredR2 } from './readings';
import { RVehicleStage } from './RVehicleStage';
import './r-full-dashboard.css';

export const HomeVehicleHeadingContext = React.createContext<string | undefined>(undefined);
const metrics = ['trip_miles', 'total_trips', 'avg_efficiency'].map(metric => ({
  metric, include_latest: true, include_series: false,
}));

export function RDashboardVehicle({ instance, ctx, children }: WidgetPresentationProps) {
  const headingWidget = React.useContext(HomeVehicleHeadingContext);
  const { vehicles, effectiveVehicleId } = useResolvedVehicleSelection();
  const vehicleId = ctx.vehicleId ?? effectiveVehicleId;
  const vehicle = vehicles.find(item => item.id === vehicleId);
  const snapshot = useCurrentVehicleStatus(vehicleId);
  const status = !snapshot.isPlaceholderData && snapshot.data?.vehicle_id === vehicleId ? snapshot.data : null;
  const showStage = isConfiguredR2(vehicle) || headingWidget === instance.id;
  const batch = useMetricBatch(showStage ? vehicleId : null, metrics, ctx.from ?? null, ctx.to ?? null, !ctx.from && !ctx.to);
  const metric = (name: string) => batch.isPlaceholderData ? null : finiteReading(batch.data?.values?.find(item => item.metric === name)?.value);
  if (!vehicle) return <p role="status">Loading vehicle…</p>;
  if (!showStage) return <>{children}</>;
  return (
    <div className="r-dashboard-vehicle">
      <RVehicleStage vehicle={vehicle} status={status} headingLevel={headingWidget === instance.id ? 'h1' : 'h2'}
        distance={metric('trip_miles')} trips={metric('total_trips')} efficiency={metric('avg_efficiency')} />
      {snapshot.isError && <p className="r-reading-note" role="alert">Vehicle readings could not be refreshed. <button type="button" onClick={() => void snapshot.refetch()}>Retry</button></p>}
      {batch.isError && <p className="r-reading-note" role="alert">Period totals could not be loaded. <button type="button" onClick={() => void batch.refetch()}>Retry</button></p>}
    </div>
  );
}

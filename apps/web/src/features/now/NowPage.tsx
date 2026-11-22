import React from 'react';
import { useAuth, useResolvedVehicleSelection } from '@panda/hooks';
import { SelectPicker } from '@panda/ui/primitives';
import { RAppLayout } from '../r-experience/RAppLayout';
import { NoVehicleState } from '../../components/layout/NoVehicleState';
import { NowView } from './NowView';

/** The Now body: vehicle selection, then the Now view. Used inside the R shell. */
export function NowBody() {
  const { setActiveVehicleId } = useAuth();
  const { authReady, effectiveVehicleId, vehicleSelectionReady, vehicles } = useResolvedVehicleSelection();
  const vehicle = vehicles.find(item => item.id === effectiveVehicleId);
  if (!authReady || !vehicleSelectionReady) return <p className="now-context" role="status" aria-busy="true">Loading your vehicle…</p>;
  if (!effectiveVehicleId) return <NoVehicleState />;
  if (!vehicle) return <p className="now-context" role="status" aria-busy="true">Loading your vehicle…</p>;
  const vehicleControl = vehicles.length > 1 ? <SelectPicker className="now-vehicle-select" value={vehicle.id}
    onChange={vehicleId => setActiveVehicleId?.(vehicleId || null)} aria-label="Select vehicle"
    options={vehicles.map(item => ({ value: item.id, label: item.display_name || item.model }))} /> : undefined;
  return <NowView key={vehicle.id} vehicle={vehicle} vehicleControl={vehicleControl} />;
}

/** "/" — the Now page. */
export function NowPage() {
  return <RAppLayout activeKey="now"><NowBody /></RAppLayout>;
}

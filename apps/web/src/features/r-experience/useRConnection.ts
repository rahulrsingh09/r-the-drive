import React from 'react';
import { useAuth, useCurrentVehicleStatus, useResolvedVehicleSelection, useVehicleStatus } from '@panda/hooks';
import { resolveVehicleOnlineState } from '../../components/layout/vehicleConnection';
import { connectionReadings, remindFeedHealth } from './connectionModel';

export function useRConnection() {
  const { effectiveVehicleId, vehicleSelectionReady } = useResolvedVehicleSelection();
  const accessToken = useAuth(state => state.accessToken);
  const vehicleId = vehicleSelectionReady ? effectiveVehicleId : null;
  const { status: live, connected, connectionState } = useVehicleStatus(vehicleId, vehicleSelectionReady ? accessToken : null);
  const snapshot = useCurrentVehicleStatus(vehicleId);
  const { status, feed, renewal } = connectionReadings(vehicleId, snapshot, live);
  React.useEffect(() => remindFeedHealth(vehicleId, feed, connected), [vehicleId, feed?.key, feed?.title, feed?.message, connected]);
  const onlineState = resolveVehicleOnlineState({
    liveVehicleId: vehicleId, vehicleSelectionReady, connectionState, connected, feedHealthIssue: feed,
  });
  return { status, feed, renewal, onlineState };
}

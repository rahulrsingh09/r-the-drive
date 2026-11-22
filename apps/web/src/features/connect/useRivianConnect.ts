import React from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { api, notifyVehicleCredentialsRefreshed, queryKeys, useAuth, useVehicles } from '@panda/hooks';
import type { ConnectedRivianVehicle, ConnectResult } from '@panda/types';
import { connectErrorMessage } from '../../components/connect/connectError';
import { emitAuthError } from '../../components/feedback/toast';

export type RConnectMode = 'add' | 'refresh' | undefined;

export interface RConnected {
  vehicleId: string;
  vehicleName: string;
  waitingMessage: string | null;
}

export function formatVehicleName(vehicle: ConnectedRivianVehicle) {
  return vehicle.name || [vehicle.model_year, vehicle.model].filter(Boolean).join(' ') || 'Rivian vehicle';
}

/**
 * The real Rivian connect flow shared by the first-run steps on /connect and /connect/otp:
 * check the Rivian result, refresh credentials (Reconnect), or add the confirmed car.
 */
export function useRivianConnect(mode: RConnectMode, vehicleId: string | undefined) {
  const queryClient = useQueryClient();
  const setDefaultVehicleId = useAuth((state) => state.setDefaultVehicleId);
  const { data: connectedVehicles, isSuccess: vehiclesLoaded } = useVehicles();
  const refreshVehicleId = mode === 'refresh' ? vehicleId : undefined;
  const refreshVehicle = connectedVehicles?.find((vehicle) => vehicle.id === refreshVehicleId);
  const [error, setError] = React.useState('');
  const [loading, setLoading] = React.useState(false);
  const [candidates, setCandidates] = React.useState<ConnectedRivianVehicle[]>([]);
  const [connected, setConnected] = React.useState<RConnected | null>(null);

  const reportError = React.useCallback((message: string) => {
    setError(message);
    emitAuthError('Rivian connection failed', message);
  }, []);

  async function handleResult(result: ConnectResult, noVehiclesMessage: string) {
    if (refreshVehicleId) {
      if (!refreshVehicle) {
        reportError('Choose an existing vehicle before refreshing Rivian credentials.');
        return;
      }
      const match = result.vehicles.find((vehicle) => vehicle.id === refreshVehicle.rivian_vehicle_id);
      if (!match) {
        reportError('That Rivian account does not include this vehicle.');
        return;
      }
      const refreshed = await api.refreshVehicleCredentials(refreshVehicleId, refreshVehicle.rivian_vehicle_id);
      setDefaultVehicleId(refreshVehicleId);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.status(refreshVehicleId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicle.health(refreshVehicleId) }),
      ]);
      notifyVehicleCredentialsRefreshed(refreshVehicleId);
      setConnected({
        vehicleId: refreshVehicleId,
        vehicleName: formatVehicleName(match),
        waitingMessage: refreshed.telemetry_status !== 'connected'
          ? refreshed.telemetry_error ?? 'Rivian accepted the new sign-in. R is still waiting for car data and keeps trying.'
          : null,
      });
      return;
    }
    if (!result.vehicles.length) {
      reportError(noVehiclesMessage);
      return;
    }
    setCandidates(result.vehicles);
  }

  async function confirmVehicle(vehicle: ConnectedRivianVehicle) {
    setError('');
    setLoading(true);
    try {
      const added = await api.addVehicle({ rivian_vehicle_id: vehicle.id, name: vehicle.name, model: vehicle.model, vin: vehicle.vin });
      setDefaultVehicleId(added.vehicle_id);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.status(added.vehicle_id) }),
      ]);
      setConnected({
        vehicleId: added.vehicle_id,
        vehicleName: formatVehicleName(vehicle),
        waitingMessage: added.telemetry_status === 'unchanged' ? 'Your car is ready. Its existing data settings are unchanged.' : null,
      });
      setCandidates([]);
    } catch (failure) {
      reportError(connectErrorMessage(failure));
    } finally {
      setLoading(false);
    }
  }

  return {
    refreshVehicleId, refreshVehicle, vehiclesLoaded, firstRun: vehiclesLoaded && (connectedVehicles?.length ?? 0) === 0 && !refreshVehicleId,
    error, setError, loading, setLoading, candidates, connected, reportError, handleResult, confirmVehicle,
  };
}

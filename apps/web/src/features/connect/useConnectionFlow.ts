import { useEffect, useReducer, useRef } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { api, notifyVehicleCredentialsRefreshed, queryKeys, useAuth, useVehicles } from '@panda/hooks';
import type { ConnectedRivianVehicle, ConnectResult } from '@panda/types';
import { connectErrorMessage } from '../../components/connect/connectError';
import { emitAuthError } from '../../components/feedback/toast';

type ConnectionState = {
  error: string;
  loading: boolean;
  vehicles: ConnectedRivianVehicle[];
  successVehicleName: string;
  successVehicleId: string;
  successWaitingMessage: string | null;
};
type ConnectionEvent =
  | { type: 'begin' }
  | { type: 'settled' }
  | { type: 'failed'; message: string }
  | { type: 'choose'; vehicles: ConnectedRivianVehicle[] }
  | { type: 'saved'; id: string; vehicle: ConnectedRivianVehicle; waiting: string | null }
  | { type: 'reset' };
const empty: ConnectionState = { error: '', loading: false, vehicles: [], successVehicleName: '', successVehicleId: '', successWaitingMessage: null };

function transition(state: ConnectionState, event: ConnectionEvent): ConnectionState {
  switch (event.type) {
    case 'reset': return empty;
    case 'begin': return { ...state, loading: true, error: '' };
    case 'settled': return { ...state, loading: false };
    case 'failed': return { ...state, error: event.message };
    case 'choose': return { ...state, vehicles: event.vehicles };
    case 'saved': return { ...state, vehicles: [], successVehicleId: event.id, successVehicleName: formatVehicleName(event.vehicle), successWaitingMessage: event.waiting };
  }
}

export function formatVehicleName(vehicle: ConnectedRivianVehicle) {
  if (vehicle.name) return vehicle.name;
  const description = [vehicle.model_year, vehicle.model].filter(Boolean).join(' ');
  return description || 'Rivian vehicle';
}

export function useConnectionFlow(refreshVehicleId: string | undefined, owner: string) {
  const queryClient = useQueryClient();
  const setDefaultVehicleId = useAuth((state) => state.setDefaultVehicleId);
  const { data: connectedVehicles } = useVehicles();
  const refreshVehicle = connectedVehicles?.find((vehicle) => vehicle.id === refreshVehicleId);
  const [state, dispatch] = useReducer(transition, empty);
  const sequence = useRef(0);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  useEffect(() => {
    dispatch({ type: 'reset' });
    return () => { sequence.current += 1; };
  }, [owner]);

  const run = async (work: (active: () => boolean) => Promise<void>) => {
    const ticket = ++sequence.current;
    const active = () => ticket === sequence.current && owner === currentOwner.current;
    dispatch({ type: 'begin' });
    try {
      await work(active);
    } catch (error) {
      if (active()) {
        const message = connectErrorMessage(error);
        dispatch({ type: 'failed', message });
        emitAuthError('Rivian connection failed', message);
      }
    } finally {
      if (active()) dispatch({ type: 'settled' });
    }
  };

  const fail = (message: string) => {
    dispatch({ type: 'failed', message });
    emitAuthError('Rivian connection failed', message);
  };

  const persist = async (vehicle: ConnectedRivianVehicle, active: () => boolean) => {
    const added = await api.addVehicle({ rivian_vehicle_id: vehicle.id, name: vehicle.name, model: vehicle.model, vin: vehicle.vin });
    if (!active()) return;
    setDefaultVehicleId(added.vehicle_id);
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.all }),
      queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.status(added.vehicle_id) }),
    ]);
    if (active()) dispatch({ type: 'saved', id: added.vehicle_id, vehicle, waiting: added.telemetry_status === 'unchanged' ? 'Your vehicle access is ready. Existing telemetry settings are unchanged.' : null });
  };

  const accept = async (result: ConnectResult, active: () => boolean, emptyMessage: string) => {
    if (!active()) return;
    if (refreshVehicleId) {
      if (!refreshVehicle) return fail('Choose an existing vehicle before refreshing Rivian credentials.');
      const vehicle = result.vehicles.find((candidate) => candidate.id === refreshVehicle.rivian_vehicle_id);
      if (!vehicle) return fail('That Rivian account does not include this vehicle.');
      const refreshed = await api.refreshVehicleCredentials(refreshVehicleId, refreshVehicle.rivian_vehicle_id);
      if (!active()) return;
      setDefaultVehicleId(refreshVehicleId);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.all }),
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicles.status(refreshVehicleId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.vehicle.health(refreshVehicleId) }),
      ]);
      if (!active()) return;
      notifyVehicleCredentialsRefreshed(refreshVehicleId);
      dispatch({ type: 'saved', id: refreshVehicleId, vehicle, waiting: refreshed.telemetry_status === 'connected' ? null : refreshed.telemetry_error ?? 'Credentials were refreshed. R:The Drive is still waiting for vehicle data and will keep retrying.' });
      return;
    }
    switch (result.vehicles.length) {
      case 0: fail(emptyMessage); return;
      case 1: await persist(result.vehicles[0]!, active); return;
      default: dispatch({ type: 'choose', vehicles: result.vehicles });
    }
  };

  return {
    ...state,
    refreshVehicle,
    selectVehicle: (vehicle: ConnectedRivianVehicle) => run((active) => persist(vehicle, active)),
    submitCredentials: (email: string, password: string, challenge: (id: string) => void) => run(async (active) => {
      const result = await api.connectRivian(email, password);
      if (!active()) return;
      if (result.requires_otp && result.challenge_id) challenge(result.challenge_id);
      else await accept(result, active, 'Rivian sign-in succeeded, but no vehicles were returned for this account.');
    }),
    submitOtp: (challengeId: string, otp: string) => run(async (active) => {
      await accept(await api.connectRivianOtp(challengeId, otp), active, 'Rivian verification succeeded, but no vehicles were returned for this account.');
    }),
  };
}

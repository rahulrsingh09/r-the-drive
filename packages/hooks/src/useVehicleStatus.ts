import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { create } from 'zustand';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { api } from './api';
import { useAuth } from './useAuth';
import { queryKeys } from './queryKeys';
import { useApiQuery, queryPolicies } from './queryRuntime';
import { LiveVehicleConnection, VEHICLE_CREDENTIALS_REFRESHED_EVENT, type ConnectionHistory, type VehicleConnectionState } from './liveVehicleConnection';
import { configuredTarget, websocketProtocol } from './urlPolicy';
export { VEHICLE_CREDENTIALS_REFRESHED_EVENT };

interface LiveStatusStore {
  status: Record<string, VehicleStatus>;
  connected: Record<string, boolean>;
  setStatus: (vehicleId: string, status: Partial<VehicleStatus>) => void;
  setConnected: (vehicleId: string, connected: boolean) => void;
  removeVehicle: (vehicleId: string) => void;
}

export function notifyVehicleCredentialsRefreshed(vehicleId: string) {
  const event = new CustomEvent(VEHICLE_CREDENTIALS_REFRESHED_EVENT, { detail: { vehicleId } });
  if (typeof window !== 'undefined') window.dispatchEvent(event);
}

export const useLiveStatusStore = create<LiveStatusStore>((set, get) => ({
  status: {}, connected: {},
  setStatus(vehicleId, patch) {
    const status = get().status;
    set({ status: Object.assign({}, status, { [vehicleId]: Object.assign({}, status[vehicleId], patch) }) });
  },
  setConnected(vehicleId, value) {
    set({ connected: Object.assign({}, get().connected, { [vehicleId]: value }) });
  },
  removeVehicle(vehicleId) {
    const without = <T,>(values: Record<string, T>) => Object.fromEntries(Object.entries(values).filter(([key]) => key !== vehicleId));
    set({ status: without(get().status), connected: without(get().connected) });
  },
}));

export function useVehicleStatus(vehicleId: string | null, accessToken: string | null) {
  const client = useQueryClient();
  const history = useRef<ConnectionHistory>({ removed: null, refreshAttempted: false });
  const [connectionState, setConnectionState] = useState<VehicleConnectionState>('idle');
  useEffect(() => {
    if (history.current.removed !== vehicleId) history.current.removed = null;
    if (!vehicleId || !accessToken) {
      setConnectionState('idle');
      if (vehicleId) useLiveStatusStore.getState().setConnected(vehicleId, false);
      return;
    }
    const connection = new LiveVehicleConnection(vehicleId, accessToken, `${getWebSocketBaseUrl()}/v1/vehicles/live?vehicle_id=${vehicleId}`, history.current, {
      transition: setConnectionState,
      connected: (value) => useLiveStatusStore.getState().setConnected(vehicleId, value),
      status: (value) => useLiveStatusStore.getState().setStatus(vehicleId, value),
      current: () => useLiveStatusStore.getState().status[vehicleId],
      refresh: () => api.refresh(),
      clearSession: () => useAuth.getState().clearSession(),
      revoke: () => {
        useLiveStatusStore.getState().removeVehicle(vehicleId);
        const auth = useAuth.getState();
        const selection = { activeVehicleId: auth.activeVehicleId === vehicleId ? null : auth.activeVehicleId, defaultVehicleId: auth.defaultVehicleId === vehicleId ? null : auth.defaultVehicleId };
        useAuth.setState(selection);
        for (const query of client.getQueryCache().findAll()) if (query.queryKey.includes(vehicleId)) client.removeQueries({ queryKey: query.queryKey, exact: true });
        client.setQueryData<Vehicle[]>(queryKeys.vehicles.all, (vehicles) => vehicles?.filter(({ id }) => id !== vehicleId));
        void client.invalidateQueries({ queryKey: queryKeys.vehicles.all });
      },
    });
    connection.start();
    return () => connection.stop();
  }, [vehicleId, accessToken, client]);
  const status = useLiveStatusStore((state) => vehicleId ? state.status[vehicleId] ?? null : null);
  const connected = useLiveStatusStore((state) => vehicleId ? state.connected[vehicleId] ?? false : false);
  return { status, connected, connectionState };
}

export function useCurrentVehicleStatus(vehicleId: string | null) {
  const live = useLiveStatusStore((state) => vehicleId ? state.status[vehicleId] : null);
  const query = useApiQuery(queryKeys.vehicles.status(vehicleId), 'vehicleStatus', [vehicleId!], [vehicleId], {
    ...queryPolicies.history, staleTime: 30_000, refetchInterval: 30_000, refetchOnReconnect: true, refetchOnWindowFocus: 'always',
  });
  const stored = query.data ?? null;
  let data = live ?? stored;
  if (stored && live) {
    const timestamp = (status: VehicleStatus) => Date.parse(status.last_updated ?? status.last_event_at ?? '');
    const storedTime = timestamp(stored), liveTime = timestamp(live);
    const newestLive = Number.isFinite(liveTime) && (!Number.isFinite(storedTime) || liveTime >= storedTime);
    const [older, newer] = newestLive ? [stored, live] : [live, stored];
    data = Object.assign({}, older, Object.fromEntries(Object.entries(newer).filter(([, value]) => value != null)));
  }
  return { ...query, data };
}

export function getWebSocketBaseUrl(
  configuredBaseUrl = (() => {
    const env = import.meta as { env?: { VITE_WS_URL?: string; VITE_API_URL?: string; VITE_PANDA_API_BASE_URL?: string } };
    return env.env?.VITE_WS_URL ?? env.env?.VITE_API_URL ?? env.env?.VITE_PANDA_API_BASE_URL;
  })(),
  location: Pick<Location, 'hostname' | 'origin'> | undefined = typeof window === 'undefined' ? undefined : window.location,
) {
  if (!location) return 'ws://localhost:3001';
  if (!configuredBaseUrl) return websocketProtocol(location.origin);
  const target = configuredTarget(configuredBaseUrl, location.origin);
  if (target.loopback) return websocketProtocol(location.origin);
  const resolved = websocketProtocol(target.url?.toString() ?? configuredBaseUrl);
  return resolved.endsWith('/') ? resolved.slice(0, -1) : resolved;
}

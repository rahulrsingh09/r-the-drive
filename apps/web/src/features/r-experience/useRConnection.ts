import React from 'react';
import type { VehicleStatus } from '@panda/types';
import { useAuth, useCurrentVehicleStatus, useResolvedVehicleSelection, useVehicleStatus } from '@panda/hooks';
import { getFeedHealthIssue, resolveVehicleOnlineState } from '../../components/layout/vehicleConnection';
import { getRivianCredentialRenewalNotice } from '../../lib/rivianCredentialRenewal';

type BannerStatus = Pick<VehicleStatus,
  'auth_state' | 'renewal_state' | 'worker_health' | 'telemetry_stale' | 'telemetry_stale_reason'
  | 'last_updated' | 'last_ws_received_at' | 'last_event_at'>;

export type RConnectionBanner =
  | { kind: 'offline'; since: string | null }
  | { kind: 'rivian'; since: string | null; reason: 'credentials' | 'feed' };

/**
 * App banner from real data. Offline comes from the browser. "Rivian connection lost" comes from the
 * vehicle status: Rivian asked for a new sign-in, the collector failed, or the Rivian socket went silent.
 * A sleeping car only makes battery readings stale; that is not a lost connection.
 */
export function rConnectionBanner(status: Partial<BannerStatus> | null | undefined, browserOnline: boolean): RConnectionBanner | null {
  const since = status?.last_updated ?? status?.last_event_at ?? null;
  if (!browserOnline) return { kind: 'offline', since };
  if (!status) return null;
  if (status.auth_state === 'needs_reauth' || status.renewal_state === 'reauth_required') {
    return { kind: 'rivian', since, reason: 'credentials' };
  }
  if (status.worker_health === 'error' || status.telemetry_stale_reason === 'ws_silent') {
    return { kind: 'rivian', since: status.last_ws_received_at ?? since, reason: 'feed' };
  }
  return null;
}

/** navigator.onLine, kept current by the online and offline events. */
export function useBrowserOnline() {
  return React.useSyncExternalStore(
    (notify) => {
      window.addEventListener('online', notify);
      window.addEventListener('offline', notify);
      return () => {
        window.removeEventListener('online', notify);
        window.removeEventListener('offline', notify);
      };
    },
    () => navigator.onLine,
    () => true,
  );
}

export function useRConnection() {
  const { effectiveVehicleId, vehicleSelectionReady } = useResolvedVehicleSelection();
  const accessToken = useAuth(state => state.accessToken);
  const vehicleId = vehicleSelectionReady ? effectiveVehicleId : null;
  const { status: live, connected, connectionState } = useVehicleStatus(vehicleId, vehicleSelectionReady ? accessToken : null);
  const snapshot = useCurrentVehicleStatus(vehicleId);
  const current = !snapshot.isPlaceholderData && snapshot.data?.vehicle_id === vehicleId ? snapshot.data : null;
  const feed = getFeedHealthIssue(current);
  const status = current ?? (live?.vehicle_id === vehicleId ? live : null);
  const renewal = getRivianCredentialRenewalNotice(current);
  const browserOnline = useBrowserOnline();
  const onlineState = resolveVehicleOnlineState({
    liveVehicleId: vehicleId, vehicleSelectionReady, connectionState, connected, feedHealthIssue: feed,
  });
  const banner = rConnectionBanner(status, browserOnline);
  return { vehicleId, status, feed, renewal, onlineState, banner };
}

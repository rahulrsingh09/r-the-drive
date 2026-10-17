import type { VehicleOnlineState } from '@panda/ui/primitives';

interface FeedHealthStatus {
  auth_state?: string | null;
  auth_reason_code?: string | null;
  worker_health?: string | null;
  worker_health_msg?: string | null;
  telemetry_stale?: boolean;
  telemetry_stale_reason?: string | null;
}

const FEED_HEALTH_TOAST_COOLDOWN_MS = 15 * 60 * 1000;

const UNHEALTHY_WORKERS = new Set(['error', 'stale', 'degraded']);

export function getFeedHealthIssue(status?: FeedHealthStatus | null) {
  if (!status) return null;
  const reconnect = status.auth_state === 'needs_reauth';
  if (!reconnect && !UNHEALTHY_WORKERS.has(status.worker_health ?? '')) return null;
  const notice = reconnect ? {
    key: `needs_reauth:${status.auth_reason_code ?? 'unknown'}`,
    title: 'Rivian feed disconnected',
    message: 'Rivian credentials are missing or expired. Reconnect the vehicle from Settings → Vehicles.',
  } : {
    key: `unhealthy:${status.worker_health ?? 'unknown'}:${status.telemetry_stale_reason ?? 'unknown'}`,
    title: 'Vehicle feed unhealthy',
    message: 'Panda is not receiving fresh vehicle telemetry. Open Health for collector details.',
  };
  return { ...notice, message: status.worker_health_msg ?? notice.message };
}

export function shouldEmitFeedHealthToast(storage: Storage, key: string, now = Date.now()) {
  const previous = Number(storage.getItem(key));
  const elapsed = Number.isFinite(previous) ? now - previous : Infinity;
  const ready = !(elapsed < FEED_HEALTH_TOAST_COOLDOWN_MS);
  if (ready) storage.setItem(key, String(now));
  return ready;
}

export function resolveVehicleOnlineState({
  liveVehicleId,
  vehicleSelectionReady,
  connectionState,
  connected,
  feedHealthIssue,
}: {
  liveVehicleId: string | null;
  vehicleSelectionReady: boolean;
  connectionState: string;
  connected: boolean;
  feedHealthIssue: ReturnType<typeof getFeedHealthIssue>;
}): VehicleOnlineState {
  const transportReady = vehicleSelectionReady && connected && connectionState !== 'failed';
  const states: VehicleOnlineState[] = ['offline', 'connecting', feedHealthIssue ? 'unhealthy' : 'online'];
  const stage = liveVehicleId ? 1 + Number(transportReady) : 0;
  return states[stage]!;
}

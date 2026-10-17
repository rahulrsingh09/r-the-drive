import type { VehicleStatus } from '@panda/types';
import { getFeedHealthIssue, shouldEmitFeedHealthToast } from '../../components/layout/vehicleConnection';
import { getRivianCredentialRenewalNotice } from '../../lib/rivianCredentialRenewal';
import { emitToast } from '../../components/feedback/toast';
import { currentVehicleRecord } from './telemetryModel';

export function connectionReadings(
  vehicleId: string | null,
  snapshot: { isPlaceholderData: boolean; data?: VehicleStatus | null | undefined },
  live: VehicleStatus | null,
) {
  const current = currentVehicleRecord(snapshot, vehicleId);
  const realtime = currentVehicleRecord({ isPlaceholderData: false, data: live }, vehicleId);
  return { status: current ?? realtime, feed: getFeedHealthIssue(current), renewal: getRivianCredentialRenewalNotice(current) };
}

export function remindFeedHealth(vehicleId: string | null, feed: ReturnType<typeof getFeedHealthIssue>, connected: boolean) {
  if (!connected || !vehicleId || !feed) return;
  const notice = { title: feed.title, message: feed.message, variant: 'error' as const };
  const storageKey = ['rm-feed-health-toast', vehicleId, feed.key].join(':');
  const poll = () => {
    const due = shouldEmitFeedHealthToast(localStorage, storageKey);
    if (due) emitToast(notice);
  };
  poll();
  const interval = window.setInterval(poll, 60_000);
  return () => window.clearInterval(interval);
}

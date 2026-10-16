import React from 'react';
import { useAuth, useCurrentVehicleStatus, useResolvedVehicleSelection, useVehicleStatus } from '@panda/hooks';
import { getFeedHealthIssue, resolveVehicleOnlineState, shouldEmitFeedHealthToast } from '../../components/layout/vehicleConnection';
import { emitToast } from '../../components/feedback/toast';
import { getRivianCredentialRenewalNotice } from '../../lib/rivianCredentialRenewal';

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
  React.useEffect(() => {
    if (!vehicleId || !feed || !connected) return;
    const emit = () => {
      const key = `rm-feed-health-toast:${vehicleId}:${feed.key}`;
      if (shouldEmitFeedHealthToast(localStorage, key)) {
        emitToast({ title: feed.title, message: feed.message, variant: 'error' });
      }
    };
    emit();
    const timer = window.setInterval(emit, 60_000);
    return () => window.clearInterval(timer);
  }, [vehicleId, feed?.key, feed?.title, feed?.message, connected]);
  const onlineState = resolveVehicleOnlineState({
    liveVehicleId: vehicleId, vehicleSelectionReady, connectionState, connected, feedHealthIssue: feed,
  });
  return { status, feed, renewal, onlineState };
}

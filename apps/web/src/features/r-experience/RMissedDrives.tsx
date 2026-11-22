import React from 'react';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Route, TriangleAlert } from 'lucide-react';
import { useRecoverTripGap, useTripGaps } from '@panda/hooks';
import type { TripGap } from '@panda/types';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import { formatDuration, formatMiles } from '@panda/ui/lib/utils';
import { emitToast } from '../../components/feedback/toast';

const DISMISSED_KEY = 'panda.r.dismissedTripGaps';

// Dismissal is a per-device display choice; gaps also expire with the lookback.
function readDismissed(): string[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? '[]');
    return Array.isArray(value) ? value.filter((id): id is string => typeof id === 'string') : [];
  } catch {
    return [];
  }
}

function useVisibleGaps(vehicleId: string | null) {
  const query = useTripGaps(vehicleId);
  const [dismissed, setDismissed] = React.useState(readDismissed);
  const gaps = (query.data?.gaps ?? []).filter((gap) => !dismissed.includes(gap.id));
  const dismiss = (id: string) => {
    const next = [...new Set([...dismissed, id])].slice(-50);
    localStorage.setItem(DISMISSED_KEY, JSON.stringify(next));
    setDismissed(next);
  };
  return { query, gaps, dismiss };
}

export function formatGapWindow(gap: TripGap) {
  return `${formatAppDate(gap.started_at, { weekday: 'short', year: undefined })} · ${formatAppTime(gap.started_at)}–${formatAppTime(gap.ended_at)}`;
}

function formatGapFacts(gap: TripGap) {
  const facts = [formatMiles(gap.distance_miles, 1), formatDuration(gap.duration_seconds / 60)];
  if (gap.soc_start != null && gap.soc_end != null) {
    facts.push(`${Math.round(gap.soc_start)}% → ${Math.round(gap.soc_end)}%`);
  }
  return facts.join(' · ');
}

/** Attention card listing drives that have telemetry but no trip. */
export function RMissedDrives({ vehicleId, canRecover }: { vehicleId: string | null; canRecover: boolean }) {
  const { gaps, dismiss } = useVisibleGaps(vehicleId);
  if (gaps.length === 0) return null;
  return (
    <section className="r-missed" aria-labelledby="r-missed-title">
      <header className="r-missed-header">
        <TriangleAlert aria-hidden="true" />
        <div>
          <h2 id="r-missed-title">
            {gaps.length === 1 ? 'Possible missed drive' : `${gaps.length} possible missed drives`}
          </h2>
          <p>The car reported movement, but no trip was recorded.</p>
        </div>
      </header>
      <ul>
        {gaps.map((gap) => (
          <MissedDrive
            key={gap.id}
            gap={gap}
            vehicleId={vehicleId}
            canRecover={canRecover}
            onDismiss={() => dismiss(gap.id)}
          />
        ))}
      </ul>
    </section>
  );
}

function MissedDrive({
  gap,
  vehicleId,
  canRecover,
  onDismiss,
}: {
  gap: TripGap;
  vehicleId: string | null;
  canRecover: boolean;
  onDismiss: () => void;
}) {
  const [confirming, setConfirming] = React.useState(false);
  const recover = useRecoverTripGap(vehicleId);
  const recoverRef = React.useRef<HTMLButtonElement>(null);
  const confirmRef = React.useRef<HTMLButtonElement>(null);
  const opened = React.useRef(false);
  React.useEffect(() => {
    // Keep keyboard focus on the control that replaced the one just used.
    if (confirming) confirmRef.current?.focus();
    else if (opened.current) recoverRef.current?.focus();
    opened.current = confirming;
  }, [confirming]);
  const confirm = () => {
    recover.mutate(gap.id, {
      onSuccess: () => emitToast({ variant: 'success', title: 'Trip recovered', message: formatGapWindow(gap) }),
      onError: (error) => emitToast(recoveryFailure(error)),
    });
  };
  return (
    <li className="r-missed-drive" data-confirming={confirming || undefined}>
      <div className="r-missed-facts">
        <strong>{formatGapWindow(gap)}</strong>
        <span>{formatGapFacts(gap)}</span>
      </div>
      {confirming ? (
        <div className="r-missed-confirm">
          <p>Recover this drive from the stored odometer, GPS and battery readings? Recorded trips are not changed.</p>
          <div className="r-missed-actions">
            <button ref={confirmRef} type="button" className="r-recover" onClick={confirm} disabled={recover.isPending}>
              {recover.isPending ? 'Recovering…' : 'Confirm recovery'}
            </button>
            <button type="button" className="r-quiet-action" onClick={() => setConfirming(false)} disabled={recover.isPending}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <div className="r-missed-actions">
          {canRecover ? (
            <button ref={recoverRef} type="button" className="r-recover" onClick={() => setConfirming(true)}>
              <Route aria-hidden="true" />
              Recover trip
            </button>
          ) : (
            <span className="r-missed-note">An owner or manager can recover this drive.</span>
          )}
          <button type="button" className="r-quiet-action" onClick={onDismiss}>
            Dismiss
          </button>
        </div>
      )}
    </li>
  );
}

function recoveryFailure(error: unknown) {
  const status = (error as { status?: number } | null)?.status;
  if (status === 404) {
    return { variant: 'info' as const, title: 'Drive no longer listed', message: 'It may already be recorded. Check Trips.' };
  }
  if (status === 409) {
    return { variant: 'warning' as const, title: 'Trip capture is busy', message: 'A live trip is still being saved. Try again in a minute.' };
  }
  return { variant: 'error' as const, title: 'Could not recover the trip', message: 'Try again in a minute.' };
}

/** Compact Trips banner that leads to the recovery card. */
export function RMissedDriveBanner({ vehicleId }: { vehicleId: string | null }) {
  const { gaps } = useVisibleGaps(vehicleId);
  if (gaps.length === 0) return null;
  return (
    <Link to="/vehicle-health" className="r-missed-banner">
      <TriangleAlert aria-hidden="true" />
      <span>
        <strong>{gaps.length === 1 ? '1 possible missed drive' : `${gaps.length} possible missed drives`}</strong>
        <small>{formatGapWindow(gaps[0]!)}</small>
      </span>
      <span className="r-missed-banner-action">
        Review
        <ArrowRight aria-hidden="true" />
      </span>
    </Link>
  );
}

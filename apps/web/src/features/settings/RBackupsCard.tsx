import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { api, queryKeys, useMe } from '@panda/hooks';
import type { BackupOverview } from '@panda/types';
import { BackupSection } from '../../components/settings/BackupSection';
import { rWhen } from '../r-experience/RAppLayout';

/** Summary line and note for the collapsed Backups card. */
export function rBackupSummary(overview: BackupOverview | undefined): { last: string | null; note: string } {
  if (!overview) return { last: null, note: 'Backup status is not available.' };
  const latest = overview.latest_successful_run;
  const last = rWhen(latest?.completed_at ?? latest?.created_at ?? null);
  const kept = overview.artifacts.length;
  const failed = overview.recent_runs.filter(run => run.status === 'failed').length;
  const schedule = overview.settings.enabled ? `${overview.settings.frequency} automatic` : 'manual';
  const count = `${kept} ${schedule} ${kept === 1 ? 'backup' : 'backups'} kept`;
  if (!kept && !latest) return { last, note: overview.settings.enabled ? 'No backup has run yet. The first one runs on schedule.' : 'Automatic backups are off.' };
  return { last, note: failed ? `${count}. ${failed} of the recent runs failed.` : `${count}, all completed. Nothing to do here.` };
}

export function RBackupsCard() {
  const me = useMe();
  const canManage = me.data?.role === 'admin' || me.data?.role === 'super_user';
  const overview = useQuery({
    queryKey: queryKeys.backups.overview(1, 10),
    queryFn: () => api.getBackupOverview({ page: 1, perPage: 10 }),
    enabled: canManage,
    retry: false,
  });
  const [manage, setManage] = React.useState(false);
  if (!canManage) return null;
  const summary = rBackupSummary(overview.data);
  return (
    <section id="backups" className="r-card r-rise" style={{ '--i': 4 } as React.CSSProperties} aria-label="Backups">
      <details className="r-pad">
        <summary><b>Backups</b>{summary.last && <span className="r-muted"> · last {summary.last}</span>}</summary>
        {overview.isLoading ? <p className="r-note" role="status">Loading backups…</p>
          : overview.isError ? <p className="r-form-error" role="alert">Backup status did not load.</p>
            : <p className="r-note">{summary.note}</p>}
        <button type="button" className="r-tablelink" aria-expanded={manage} onClick={() => setManage(value => !value)}>
          {manage ? 'Hide backup schedule and restore' : 'Backup schedule and restore'}
        </button>
        {manage && <div className="r-backups-manage"><BackupSection /></div>}
      </details>
    </section>
  );
}

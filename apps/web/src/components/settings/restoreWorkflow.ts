import type { BackupOverview, RestoreJob, RestorePlan } from '@panda/types';

type Artifact = BackupOverview['artifacts'][number];
export interface RestoreWorkflow {
  pendingRestoreArtifact: Artifact | null;
  pendingRestorePlan: RestorePlan | null;
  restoreConfirmation: string;
  activeRestore: { job: RestoreJob; token: string } | null;
  activeRestoreSizeBytes: number | null;
  restoreStatusUnavailable: boolean;
}
export const emptyRestoreWorkflow: RestoreWorkflow = {
  pendingRestoreArtifact: null, pendingRestorePlan: null, restoreConfirmation: '',
  activeRestore: null, activeRestoreSizeBytes: null, restoreStatusUnavailable: false,
};
type RestoreEvent =
  | { type: 'inspect'; artifact: Artifact; plan: RestorePlan }
  | { type: 'confirmation'; value: string }
  | { type: 'close' }
  | { type: 'dismiss' }
  | { type: 'start'; job: RestoreJob; token: string; size: number | null }
  | { type: 'poll'; id: string; job: RestoreJob }
  | { type: 'unavailable'; value: boolean };

export function restoreWorkflowTransition(state: RestoreWorkflow, event: RestoreEvent): RestoreWorkflow {
  switch (event.type) {
    case 'inspect': return { ...state, pendingRestoreArtifact: event.artifact, pendingRestorePlan: event.plan, restoreConfirmation: '' };
    case 'confirmation': return { ...state, restoreConfirmation: event.value };
    case 'dismiss': return { ...state, activeRestore: null, activeRestoreSizeBytes: null, restoreStatusUnavailable: false };
    case 'close': return { ...state, pendingRestoreArtifact: null, pendingRestorePlan: null, restoreConfirmation: '' };
    case 'start': return { ...emptyRestoreWorkflow, activeRestore: { job: event.job, token: event.token }, activeRestoreSizeBytes: event.size };
    case 'poll': return state.activeRestore?.job.id === event.id ? { ...state, activeRestore: { ...state.activeRestore, job: event.job } } : state;
    case 'unavailable': return { ...state, restoreStatusUnavailable: event.value };
  }
}

export function backupCatalog(overview: BackupOverview, requested: 'all' | 'local' | 's3' | null) {
  const buckets = { local: [] as Artifact[], s3: [] as Artifact[] };
  for (const artifact of overview.artifacts) buckets[artifact.storage_type === 's3' ? 's3' : 'local'].push(artifact);
  const local = buckets.local.length > 0 || buckets.s3.length === 0;
  const s3 = buckets.s3.length > 0 || Boolean(overview.settings.endpoint && overview.settings.bucket);
  const availableCatalogSources = [
    ...(local && s3 ? [{ value: 'all', label: 'All backups' }] : []),
    ...(local ? [{ value: 'local', label: 'Local' }] : []),
    ...(s3 ? [{ value: 's3', label: 'S3' }] : []),
  ];
  const fallback = local && buckets.s3.length > 0 ? 'all' : !local && buckets.s3.length > 0 ? 's3' : 'local';
  const selectedCatalogSource = requested && availableCatalogSources.some((source) => source.value === requested) ? requested : fallback;
  return { availableCatalogSources, selectedCatalogSource, visibleArtifacts: selectedCatalogSource === 'all' ? overview.artifacts : buckets[selectedCatalogSource] };
}

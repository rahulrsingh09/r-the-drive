import {
  findOwnedDashboardBySlug,
  isSystemDefaultDashboard,
  materializeSystemDashboardDraft,
  materializeUserDashboardDraft,
  type DashboardConfig,
} from '@panda/dashboards';
import type { DashboardEditMutations } from './DashboardPageShell';

type SaveTarget = { kind: 'system' | 'owned'; config: DashboardConfig } | { kind: 'create' };

function ownedTarget(preferred: DashboardConfig | undefined, slug: string, mutations: DashboardEditMutations) {
  return preferred?.ownerId != null
    ? preferred
    : findOwnedDashboardBySlug(mutations.qc.getQueryData<DashboardConfig[]>(['dashboards']), slug);
}

export async function persistDashboardDraft(
  draft: DashboardConfig,
  saved: DashboardConfig,
  mutations: DashboardEditMutations,
): Promise<void> {
  let target: SaveTarget;
  if (mutations.isAdmin && isSystemDefaultDashboard(saved)) {
    target = { kind: 'system', config: saved };
  } else {
    const owned = ownedTarget(saved, draft.slug, mutations);
    target = owned ? { kind: 'owned', config: owned } : { kind: 'create' };
  }
  if (target.kind === 'system') {
    await mutations.updateAdminDashboard.mutateAsync(materializeSystemDashboardDraft(draft, target.config));
    return;
  }
  if (target.kind === 'create') {
    try {
      await mutations.createDashboard.mutateAsync(materializeUserDashboardDraft(draft));
      return;
    } catch {
      const queryKey = ['dashboards', 'slug', draft.slug];
      await mutations.qc.refetchQueries({ queryKey });
      const owned = ownedTarget(mutations.qc.getQueryData<DashboardConfig>(queryKey), draft.slug, mutations);
      if (!owned) throw new Error('Could not find owned dashboard after refetch');
      target = { kind: 'owned', config: owned };
    }
  }
  await mutations.updateDashboard.mutateAsync(materializeUserDashboardDraft(draft, target.config));
}

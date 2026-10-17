import type { DashboardConfig } from '@panda/dashboards';

export function resolveShellDashboard({
  exact, slug, response, fallback, loading, failed,
}: {
  exact: boolean; slug: string;
  response: DashboardConfig | undefined; fallback: DashboardConfig | undefined;
  loading: boolean; failed: boolean;
}) {
  const matching = response?.slug === slug ? response : undefined;
  return {
    saved: matching ?? (exact ? undefined : fallback),
    unavailable: exact && !loading && (failed || matching === undefined),
  };
}

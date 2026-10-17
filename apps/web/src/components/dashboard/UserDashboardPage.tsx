import React from 'react';
import { useNavigate, useParams, useSearch } from '@tanstack/react-router';
import { useCloneDashboard, downloadDashboardYaml, importDashboardYaml } from '@panda/dashboards';
import { Copy, Download, Upload } from 'lucide-react';
import { DashboardPageShell } from './DashboardPageShell';

export function UserDashboardPage() {
  const { slug } = useParams({ from: '/d/$slug' });
  const search = useSearch({ from: '/d/$slug' });
  const navigate = useNavigate();

  const isEditMode = search.edit === 1;
  const openDashboard = (destination: string, options: { edit?: 1; dashboardId?: string }) =>
    navigate({ to: '/d/$slug', params: { slug: destination }, search: options });
  const changeEditMode = (editing: boolean) => {
    const next: { edit?: 1; dashboardId?: string } = {};
    if (editing) next.edit = 1;
    if (search.dashboardId) next.dashboardId = search.dashboardId;
    return openDashboard(slug, next);
  };
  const cloneDashboard = useCloneDashboard();

  return (
    <DashboardPageShell
      navKey="dashboard"
      slug={slug}
      dashboardId={search.dashboardId}
      showEfficiencyDisplayToggle
      isEditMode={isEditMode}
      onEditModeChange={changeEditMode}
      renderActions={(state) => {
        const { activeConfig, setLocalConfig } = state;

        const actions = {
          async clone() {
            if (activeConfig) {
              const copy = await cloneDashboard.mutateAsync(activeConfig.id);
              openDashboard(copy.slug, { edit: 1, dashboardId: copy.id });
            }
          },
          export() {
            if (activeConfig) downloadDashboardYaml(activeConfig);
          },
          async import(input: HTMLInputElement) {
            const file = input.files?.[0];
            if (!file) return;
            const text = await file.text();
            let imported;
            try {
              imported = importDashboardYaml(text);
              setLocalConfig(imported);
              openDashboard(slug, { edit: 1 });
            } catch (cause) {
              const reason = cause instanceof Error ? cause.message : String(cause);
              alert(`Import failed: ${reason}`);
            }
            input.value = '';
          },
        };

        return (
          <>
            <button
              type="button"
              onClick={() => {
                void actions.clone();
              }}
              disabled={!activeConfig || cloneDashboard.isPending}
              title="Duplicate dashboard"
              className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs transition-colors hover:bg-bg-elevated disabled:opacity-40"
            >
              <Copy className="h-3.5 w-3.5" />
              Duplicate
            </button>

            <button
              type="button"
              onClick={actions.export}
              title="Export as YAML"
              className="p-1.5 rounded-lg border border-border hover:bg-bg-elevated transition-colors text-fg-tertiary hover:text-fg"
            >
              <Download className="h-3.5 w-3.5" />
            </button>
            <label
              title="Import from YAML"
              className="cursor-pointer p-1.5 rounded-lg border border-border hover:bg-bg-elevated transition-colors text-fg-tertiary hover:text-fg"
            >
              <Upload className="h-3.5 w-3.5" />
              <input type="file" accept=".yaml,.yml" className="sr-only" onChange={(event) => {
                void actions.import(event.target);
              }} />
            </label>
          </>
        );
      }}
    />
  );
}

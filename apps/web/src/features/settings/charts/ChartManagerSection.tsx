import { chartUpdateCommand, unassignedChartCopy } from './chartUpdateCommand';
import React from 'react';
import { downloadFeatureFile, useFeatureOperation } from '../../featureOperation';
import { useNavigate } from '@tanstack/react-router';
import {
  CHART_SOURCE_MANIFESTS,
  exportChartJson,
  importChartJson,
} from '@panda/dashboards';
import {
  useChartManager as useChartManagerApi,
  useChartSources as useChartSourcesApi,
  useAdminSetChartLock,
  useAdminRestoreChart,
  useCreateChart as useCreateChartApi,
  useDeleteChart as useDeleteChartApi,
  useResetChart as useResetChartApi,
  useSetChartEnabled as useSetChartEnabledApi,
} from '@panda/hooks';
import type { ChartManagerEntry, ChartRecord } from '@panda/types';
import { useDashboards } from '@panda/dashboards';
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, ResponsiveDialog, Switch } from '@panda/ui/primitives';
import { Download, Ellipsis, Pencil, Plus, Upload, Copy, Search, LockKeyhole, RotateCcw, Tag, X } from 'lucide-react';

type ChartFilter = 'all' | 'defaults' | 'mine' | 'customized' | 'disabled' | 'unassigned';

function downloadText(filename: string, contents: string) {
  downloadFeatureFile(new Blob([contents], { type: 'application/json;charset=utf-8' }), filename);
}

function recordExport(record: ChartRecord) {
  return exportChartJson(record);
}

export function ChartManagerSection() {
  const navigate = useNavigate();
  const operation = useFeatureOperation();
  const charts = useChartManagerApi();
  const sources = useChartSourcesApi();
  const dashboards = useDashboards();
  const createChart = useCreateChartApi();
  const deleteChart = useDeleteChartApi();
  const resetChart = useResetChartApi();
  const setEnabled = useSetChartEnabledApi();
  const adminSetLock = useAdminSetChartLock();
  const adminRestore = useAdminRestoreChart();
  const [query, setQuery] = React.useState('');
  const [filter, setFilter] = React.useState<ChartFilter>('all');
  const [mutationError, setMutationError] = React.useState<string | null>(null);
  const [confirmation, setConfirmation] = React.useState<{ kind: 'delete' | 'restore'; entry: ChartManagerEntry } | null>(null);
  const importInputRef = React.useRef<HTMLInputElement>(null);

  const sourceLabels = React.useMemo(
    () => new Map((sources.data ?? CHART_SOURCE_MANIFESTS).map((source) => [source.id, source.label])),
    [sources.data],
  );
  const dashboardLabels = React.useMemo(
    () => new Map((dashboards.data ?? []).map((dashboard) => [dashboard.slug, dashboard.name])),
    [dashboards.data],
  );
  const entries = React.useMemo(() => {
    const needle = query.trim().toLowerCase();
    const categories: Record<ChartFilter, (entry: ChartManagerEntry) => boolean> = {
      all: () => true, defaults: (entry) => entry.origin === 'system', mine: (entry) => entry.origin === 'personal',
      customized: (entry) => entry.origin === 'override', disabled: (entry) => !entry.effective.isEnabled,
      unassigned: (entry) => entry.effective.config.placements.length === 0,
    };
    const eligible = (charts.data ?? []).filter(categories[filter]);
    return needle ? eligible.filter(({ effective: chart }) => `${chart.name} ${chart.slug} ${chart.description ?? ''}`.toLowerCase().includes(needle)) : eligible;
  }, [charts.data, filter, query]);

  async function runChartAction(work: (active: () => boolean) => Promise<unknown>, fallback: string, commit?: () => void) {
    const active = operation.begin();
    try {
      await work(active);
      if (active()) { commit?.(); setMutationError(null); }
    } catch (error) {
      if (active()) setMutationError(error instanceof Error ? error.message : fallback);
    }
  }

  function handleImport(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    return runChartAction(async (active) => {
      const imported = await importChartJson(await file.text());
      if (!active()) return;
      const slugs = new Set((charts.data ?? []).map(({ effective }) => effective.slug));
      await createChart.mutateAsync({ ...unassignedChartCopy(imported), slug: slugs.has(imported.slug) ? `${imported.slug}-copy` : imported.slug });
    }, 'Unable to import chart.');
  }

  function handleDuplicate(entry: ChartManagerEntry) {
    const chart = entry.effective;
    return runChartAction(() => createChart.mutateAsync({ ...unassignedChartCopy(chart), slug: `${chart.slug}-copy`, name: `${chart.name} Copy` }), 'Unable to duplicate chart.');
  }

  async function saveEffectiveUpdate(entry: ChartManagerEntry, patch: { isEnabled: boolean }) {
    const command = chartUpdateCommand(entry, patch);
    if (!command) return;
    switch (command.kind) {
      case 'create': await createChart.mutateAsync(command.body); break;
      case 'enabled': await setEnabled.mutateAsync(command.body); break;
    }
  }

  async function handleDelete(entry: ChartManagerEntry) {
    const isReset = entry.origin === 'override' && !!entry.systemBase;
    if (isReset || entry.permissions.delete) setConfirmation({ kind: 'delete', entry });
  }

  async function confirmAction() {
    if (!confirmation) return;
    const pending = confirmation;
    const { entry } = pending;
    const mutate = pending.kind === 'restore' ? adminRestore.mutateAsync : entry.origin === 'override' && entry.systemBase ? resetChart.mutateAsync : deleteChart.mutateAsync;
    await runChartAction(() => mutate(entry.effective.id), 'Unable to update chart.');
    setConfirmation((current) => current === pending ? null : current);
  }

  function handleLock(entry: ChartManagerEntry) {
    return runChartAction(() => adminSetLock.mutateAsync({ id: entry.effective.id, locked: !entry.effective.isLocked }), 'Unable to update chart lock state.');
  }

  async function handleRestore(entry: ChartManagerEntry) {
    setConfirmation({ kind: 'restore', entry });
  }

  if (charts.isLoading) {
    return <Card><CardContent className="grid gap-3 p-5">{[1, 2, 3].map((item) => <div key={item} className="h-20 animate-pulse rounded-xl bg-bg-elevated" />)}</CardContent></Card>;
  }

  if (charts.isError) {
    return (
      <Card>
        <CardContent className="grid gap-3 p-5">
          <p className="text-sm text-fg-secondary">Chart management is unavailable from the connected API.</p>
          <p className="text-xs text-fg-tertiary">This usually means the server needs the chart-resource migration before this section can be used.</p>
          <Button variant="secondary" size="sm" onClick={() => { void charts.refetch(); }}>Retry</Button>
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="grid gap-5">
      <Card>
        <CardHeader>
          <div className="flex min-w-0 flex-1 items-start justify-between gap-3">
            <div className="min-w-0">
              <CardTitle>Charts</CardTitle>
              <p className="mt-1 text-sm text-fg-tertiary">Manage bundled chart defaults and personal copies.</p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <input ref={importInputRef} type="file" accept=".json,.panda-chart.json" className="sr-only" onChange={handleImport} />
              <Button variant="secondary" size="sm" iconLeft={<Upload className="h-4 w-4" />} aria-label="Import chart" onClick={() => importInputRef.current?.click()}>Import</Button>
              <Button variant="primary" size="sm" iconLeft={<Plus className="h-4 w-4" />} onClick={() => navigate({ to: '/settings/charts/new' })}>Create chart</Button>
            </div>
          </div>
        </CardHeader>
        <CardContent className="grid gap-4">
          <div className="grid gap-2 md:grid-cols-[minmax(0,1fr)_12rem]">
            <label className="relative block">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-fg-tertiary" />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search charts" aria-label="Search charts" className="h-10 w-full rounded-lg border border-border bg-bg-elevated pl-9 pr-3 text-sm text-fg outline-none focus:border-accent" />
            </label>
            <select value={filter} onChange={(event) => setFilter(event.target.value as ChartFilter)} aria-label="Filter charts" className="h-10 rounded-lg border border-border bg-bg-elevated px-3 text-sm text-fg outline-none focus:border-accent">
              <option value="all">All charts</option>
              <option value="defaults">App defaults</option>
              <option value="mine">My charts</option>
              <option value="customized">Customized</option>
              <option value="disabled">Disabled</option>
              <option value="unassigned">Unassigned</option>
            </select>
          </div>
          {mutationError ? <p role="alert" className="rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">{mutationError}</p> : null}
          {entries.length === 0 ? (
            <div className="rounded-xl border border-dashed border-border p-6 text-center text-sm text-fg-tertiary">No charts match this filter.</div>
          ) : (
            <div className="grid gap-2">
              {entries.map((entry) => (
                <ChartManagerRow
                  key={entry.effective.id}
                  entry={entry}
                  sourceLabels={sourceLabels}
                  dashboardLabels={dashboardLabels}
                  onEdit={() => navigate({ to: '/settings/charts/$chartId', params: { chartId: entry.effective.id } })}
                  onDuplicate={() => { void handleDuplicate(entry); }}
                  onExport={() => downloadText(`${entry.effective.slug}.panda-chart.json`, recordExport(entry.effective))}
                  onToggle={() => { void runChartAction(() => saveEffectiveUpdate(entry, { isEnabled: !entry.effective.isEnabled }), 'Unable to update chart.'); }}
                  onDelete={() => { void handleDelete(entry); }}
                  {...(entry.permissions.lock ? { onLock: () => { void handleLock(entry); } } : {})}
                  {...(entry.permissions.restore ? { onRestore: () => { void handleRestore(entry); } } : {})}
                />
              ))}
            </div>
          )}
        </CardContent>
      </Card>
      {confirmation ? <ChartConfirmationDialog confirmation={confirmation} pending={adminRestore.isPending || deleteChart.isPending || resetChart.isPending} onCancel={() => setConfirmation(null)} onConfirm={() => { void confirmAction(); }} /> : null}
    </div>
  );
}

function ChartConfirmationDialog({
  confirmation,
  pending,
  onCancel,
  onConfirm,
}: {
  confirmation: { kind: 'delete' | 'restore'; entry: ChartManagerEntry };
  pending: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const isRestore = confirmation.kind === 'restore';
  const isReset = !isRestore && confirmation.entry.origin === 'override' && !!confirmation.entry.systemBase;
  const title = isRestore
    ? `Restore ${confirmation.entry.effective.name}?`
    : isReset
      ? `Reset ${confirmation.entry.effective.name} to the app default?`
      : `Delete ${confirmation.entry.effective.name}?`;
  const description = isRestore
    ? 'This replaces the system chart with the bundled baseline.'
    : isReset
      ? 'Your personal override will be removed and the bundled chart will become effective again.'
      : 'This removes the personal chart and cannot be undone.';
  return <ResponsiveDialog titleId="chart-confirm-title" onClose={onCancel}><div className="flex min-h-0 flex-1 flex-col p-5"><div className="flex items-start justify-between gap-3"><div><h2 id="chart-confirm-title" className="text-base font-semibold text-fg">{title}</h2><p className="mt-2 text-sm text-fg-tertiary">{description}</p></div><IconButton label="Close confirmation" icon={<X className="h-4 w-4" />} onClick={onCancel} compact /></div><div className="mt-auto flex flex-col-reverse gap-2 pt-6 sm:mt-5 sm:flex-row sm:justify-end"><Button type="button" variant="secondary" size="md" onClick={onCancel}>Cancel</Button><Button type="button" variant="danger" size="md" loading={pending} onClick={onConfirm}>{isRestore ? 'Restore bundled' : isReset ? 'Reset to app default' : 'Delete chart'}</Button></div></div></ResponsiveDialog>;
}

function ChartManagerRow({
  entry,
  sourceLabels,
  dashboardLabels,
  onEdit,
  onDuplicate,
  onExport,
  onToggle,
  onDelete,
  onLock,
  onRestore,
}: {
  entry: ChartManagerEntry;
  sourceLabels: Map<string, string>;
  dashboardLabels: Map<string, string>;
  onEdit: () => void;
  onDuplicate: () => void;
  onExport: () => void;
  onToggle: () => void;
  onDelete: () => void;
  onLock?: () => void;
  onRestore?: () => void;
}) {
  const chart = entry.effective;
  const placements = chart.config.placements;

  return (
    <article className="relative grid gap-4 rounded-xl border border-border bg-bg-elevated/25 p-4 pb-14 md:grid-cols-[minmax(0,1fr)_auto] md:items-start md:pb-4">
      <div className="min-w-0">
        <div className="flex min-w-0 flex-wrap items-center gap-2"><h3 className="truncate text-sm font-semibold text-fg">{chart.name}</h3><OriginBadge entry={entry} />{statusBadgesForRow(entry)}</div>
        <p className="mt-1 truncate text-xs text-fg-tertiary">{chart.slug} · {chart.config.series.length} series · {chart.config.sources.map((source) => sourceLabels.get(source.sourceId) ?? source.sourceId).join(', ')}</p>
        {chart.description ? <p className="mt-1 line-clamp-2 text-sm text-fg-secondary">{chart.description}</p> : null}
        <div className="mt-4">
          <p className="mb-2 text-xs font-medium text-fg-secondary">Assigned dashboards</p>
          <div className="flex flex-wrap items-center gap-1.5">
            {placements.map((placement) => <DashboardBadge key={placement.dashboardSlug} slug={placement.dashboardSlug} name={dashboardLabels.get(placement.dashboardSlug) ?? placement.dashboardSlug} />)}
            {placements.length === 0 ? <span className="text-xs text-fg-tertiary">None assigned</span> : null}
          </div>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5 md:justify-end">
        <IconButton label={`Edit ${chart.name}`} icon={<Pencil className="h-4 w-4" />} onClick={onEdit} />
        <IconButton label={`Duplicate ${chart.name}`} icon={<Copy className="h-4 w-4" />} onClick={onDuplicate} />
        <IconButton label={`Export ${chart.name}`} icon={<Download className="h-4 w-4" />} onClick={onExport} />
        {onLock ? <IconButton label={`${chart.isLocked ? 'Unlock' : 'Lock'} ${chart.name}`} icon={<LockKeyhole className="h-4 w-4" />} onClick={onLock} /> : null}
        {onRestore ? <IconButton label={`Restore bundled ${chart.name}`} icon={<RotateCcw className="h-4 w-4" />} onClick={onRestore} /> : null}
        <div className="relative group">
          <IconButton label={`More actions for ${chart.name}`} icon={<Ellipsis className="h-4 w-4" />} onClick={onDelete} />
          <span className="pointer-events-none absolute right-0 top-12 z-10 hidden w-44 rounded-lg border border-border bg-bg-surface p-2 text-xs text-fg-secondary shadow-xl group-focus-within:block">{entry.origin === 'override' ? 'Reset to app default' : 'Delete personal chart'}</span>
        </div>
      </div>
      <div className="absolute bottom-3 right-4 flex items-center gap-2 text-xs font-medium text-fg-secondary"><span>{chart.isEnabled ? 'Enabled' : 'Disabled'}</span><Switch checked={chart.isEnabled} onChange={onToggle} aria-label={`${chart.isEnabled ? 'Disable' : 'Enable'} ${chart.name}`} /></div>
    </article>
  );
}

function OriginBadge({ entry }: { entry: ChartManagerEntry }) {
  return <Badge variant={entry.origin === 'system' ? 'info' : 'default'} size="sm">{entry.origin === 'system' ? 'App default' : entry.origin === 'override' ? 'Customized' : 'My chart'}</Badge>;
}

function statusBadgesForRow(entry: ChartManagerEntry) {
  const chart = entry.effective;
  return <span className="flex flex-wrap items-center gap-1.5">{chart.isLocked ? <Badge variant="warning" size="sm">Locked</Badge> : null}{chart.config.placements.length === 0 ? <Badge variant="default" size="sm">Unassigned</Badge> : null}</span>;
}

const DASHBOARD_VARIANTS: Array<NonNullable<React.ComponentProps<typeof Badge>['variant']>> = ['accent', 'info', 'success', 'warning', 'danger'];

function dashboardVariant(slug: string) {
  const hash = [...slug].reduce((value, character) => ((value * 31) + character.charCodeAt(0)) >>> 0, 0);
  return DASHBOARD_VARIANTS[hash % DASHBOARD_VARIANTS.length] ?? 'default';
}

function DashboardBadge({ slug, name }: { slug: string; name: string }) {
  return <Badge variant={dashboardVariant(slug)} size="md" className="max-w-40"><Tag className="h-3 w-3 shrink-0" aria-hidden="true" /><span className="truncate">{name}</span></Badge>;
}

function IconButton({ label, icon, onClick, compact = false }: { label: string; icon: React.ReactNode; onClick: () => void; compact?: boolean }) {
  return <button type="button" aria-label={label} title={label} onClick={onClick} className={`${compact ? 'h-9 w-9' : 'h-11 w-11'} inline-flex items-center justify-center rounded-lg border border-border text-fg-secondary transition-colors hover:bg-bg-elevated hover:text-fg focus:outline-none focus-visible:ring-2 focus-visible:ring-accent`}>{icon}</button>;
}

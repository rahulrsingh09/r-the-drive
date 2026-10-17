import * as React from 'react';
import { themeDraftTransition } from './themeDraft';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { ArrowLeft, CheckCircle2, Eye, Save, Send, Trash2 } from 'lucide-react';
import { resolveTheme, type ThemeColorPair } from '@panda/themes';
import { queryKeys, themeClient, useAuth } from '@panda/hooks';
import type { ThemePreferencesResponse, ThemeResource } from '@panda/types';
import { applyThemeRuntime, resetThemePreferences } from '@panda/ui/lib/theme';
import { useThemeRuntime } from '@panda/ui/lib/theme';
import { auditThemeContrast } from './themeContrast';
import { Button, Card, CardContent, ColorPicker, Skeleton } from '@panda/ui/primitives';
import { cn } from '@panda/ui/lib/utils';

type StudioSectionId = 'overview' | 'interface' | 'charts' | 'brand' | 'review';
const INTERFACE_TOKENS = [
  ['accent', 'Accent'], ['accent-hover', 'Accent hover'], ['bg-page', 'Page background'], ['bg-surface', 'Surface'],
  ['text-primary', 'Primary text'], ['text-secondary', 'Secondary text'], ['border-default', 'Border'],
  ['status-positive', 'Success'], ['status-warning', 'Warning'], ['status-danger', 'Danger'], ['status-info', 'Information'],
] as const;

export function ThemeStudioPage({ themeId }: { themeId: string }) {
  const userId = useAuth((state) => state.userId);
  return <ThemeStudioSession key={`${userId ?? 'signed-out'}:${themeId}`} themeId={themeId} />;
}

function ThemeStudioSession({ themeId }: { themeId: string }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const userId = useAuth((state) => state.userId);
  const preferenceKey = queryKeys.themePreferences.forUser(userId ?? 'signed-out');
  const catalogKey = queryKeys.themes.catalog(userId ?? 'signed-out');
  const resourceKey = queryKeys.themes.resource(userId ?? 'signed-out', themeId);
  const [section, setSection] = React.useState<StudioSectionId>('overview');
  const [editor, dispatch] = React.useReducer(themeDraftTransition, { owner: themeId, draft: null, etag: '', revision: null });
  const { draft, etag: resourceEtag, revision: publishTarget } = editor;
  const [previewing, setPreviewing] = React.useState(false);
  const [confirmRetire, setConfirmRetire] = React.useState(false);
  const [picker, setPicker] = React.useState<{ kind: 'token' | 'series'; key: string; value: ThemeColorPair } | null>(null);
  const previewActive = React.useRef(false);
  const mounted = React.useRef(true);
  const detail = useQuery({ queryKey: resourceKey, queryFn: () => themeClient.get(themeId) as Promise<ThemeResource>, enabled: !!userId });

  React.useEffect(() => {
    if (detail.data) dispatch({ type: 'load', owner: themeId, resource: detail.data });
  }, [detail.data, themeId]);
  React.useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (!previewActive.current) return;
      previewActive.current = false;
      resetThemePreferences();
      void queryClient.invalidateQueries({ queryKey: queryKeys.themePreferences.all });
    };
  }, [queryClient]);

  const saveRevision = useMutation({
    mutationFn: () => themeClient.saveRevision(themeId, draft!, resourceEtag),
    onSuccess: (result: { etag: string; revision?: number }) => {
      if (!mounted.current) return;
      dispatch({ type: 'saved', ...result });
      void detail.refetch();
    },
  });
  const publish = useMutation({
    mutationFn: (apply: boolean) => {
      const revision = publishTarget;
      if (!revision) throw new Error('Save a revision before publishing.');
      const preferenceEtag = queryClient.getQueryData<ThemePreferencesResponse>(preferenceKey)?.etag;
      return themeClient.publishRevision(themeId, revision, apply, resourceEtag, apply ? preferenceEtag : undefined);
    },
    onSuccess: (result: { etag: string }, apply) => {
      if (!mounted.current) return;
      dispatch({ type: 'saved', ...result });
      void detail.refetch();
      if (apply) void queryClient.invalidateQueries({ queryKey: queryKeys.themePreferences.all });
    },
  });
  const rollback = useMutation({
    mutationFn: (revision: number) => {
      const preferenceEtag = queryClient.getQueryData<ThemePreferencesResponse>(preferenceKey)?.etag;
      if (!preferenceEtag) throw new Error('Appearance preferences are still loading.');
      return themeClient.rollback(themeId, revision, resourceEtag, preferenceEtag);
    },
    onSuccess: (response: ThemePreferencesResponse) => {
      if (mounted.current) queryClient.setQueryData(preferenceKey, response);
    },
  });
  const retire = useMutation({
    mutationFn: () => themeClient.retire(themeId, resourceEtag),
    onSuccess: () => { if (!mounted.current) return; void queryClient.invalidateQueries({ queryKey: catalogKey }); void navigate({ to: '/settings', search: { section: 'appearance' } }); },
  });

  if (detail.isLoading || !draft) return <div className="mx-auto grid max-w-6xl gap-4 p-4 sm:p-6"><Skeleton className="h-14" /><Skeleton className="h-[32rem]" /></div>;
  if (detail.isError || !detail.data) return <div className="mx-auto max-w-3xl p-6"><Card><CardContent className="grid gap-3 p-6"><h1 className="text-lg font-semibold text-fg">Theme unavailable</h1><p className="text-sm text-fg-secondary">This theme may have been retired or belongs to another account.</p><Button className="justify-self-start" variant="secondary" onClick={() => void navigate({ to: '/settings', search: { section: 'appearance' } })}>Back to Appearance</Button></CardContent></Card></div>;

  const resolved = resolveTheme(draft);
  const { primary: criticalContrastFailures, charts: chartWarnings } = auditThemeContrast(resolved);
  const draftRevision = (detail.data.revisions[0]?.revision ?? 0) + 1;
  const togglePreview = () => {
    previewActive.current = !previewActive.current;
    setPreviewing(previewActive.current);
    if (previewActive.current) {
      applyThemeRuntime({ schemaVersion: 2, mode: 'dark', selection: { kind: 'custom', themeId, revision: draftRevision } }, resolved);
      return;
    }
    resetThemePreferences();
    void queryClient.invalidateQueries({ queryKey: queryKeys.themePreferences.all });
  };
  const updatePair = (value: ThemeColorPair) => {
    if (picker) dispatch({ type: 'paint', kind: picker.kind, key: picker.key, value });
  };

  return <div className="min-h-screen bg-bg-page text-fg">
    {previewing ? <div className="sticky top-0 z-50 flex min-h-11 items-center justify-center gap-3 border-b border-accent bg-bg-surface px-4 text-sm shadow-md"><Eye className="h-4 w-4 text-accent" /><span className="font-medium">Full-app draft preview</span><Button size="sm" variant="secondary" onClick={togglePreview}>Exit preview</Button></div> : null}
    <header className="border-b border-border bg-bg-surface">
      <div className="mx-auto flex max-w-7xl items-center justify-between gap-4 px-4 py-4 sm:px-6">
        <div className="flex min-w-0 items-center gap-3"><Button aria-label="Back to Appearance" size="sm" variant="ghost" onClick={() => void navigate({ to: '/settings', search: { section: 'appearance' } })}><ArrowLeft className="h-4 w-4" /></Button><div className="min-w-0"><h1 className="truncate text-lg font-semibold">{detail.data.name}</h1><p className="text-xs text-fg-tertiary">Theme Studio · based on {detail.data.baseThemeId}</p></div></div>
        <div className="flex gap-2"><Button variant="secondary" iconLeft={<Eye className="h-4 w-4" />} onClick={togglePreview}>{previewing ? 'Exit preview' : 'Preview app'}</Button><Button disabled={criticalContrastFailures.length > 0} loading={saveRevision.isPending} iconLeft={<Save className="h-4 w-4" />} onClick={() => saveRevision.mutate()}>Save revision</Button></div>
      </div>
    </header>
    <div className="mx-auto grid max-w-7xl gap-6 px-4 py-6 lg:grid-cols-[13rem_minmax(0,1fr)] lg:px-6">
      <nav className="flex gap-1 overflow-x-auto lg:grid lg:content-start" aria-label="Theme Studio sections">{(['overview', 'interface', 'charts', 'brand', 'review'] as const).map((item) => <button key={item} onClick={() => setSection(item)} className={cn('min-h-10 shrink-0 rounded-lg px-3 text-left text-sm font-medium capitalize outline-none focus-visible:ring-2 focus-visible:ring-accent', section === item ? 'bg-accent-muted text-fg' : 'text-fg-secondary hover:bg-bg-elevated hover:text-fg')}>{item}</button>)}</nav>
      <main className="min-w-0">
        {section === 'overview' ? <StudioSection title="Overview" description="Custom themes inherit every missing value from a built-in theme. Only color and trusted brand paint overrides are stored."><ThemePreview resolved={resolved} /><div className="mt-4 grid gap-2 rounded-xl border border-border bg-bg-elevated/35 p-4 text-sm"><p><span className="text-fg-tertiary">Base:</span> {detail.data.baseThemeId}</p><p><span className="text-fg-tertiary">Published:</span> {detail.data.publishedRevision ?? 'Not yet'}</p><p><span className="text-fg-tertiary">Draft revision:</span> {draftRevision}</p></div></StudioSection> : null}
        {section === 'interface' ? <StudioSection title="Interface" description="Tune semantic roles. Text and surface choices are checked together before publication."><PairGrid items={INTERFACE_TOKENS.map(([key, label]) => ({ key, label, pair: resolvedPair(resolved.tokens, key) }))} onPick={(item) => setPicker({ kind: 'token', ...item })} /></StudioSection> : null}
        {section === 'charts' ? <StudioSection title="Charts" description="Sixteen ordered categorical colors are available to every renderer and chart editor."><PairGrid items={(Object.entries(resolved.series) as Array<[string, ThemeColorPair]>).map(([key, pair], index) => ({ key, label: `Series ${index + 1}`, pair }))} onPick={(item) => setPicker({ kind: 'series', ...item })} /></StudioSection> : null}
        {section === 'brand' ? <StudioSection title="Brand" description="Brand assets inherit from the built-in base. Custom brand colors are not editable yet."><BrandPreview resolved={resolved} /></StudioSection> : null}
        {section === 'review' ? <StudioSection title="Review and publish" description="Saving creates an immutable revision. Publishing a newer revision never moves an active selection unless you choose Publish and apply."><ThemePreview resolved={resolved} />{criticalContrastFailures.length ? <div className="mt-4 rounded-lg border border-status-danger/30 bg-status-danger/10 p-3 text-sm text-status-danger" role="alert"><p className="font-medium">Fix critical interface contrast before saving.</p><ul className="mt-1 list-disc pl-5">{criticalContrastFailures.map((issue) => <li key={issue}>{issue}</li>)}</ul></div> : null}{chartWarnings.length ? <div className="mt-4 rounded-lg border border-status-warning/30 bg-status-warning/10 p-3 text-sm text-status-warning" role="status"><p className="font-medium">Chart colors may be difficult to distinguish.</p><ul className="mt-1 list-disc pl-5">{chartWarnings.slice(0, 4).map((issue) => <li key={issue}>{issue}</li>)}</ul></div> : null}<div className="mt-5 flex flex-wrap gap-2"><Button disabled={criticalContrastFailures.length > 0} variant="secondary" loading={publish.isPending} iconLeft={<Send className="h-4 w-4" />} onClick={() => publish.mutate(false)}>Publish</Button><Button disabled={criticalContrastFailures.length > 0} loading={publish.isPending} iconLeft={<CheckCircle2 className="h-4 w-4" />} onClick={() => publish.mutate(true)}>Publish and apply</Button>{confirmRetire ? <><Button variant="danger" loading={retire.isPending} iconLeft={<Trash2 className="h-4 w-4" />} onClick={() => retire.mutate()}>Confirm retirement</Button><Button variant="secondary" onClick={() => setConfirmRetire(false)}>Keep theme</Button></> : <Button variant="danger" iconLeft={<Trash2 className="h-4 w-4" />} onClick={() => setConfirmRetire(true)}>Retire theme</Button>}</div><RevisionHistory revisions={detail.data.revisions} activeRevision={detail.data.publishedRevision} pending={rollback.isPending} onRollback={(revision) => rollback.mutate(revision)} />{saveRevision.isError || publish.isError || rollback.isError || retire.isError ? <p className="mt-3 text-sm text-status-danger" role="alert">The theme changed elsewhere or could not be saved. Refresh before trying again.</p> : null}</StudioSection> : null}
      </main>
    </div>
    <ColorPicker open={picker !== null} value={picker?.value ?? resolved.brandPaints.accent} onOpenChange={(open) => { if (!open) setPicker(null); }} onApply={updatePair} title={picker ? `Edit ${picker.key}` : 'Edit color'} />
  </div>;
}

function StudioSection({ title, description, children }: { title: string; description: string; children: React.ReactNode }) { return <section><h2 className="text-xl font-semibold text-fg">{title}</h2><p className="mt-1 max-w-2xl text-sm text-fg-secondary">{description}</p><div className="mt-5">{children}</div></section>; }
function PairGrid({ items, onPick }: { items: Array<{ key: string; label: string; pair: ThemeColorPair }>; onPick: (item: { key: string; value: ThemeColorPair }) => void }) { return <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">{items.map((item) => <button key={item.key} type="button" onClick={() => onPick({ key: item.key, value: item.pair })} className="flex min-h-14 items-center gap-3 rounded-xl border border-border bg-bg-elevated/35 px-3 text-left outline-none hover:border-border-strong focus-visible:ring-2 focus-visible:ring-accent"><span className="flex h-8 w-12 overflow-hidden rounded-lg border border-border"><span className="flex-1" style={{ backgroundColor: item.pair.light }} /><span className="flex-1" style={{ backgroundColor: item.pair.dark }} /></span><span className="min-w-0"><span className="block truncate text-sm font-medium text-fg">{item.label}</span><span className="block truncate font-mono text-[10px] text-fg-tertiary">{item.pair.light} · {item.pair.dark}</span></span></button>)}</div>; }
function resolvedPair(tokens: { light: Record<string, string>; dark: Record<string, string> }, key: string): ThemeColorPair { return { light: tokens.light[key]!, dark: tokens.dark[key]! }; }
function ThemePreview({ resolved }: { resolved: ReturnType<typeof resolveTheme> }) { const style = Object.fromEntries(Object.entries(resolved.tokens.dark).map(([key, value]) => [`--rm-${key}`, value])) as React.CSSProperties; const series = Object.values(resolved.series) as ThemeColorPair[]; return <div style={style} className="overflow-hidden rounded-xl border border-border bg-bg-page p-4"><div className="grid gap-3 sm:grid-cols-[1fr_1.3fr]"><div className="rounded-lg border border-border bg-bg-surface p-4"><div className="mb-3 h-3 w-24 rounded bg-accent" /><p className="text-sm font-semibold text-fg">Interface preview</p><p className="mt-1 text-xs text-fg-secondary">Semantic surfaces, text, borders, and status colors.</p><Button className="mt-4" size="sm">Primary action</Button></div><div className="rounded-lg border border-border bg-bg-surface p-4"><p className="mb-3 text-xs font-medium text-fg-secondary">Chart palette</p><div className="flex h-24 items-end gap-1">{series.slice(0, 16).map((pair, index) => <span key={index} className="min-w-0 flex-1 rounded-t" style={{ height: `${30 + (index * 23) % 70}%`, backgroundColor: pair.dark }} />)}</div></div></div></div>; }
function BrandPreview({ resolved }: { resolved: ReturnType<typeof resolveTheme> }) {
  const { effectiveMode } = useThemeRuntime();
  const isVector = resolved.sourceTheme === 'rad';
  return (
    <div className="grid gap-4 rounded-xl border border-border bg-bg-elevated/35 p-4 sm:grid-cols-2">
      <div className="grid min-h-36 place-items-center rounded-lg border border-border bg-bg-page p-5">
        <img src={resolved.brandAssets.wordmark[effectiveMode]} alt="Panda wordmark preview" className="max-h-16 max-w-full" />
      </div>
      <div className="grid content-center gap-2">
        <p className="text-sm font-medium text-fg">{isVector ? 'RAD vector artwork' : 'Raster-backed fallback'}</p>
        <p className="text-xs leading-relaxed text-fg-secondary">
          {isVector
            ? 'RAD includes a new vector logo, wordmark, icon, and favicon in gold, vermilion, and teal. Artwork stays sharp at every size.'
            : 'Classic retains its original raster-backed logo and wordmark.'}
          {' '}Brand artwork follows the base theme. Custom brand colors are not editable yet.
        </p>
      </div>
    </div>
  );
}
function RevisionHistory({ revisions, activeRevision, pending, onRollback }: { revisions: ThemeResource['revisions']; activeRevision: number | null; pending: boolean; onRollback: (revision: number) => void }) { const published = revisions.filter((revision) => revision.publishedAt !== null); if (!published.length) return null; return <div className="mt-5 rounded-xl border border-border bg-bg-elevated/35 p-4"><h3 className="text-sm font-semibold text-fg">Published history</h3><div className="mt-3 grid gap-2">{published.map((revision) => <div key={revision.revision} className="flex min-h-11 items-center justify-between gap-3 rounded-lg border border-border bg-bg-surface px-3"><span className="text-sm text-fg">Revision {revision.revision}{revision.revision === activeRevision ? <span className="ml-2 text-xs text-fg-tertiary">latest publication</span> : null}</span><Button size="sm" variant="secondary" disabled={pending} onClick={() => onRollback(revision.revision)}>Apply revision</Button></div>)}</div></div>; }

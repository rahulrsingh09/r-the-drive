import { useSearchPicker } from './useSearchPicker';
import { editChartDefinition } from './definitionEdit';
import { applySeriesChoice, addSeriesFromChoice } from './curveTransaction';
import React from 'react';
import { chartDraftInitial, chartDraftProblems, chartDraftTransition } from './chartDraft';
import { RMark } from '../../r-experience/RMark';
import { useNavigate } from '@tanstack/react-router';
import {
  CHART_SOURCE_MANIFESTS,
  ManagedChartRuntime,
  ChartDefinitionV1Schema,
  downloadChartJson,
  getBundledChartDefinition,
  resolveChartSourceCapabilities,
  buildCurveCatalog,
  isMixedDomain,
  removeCurveAndUnusedSources,
  supportsBundledRendererCurve,
  supportsBundledRichEditing,
} from '@panda/dashboards';
import {
  useChartManager,
  useChartSources,
  useMetricCatalog,
  useResolvedVehicleSelection,
  useCreateChart,
  useUpdateChart,
} from '@panda/hooks';
import type {
  ChartAxisDefinition,
  ChartDefinitionV1,
  ChartMark,
  ChartRecord,
  ChartSeriesDefinition,
  ChartSourceManifest,
  MetricCatalogEntry,
} from '@panda/types';
import { ChartColorField } from '@panda/ui/charts';
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Switch,
} from '@panda/ui/primitives';
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronUp,
  Download,
  Filter,
  Save,
  Search,
  Trash2,
} from 'lucide-react';

type EditorSection = 'basics' | 'curves' | 'display' | 'advanced';

const MARKS: ChartMark[] = ['line', 'step', 'bar', 'scatter', 'histogram'];

function copyDefinition(definition: ChartDefinitionV1): ChartDefinitionV1 {
  return structuredClone(definition);
}

function defaultDefinition(): ChartDefinitionV1 {
  const bundled = getBundledChartDefinition('soc-history');
  if (!bundled) throw new Error('Bundled chart defaults are unavailable');
  const { slug, title, description, ...definition } = bundled;
  void slug;
  void title;
  void description;
  return copyDefinition(ChartDefinitionV1Schema.parse(definition) as ChartDefinitionV1);
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : 'Unable to save chart.';
}

function previewRange(policy: ChartDefinitionV1['timeframe']) {
  if (policy.mode === 'lifetime') return { from: null, to: null, lifetime: true };
  const days =
    policy.mode === 'relative'
      ? ({ '1h': 1 / 24, '6h': 0.25, '24h': 1, '7d': 7, '30d': 30, '90d': 90, '1y': 365 }[
          policy.preset
        ] ?? 30)
      : 30;
  const to = new Date();
  const from = new Date(to.getTime() - days * 86_400_000);
  return { from: from.toISOString(), to: to.toISOString(), lifetime: false };
}

export function ChartEditorPage(props: { mode: 'new' | 'edit'; chartId?: string }) {
  return <ChartEditorSession key={`${props.mode}:${props.chartId ?? ''}`} {...props} />;
}

function ChartEditorSession({ mode, chartId }: { mode: 'new' | 'edit'; chartId?: string }) {
  const navigate = useNavigate();
  const manager = useChartManager();
  const sources = useChartSources();
  const metricCatalog = useMetricCatalog();
  const createChart = useCreateChart();
  const updateChart = useUpdateChart();
  const [section, setSection] = React.useState<EditorSection>('basics');
  const [editor, dispatch] = React.useReducer(chartDraftTransition, null, () => chartDraftInitial(mode === 'new' ? {
    id: '', ownerId: null, slug: 'new-chart', name: 'New chart', isDefault: false,
    isLocked: false, isEnabled: true, config: { ...defaultDefinition(), placements: [] },
  } : null));
  const { draft, dirty, advancedText, saveError, sourceNotice } = editor;
  const setSaveError = (message: string | null) => dispatch({ type: 'error', message });
  const setSourceNotice = (message: string | null) => dispatch({ type: 'notice', message });
  const [discardRequested, setDiscardRequested] = React.useState(false);
  const discardDialogRef = React.useRef<HTMLDivElement | null>(null);
  const mounted = React.useRef(true);
  React.useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  const entry = manager.data?.find((candidate) => candidate.effective.id === chartId);
  const manifests = React.useMemo(
    () =>
      resolveChartSourceCapabilities(
        sources.data && sources.data.some((source) => source.fields.length > 0)
          ? sources.data
          : [...CHART_SOURCE_MANIFESTS],
        metricCatalog.data ?? []
      ),
    [metricCatalog.data, sources.data]
  );
  const { effectiveVehicleId } = useResolvedVehicleSelection();
  const previewTimeframe = draft?.config.timeframe;
  const previewPreset = previewTimeframe?.mode === 'relative' ? previewTimeframe.preset : undefined;
  const previewContext = React.useMemo(
    () => previewRange(previewTimeframe ?? { mode: 'dashboard' }),
    [previewTimeframe?.mode, previewPreset]
  );
  React.useEffect(() => {
    if (mode === 'edit' && entry) dispatch({ type: 'load', record: entry.effective });
  }, [dirty, entry, mode]);

  React.useEffect(() => {
    if (!dirty) return;
    const controller = new AbortController();
    window.addEventListener('beforeunload', (event) => {
      event.preventDefault();
      event.returnValue = '';
    }, { signal: controller.signal });
    return () => controller.abort();
  }, [dirty]);

  React.useEffect(() => {
    if (!discardRequested) return;
    const controller = new AbortController();
    const restoreOverflow = document.body.style.overflow;
    const restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = discardDialogRef.current;
    document.body.style.overflow = 'hidden';
    dialog?.querySelector<HTMLElement>('button')?.focus();
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') { setDiscardRequested(false); return; }
      if (event.key !== 'Tab' || !dialog) return;
      const controls = dialog.querySelectorAll<HTMLElement>('button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])');
      const boundary = event.shiftKey ? 0 : controls.length - 1;
      if (!controls.length || document.activeElement !== controls[boundary]) return;
      event.preventDefault();
      controls[event.shiftKey ? controls.length - 1 : 0]?.focus();
    }, { signal: controller.signal });
    return () => {
      controller.abort();
      document.body.style.overflow = restoreOverflow;
      restoreFocus?.focus();
    };
  }, [discardRequested]);

  const validationErrors = React.useMemo(() => chartDraftProblems(editor, manifests), [editor, manifests]);

  function updateDraft(patch: Partial<ChartRecord>) {
    dispatch({ type: 'record', patch });
  }

  function updateDefinition(update: (definition: ChartDefinitionV1) => ChartDefinitionV1) {
    dispatch({ type: 'definition', update });
  }

  function goBack() {
    if (dirty) {
      setDiscardRequested(true);
      return;
    }
    navigate({ to: '/settings', search: { section: 'charts' } });
  }

  function discardAndGoBack() {
    setDiscardRequested(false);
    dispatch({ type: 'saved' });
    navigate({ to: '/settings', search: { section: 'charts' } });
  }

  async function save() {
    if (!draft) return;
    if (validationErrors.length > 0) {
      setSaveError(
        `Fix ${validationErrors.length} validation error${validationErrors.length === 1 ? '' : 's'} before saving.`
      );
      const firstPath = validationErrors[0]?.path ?? '';
      setSection(
        firstPath === 'advanced'
          ? 'advanced'
          : firstPath.startsWith('config.sources') || firstPath.startsWith('config.series')
            ? 'curves'
            : 'display'
      );
      return;
    }
    const body = {
      slug: draft.slug, name: draft.name,
      ...(draft.description !== undefined ? { description: draft.description } : {}),
      isEnabled: draft.isEnabled, config: draft.config,
    };
    try {
      await (mode === 'new' || entry?.origin === 'system'
        ? createChart.mutateAsync(body)
        : updateChart.mutateAsync({ id: draft.id, ...body }));
      if (!mounted.current) return;
      dispatch({ type: 'saved' });
      navigate({ to: '/settings', search: { section: 'charts' } });
    } catch (error) {
      if (mounted.current) setSaveError(errorText(error));
    }
  }

  if (!draft) {
    return (
      <div className="grid min-h-screen place-items-center bg-bg-page p-5 text-sm text-fg-secondary">
        Loading chart editor…
      </div>
    );
  }

  return (
    <div className="r-editor min-h-screen bg-bg-page text-fg">
      <header className="sticky top-0 z-30 border-b border-border bg-bg-page/95 px-4 py-3 backdrop-blur md:px-6">
        <div className="mx-auto flex max-w-[1500px] items-center gap-3">
          <RMark />
          <button
            type="button"
            aria-label="Back to charts"
            title="Back to charts"
            onClick={goBack}
            className="inline-flex h-[2.125rem] w-[2.125rem] shrink-0 items-center justify-center rounded-lg border border-accent bg-bg-surface text-accent transition-colors hover:bg-accent/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <ArrowLeft className="h-5 w-5" />
          </button>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="truncate text-base font-semibold">{draft.name}</h1>
              <Badge variant={entry?.origin === 'system' ? 'info' : 'default'} size="sm">
                {entry?.origin === 'system'
                  ? 'Customize app default'
                  : mode === 'new'
                    ? 'New chart'
                    : 'My chart'}
              </Badge>
              {dirty ? <span className="text-xs text-fg-tertiary">Unsaved changes</span> : null}
            </div>
            <p className="truncate text-xs text-fg-tertiary">{draft.slug}</p>
          </div>
          <Button
            variant="secondary"
            size="sm"
            iconLeft={<Download className="h-4 w-4" />}
            onClick={() => {
              if (draft.id) downloadChartJson(draft);
              else setSaveError('Save the chart before exporting it.');
            }}
          >
            Export
          </Button>
          <Button
            variant="primary"
            size="sm"
            iconLeft={<Save className="h-4 w-4" />}
            loading={createChart.isPending || updateChart.isPending}
            disabled={validationErrors.length > 0}
            onClick={() => {
              void save();
            }}
          >
            Save
          </Button>
        </div>
      </header>
      <main className="mx-auto grid max-w-[1500px] gap-5 p-4 md:p-6 lg:grid-cols-[minmax(0,1fr)_minmax(22rem,30rem)] lg:items-start">
        <PreviewPanel
          draft={draft}
          errors={validationErrors}
          previewContext={previewContext}
          vehicleId={effectiveVehicleId}
        />
        <section className="min-w-0 lg:col-start-2 lg:row-start-1">
          <nav
            aria-label="Chart editor sections"
            className="mb-4 flex gap-1 overflow-x-auto rounded-xl border border-border bg-bg-surface p-1"
          >
            {(['basics', 'curves', 'display', 'advanced'] as EditorSection[]).map((item) => (
              <button
                key={item}
                type="button"
                onClick={() => setSection(item)}
                className={`min-h-10 shrink-0 rounded-lg px-3 text-left text-sm capitalize transition-colors ${section === item ? 'bg-bg-elevated font-medium text-fg' : 'text-fg-secondary hover:bg-bg-elevated/70'}`}
              >
                {item}
              </button>
            ))}
          </nav>
          {saveError ? (
            <div
              role="alert"
              className="mb-4 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger"
            >
              {saveError}
            </div>
          ) : null}
          {sourceNotice ? (
            <div
              role="status"
              className="mb-4 rounded-lg border border-accent/30 bg-accent/10 px-3 py-2 text-sm text-fg-secondary"
            >
              {sourceNotice}
            </div>
          ) : null}
          {section === 'basics' && <BasicsSection draft={draft} onChange={updateDraft} />}
          {section === 'curves' && (
            <CurvesSection
              chartSlug={draft.slug}
              definition={draft.config}
              manifests={manifests}
              metrics={metricCatalog.data ?? []}
              onNotice={setSourceNotice}
              onChange={updateDefinition}
            />
          )}
          {section === 'display' && (
            <DisplaySection
              definition={draft.config}
              onChange={updateDefinition}
              richEditing={supportsBundledRichEditing(draft.slug) !== false}
            />
          )}
          {section === 'advanced' && (
            <AdvancedSection
              value={advancedText}
              onChange={(text) => dispatch({ type: 'json', text })}
            />
          )}
        </section>
      </main>
      {discardRequested ? (
        <div className="fixed inset-0 z-[70] flex items-stretch justify-center bg-bg-page/85 sm:items-center sm:p-4">
          <div
            ref={discardDialogRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="discard-chart-title"
            className="flex min-h-[100dvh] w-full flex-col bg-bg-surface p-[max(1.25rem,env(safe-area-inset-top))] shadow-lg sm:min-h-0 sm:max-w-md sm:rounded-xl sm:border sm:border-border sm:p-5"
          >
            <h2 id="discard-chart-title" className="text-base font-semibold">
              Discard unsaved changes?
            </h2>
            <p className="mt-2 text-sm text-fg-tertiary">Your chart draft will not be saved.</p>
            <div className="mt-auto flex flex-col-reverse gap-2 pt-6 sm:mt-5 sm:flex-row sm:justify-end">
              <Button
                type="button"
                variant="secondary"
                size="md"
                onClick={() => setDiscardRequested(false)}
              >
                Keep editing
              </Button>
              <Button type="button" variant="danger" size="md" onClick={discardAndGoBack}>
                Discard changes
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function BasicsSection({
  draft,
  onChange,
}: {
  draft: ChartRecord;
  onChange: (patch: Partial<ChartRecord>) => void;
}) {
  return (
    <EditorCard
      title="Basics"
      description="Stable slugs keep favorites and dashboard settings attached to the same chart."
    >
      <div className="grid gap-4">
        <EditorField label="Name">
          <input
            value={draft.name}
            onChange={(event) => onChange({ name: event.target.value })}
            className="editor-input"
          />
        </EditorField>
        <EditorField label="Slug" hint="Locked after first save in the production editor.">
          <input
            value={draft.slug}
            disabled={!!draft.id}
            onChange={(event) => onChange({ slug: event.target.value })}
            className="editor-input disabled:opacity-60"
          />
        </EditorField>
        <EditorField label="Description">
          <textarea
            value={draft.description ?? ''}
            onChange={(event) => onChange({ description: event.target.value })}
            rows={3}
            className="editor-input py-2"
          />
        </EditorField>
        <ToggleRow
          label="Enabled"
          description="Disabled charts remain saved but are hidden from assigned dashboards."
          checked={draft.isEnabled}
          onCheckedChange={(checked) => onChange({ isEnabled: checked })}
        />
      </div>
    </EditorCard>
  );
}

function CurvesSection({
  chartSlug,
  definition,
  manifests,
  metrics,
  onNotice,
  onChange,
}: {
  chartSlug: string;
  definition: ChartDefinitionV1;
  manifests: ChartSourceManifest[];
  metrics: MetricCatalogEntry[];
  onNotice: (notice: string | null) => void;
  onChange: (mutator: (definition: ChartDefinitionV1) => ChartDefinitionV1) => void;
}) {
  const [query, setQuery] = React.useState('');
  const [availableOnly, setAvailableOnly] = React.useState(true);
  const bundledRichEditing = supportsBundledRichEditing(chartSlug);
  const richEditing = bundledRichEditing !== false;
  const isBundled = bundledRichEditing !== undefined;
  const catalog = rendererCompatibleCatalog(
    chartSlug,
    buildCurveCatalog(manifests, metrics, definition)
  );
  const mixed = isMixedDomain(definition, manifests);
  const normalizedQuery = query.toLowerCase();
  const filtered: typeof catalog = [];
  for (const option of catalog) {
    if (availableOnly && (mixed || !option.enabled)) continue;
    const text = [option.label, option.category, option.description].join(' ').toLowerCase();
    if (text.includes(normalizedQuery)) filtered.push(option);
  }

  return (
    <div className="grid gap-4">
      <EditorCard
        title="Curves"
        description={
          richEditing
            ? 'Build the chart from searchable sensors and values. Each curve keeps its own mark, color, and axis.'
            : 'This production renderer owns its curve geometry. You can change its color without changing the established chart type.'
        }
      >
        <div className="grid gap-4">
          {!isBundled ? <EditorField label="Timeframe">
            <select
              value={
                definition.timeframe.mode === 'relative'
                  ? definition.timeframe.preset
                  : definition.timeframe.mode
              }
              onChange={(event) => {
                const value = event.target.value;
                onChange((current) => editChartDefinition(current, { type: 'timeframe', value }));
              }}
              className="editor-input"
            >
              <option value="dashboard">Dashboard range</option>
              <option value="24h">Last 24 hours</option>
              <option value="7d">Last 7 days</option>
              <option value="30d">Last 30 days</option>
              <option value="90d">Last 90 days</option>
              <option value="1y">Last year</option>
              <option value="lifetime">Lifetime</option>
            </select>
          </EditorField> : (
            <p className="rounded-lg border border-border bg-bg-elevated/20 p-3 text-xs text-fg-tertiary">
              Bundled charts inherit the active dashboard timeframe so the editor preview and dashboard always use the same range.
            </p>
          )}
          {mixed ? (
            <p className="rounded-lg border border-warning/30 bg-warning/10 p-3 text-xs text-warning">
              This chart uses mixed domains. Use Advanced to edit its exact field bindings.
            </p>
          ) : null}
          {richEditing ? <details className="rounded-lg border border-border bg-bg-elevated/20 p-3">
            <summary className="cursor-pointer text-sm font-medium">Add curve</summary>
            <div className="mt-3 grid gap-2">
              <div className="flex items-center gap-2">
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Search curves"
                  aria-label="Search curves"
                  className="editor-input min-w-0 flex-1"
                />
                <button
                  type="button"
                  aria-label={
                    availableOnly ? 'Showing available curves only' : 'Showing all curves'
                  }
                  title={availableOnly ? 'Show all curves' : 'Show available curves only'}
                  aria-pressed={availableOnly}
                  onClick={() => setAvailableOnly((current) => !current)}
                  className={`icon-button shrink-0 ${availableOnly ? 'border-accent bg-accent/10 text-accent' : 'text-fg-tertiary'}`}
                >
                  <Filter className="h-4 w-4" aria-hidden="true" />
                </button>
              </div>
              <div className="grid gap-1">
                {filtered.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    disabled={!option.enabled || mixed}
                    title={option.disabledReason}
                    className="flex min-h-11 items-center justify-between rounded-md px-3 text-left text-sm hover:bg-bg-elevated disabled:cursor-not-allowed disabled:opacity-45"
                    onClick={() =>
                      onChange((current) =>
                        addSeriesFromChoice(current, option.id, manifests, metrics, onNotice)
                      )
                    }
                  >
                    <span>
                      <span className="block">{option.label}</span>
                      <span className="block text-xs text-fg-tertiary">
                        {option.category} · {option.description}
                      </span>
                      {!option.enabled || mixed ? (
                        <span className="block text-xs text-warning">
                          {mixed
                            ? 'Mixed-domain chart; edit in Advanced.'
                            : (option.disabledReason ?? 'Unavailable for this chart.')}
                        </span>
                      ) : null}
                    </span>
                  </button>
                ))}
                {filtered.length === 0 ? (
                  <p className="px-3 py-4 text-sm text-fg-tertiary">
                    {availableOnly
                      ? 'No available curves match your search.'
                      : 'No matching curves.'}
                  </p>
                ) : null}
              </div>
            </div>
          </details> : null}
          <CurvesListSection
            chartSlug={chartSlug}
            definition={definition}
            manifests={manifests}
            metrics={metrics}
            onNotice={onNotice}
            onChange={onChange}
          />
        </div>
      </EditorCard>
    </div>
  );
}

function CurvesListSection({
  chartSlug,
  definition,
  manifests,
  metrics,
  onNotice,
  onChange,
}: {
  chartSlug: string;
  definition: ChartDefinitionV1;
  manifests: ChartSourceManifest[];
  metrics: MetricCatalogEntry[];
  onNotice: (notice: string | null) => void;
  onChange: (mutator: (definition: ChartDefinitionV1) => ChartDefinitionV1) => void;
}) {
  const mixed = isMixedDomain(definition, manifests);
  const richEditing = supportsBundledRichEditing(chartSlug) !== false;
  const replacementDefinition =
    definition.series.length === 1 ? { ...definition, series: [] } : definition;
  const catalog = rendererCompatibleCatalog(
    chartSlug,
    buildCurveCatalog(manifests, metrics, replacementDefinition)
  );
  const bindings = new Map(definition.sources.map((binding) => [binding.id, binding]));
  const sourceManifests = new Map(manifests.map((manifest) => [manifest.id, manifest]));
  const valueOptions: SearchOption[] = [];
  for (const option of catalog) {
    const row: SearchOption = { value: option.id, label: option.label, description: `${option.category} · ${option.description}`, disabled: mixed || !option.enabled };
    if (option.disabledReason) row.disabledReason = option.disabledReason;
    if (mixed) row.disabledReason = 'Mixed-domain chart; edit exact values in Advanced.';
    valueOptions.push(row);
  }
  return (
    <div className="grid gap-3">
      {definition.series.map((series, index) => {
        const binding = bindings.get(series.y.sourceBindingId);
        const manifest = sourceManifests.get(binding?.sourceId ?? '');
        const inheritanceKeys: Array<'vehicle' | 'timeframe' | 'tripTags'> = ['vehicle', 'timeframe'];
        if (manifest?.supportsTripTagInheritance) inheritanceKeys.push('tripTags');
        return (
          <div key={series.id} className="grid gap-3 rounded-xl border border-border p-3">
            <div className="flex items-center justify-between gap-2">
              <strong className="text-sm">Curve {index + 1}</strong>
              {richEditing ? <div className="flex gap-1">
                <button
                  type="button"
                  aria-label="Move curve up"
                  disabled={index === 0}
                  onClick={() =>
                    onChange((current) => editChartDefinition(current, { type: 'move-series', from: index, to: index - 1 }))
                  }
                  className="icon-button"
                >
                  <ChevronUp className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  aria-label="Move curve down"
                  disabled={index === definition.series.length - 1}
                  onClick={() =>
                    onChange((current) => editChartDefinition(current, { type: 'move-series', from: index, to: index + 1 }))
                  }
                  className="icon-button"
                >
                  <ChevronDown className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  aria-label="Remove curve"
                  disabled={definition.series.length === 1}
                  onClick={() => onChange((current) => removeCurveAndUnusedSources(current, index))}
                  className="icon-button text-danger"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div> : null}
            </div>
            {richEditing ? <EditorField label="Value">
              <SearchPicker
                value={choiceValueForSeries(series, definition)}
                options={valueOptions}
                onChange={(choice) =>
                  onChange((current) =>
                    applySeriesChoice(current, index, choice, manifests, metrics, onNotice)
                  )
                }
                ariaLabel={`Curve ${index + 1} value`}
              />
            </EditorField> : null}
            {richEditing ? <EditorField label="Label">
              <input
                value={series.label}
                onChange={(event) => patchSeries(onChange, index, { label: event.target.value })}
                className="editor-input"
              />
            </EditorField> : null}
            <div className="grid gap-3 sm:grid-cols-2">
              {richEditing ? <EditorField label="Mark">
                <select
                  value={series.mark === 'area' ? 'line' : series.mark}
                  onChange={(event) =>
                    onChange((current) => editChartDefinition(current, { type: 'mark', index, mark: event.target.value as ChartMark }))
                  }
                  className="editor-input"
                >
                  {MARKS.map((mark) => (
                    <option key={mark} value={mark}>
                      {mark}
                    </option>
                  ))}
                </select>
              </EditorField> : null}
              {richEditing ? <EditorField label="Axis">
                <select
                  value={series.yAxis}
                  onChange={(event) =>
                    patchSeries(onChange, index, { yAxis: event.target.value as 'y' | 'y2' })
                  }
                  className="editor-input"
                >
                  <option value="y">Left axis</option>
                  <option value="y2">Right axis</option>
                </select>
              </EditorField> : null}
              <ChartColorField
                className="sm:col-span-2"
                value={series.color}
                onChange={(color) => patchSeries(onChange, index, { color })}
              />
            </div>
            {series.mark === 'line' || series.mark === 'step' || series.mark === 'area' ? (
              <ToggleRow
                label="Fill under curve"
                description="Shade the area between this curve and its axis baseline."
                checked={series.mark === 'area' || series.fill === true}
                onCheckedChange={(checked) =>
                  onChange((current) => editChartDefinition(current, { type: 'fill', index, enabled: checked }))
                }
              />
            ) : null}
            <ToggleRow
              label="Show in legend"
              checked={series.visibleInLegend !== false}
              onCheckedChange={(checked) =>
                patchSeries(onChange, index, { visibleInLegend: checked })
              }
            />
            <details className="rounded-lg border border-border bg-bg-elevated/20 p-2">
              <summary className="cursor-pointer text-sm font-medium">Data settings</summary>
              <div className="mt-3 grid gap-3">
                {binding ? (
                  <>
                    {inheritanceKeys.map((key) => (
                      <ToggleRow
                        key={key}
                        label={
                          key === 'tripTags'
                            ? 'Use trip filters'
                            : key === 'vehicle'
                              ? 'Use active vehicle'
                              : 'Use selected timeframe'
                        }
                        checked={binding.inherit[key] ?? false}
                        onCheckedChange={(value) => onChange((current) => editChartDefinition(current, { type: 'inheritance', bindingId: binding.id, key, value }))}
                      />
                    ))}
                    {manifest?.requiredContext.includes('chargeSession') ? (
                      <p className="rounded-lg border border-border bg-bg-elevated/30 p-3 text-xs text-fg-tertiary">
                        Uses the active charging session when this chart is viewed in a
                        charging-session context.
                      </p>
                    ) : null}
                  </>
                ) : (
                  <p className="text-xs text-warning">
                    This curve's data settings are unavailable. Choose another value or repair it in
                    Advanced.
                  </p>
                )}
              </div>
            </details>
          </div>
        );
      })}
    </div>
  );
}

function DisplaySection({
  definition,
  onChange,
  richEditing,
}: {
  definition: ChartDefinitionV1;
  onChange: (mutator: (definition: ChartDefinitionV1) => ChartDefinitionV1) => void;
  richEditing: boolean;
}) {
  if (!richEditing) {
    return (
      <EditorCard
        title="Display and interaction"
        description="This chart keeps its established production display and interaction contract. Use Curves to change its color."
      >
        <p className="text-sm text-fg-secondary">
          Axis labels, curve geometry, legends, grids, tooltips, and interaction behavior are fixed for this specialized renderer so the preview and dashboard cannot diverge.
        </p>
      </EditorCard>
    );
  }
  return (
    <EditorCard
      title="Display and interaction"
      description="Preview the exact labels, marks, and interaction settings that will be saved."
    >
      <div className="grid gap-5">
        <div className="grid gap-4">
          <h3 className="text-sm font-semibold text-fg">Axis labels</h3>
          <EditorField label="X axis label">
            <input
              value={definition.axes.x.label ?? ''}
              onChange={(event) => patchAxis(onChange, 'x', { label: event.target.value })}
              className="editor-input"
            />
          </EditorField>
          <EditorField label="Y axis label">
            <input
              value={definition.axes.y.label ?? ''}
              onChange={(event) => patchAxis(onChange, 'y', { label: event.target.value })}
              className="editor-input"
            />
          </EditorField>
          <ToggleRow
            label="Right Y axis"
            description="Adds a separately labeled scale for series assigned to the right axis."
            checked={!!definition.axes.y2}
            onCheckedChange={(enabled) => onChange((current) => editChartDefinition(current, { type: 'right-axis', enabled }))}
          />
          {definition.axes.y2 ? (
            <EditorField label="Right Y axis label">
              <input
                value={definition.axes.y2.label ?? ''}
                onChange={(event) => patchAxis(onChange, 'y2', { label: event.target.value })}
                className="editor-input"
              />
            </EditorField>
          ) : null}
        </div>
        <div className="border-t border-border pt-5">
          <h3 className="mb-4 text-sm font-semibold text-fg">Ranges</h3>
          <div className="grid gap-3">
            {(['x', 'y', ...(definition.axes.y2 ? ['y2'] : [])] as Array<'x' | 'y' | 'y2'>).map(
              (axis) => (
                <AxisRangeEditor
                  key={axis}
                  axis={axis}
                  definition={definition}
                  onChange={onChange}
                />
              )
            )}
          </div>
        </div>
        <div className="grid gap-4 border-t border-border pt-5">
          <div className="grid gap-3 sm:grid-cols-2">
            <EditorField label="Legend">
              <select
                value={definition.display.legend}
                onChange={(event) =>
                  onChange((current) => editChartDefinition(current, { type: 'display', patch: { legend: event.target.value as ChartDefinitionV1['display']['legend'] } }))
                }
                className="editor-input"
              >
                <option value="auto">Auto</option>
                <option value="show">Show</option>
                <option value="hide">Hide</option>
              </select>
            </EditorField>
            <EditorField label="Curve smoothness">
              <select
                value={definition.display.curveSmoothness}
                onChange={(event) =>
                  onChange((current) => editChartDefinition(current, { type: 'display', patch: { curveSmoothness: event.target.value as ChartDefinitionV1['display']['curveSmoothness'] } }))
                }
                className="editor-input"
              >
                <option value="straight">Straight</option>
                <option value="gentle">Gentle</option>
                <option value="smooth">Smooth</option>
              </select>
            </EditorField>
          </div>
          <ToggleRow
            label="Show grid"
            checked={definition.display.grid}
            onCheckedChange={(checked) =>
              onChange((current) => editChartDefinition(current, { type: 'display', patch: { grid: checked } }))
            }
          />
          <ToggleRow
            label="Enable tooltip"
            checked={definition.display.tooltip}
            onCheckedChange={(checked) =>
              onChange((current) => editChartDefinition(current, { type: 'display', patch: { tooltip: checked } }))
            }
          />
        </div>
      </div>
    </EditorCard>
  );
}

function AdvancedSection({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <EditorCard
      title="Advanced definition"
      description="JSON is validated by the same schema used for visual controls. Unsupported executable content is rejected."
    >
      <textarea
        value={value}
        onChange={(event) => onChange(event.target.value)}
        spellCheck={false}
        rows={24}
        className="min-h-[30rem] w-full rounded-lg border border-border bg-bg-elevated p-3 font-mono text-xs text-fg outline-none focus:border-accent"
      />
    </EditorCard>
  );
}

function PreviewPanel({
  draft,
  errors,
  previewContext,
  vehicleId,
}: {
  draft: ChartRecord;
  errors: Array<{ path: string; message: string }>;
  previewContext: { from: string | null; to: string | null; lifetime: boolean };
  vehicleId: string | null;
}) {
  const totalIssues = errors.length;
  const ctx = { vehicleId, from: previewContext.from, to: previewContext.to };
  const contextLabel = previewContext.lifetime
    ? 'Active vehicle · Lifetime'
    : `Active vehicle · ${previewContext.from?.slice(0, 10) ?? 'range start'} to ${previewContext.to?.slice(0, 10) ?? 'range end'}`;
  return (
    <aside className="grid content-start gap-4 lg:sticky lg:top-24 lg:self-start">
      <Card>
        <CardHeader>
          <CardTitle>Live preview</CardTitle>
          <Badge variant={totalIssues ? 'warning' : 'success'} size="sm">
            {totalIssues ? `${totalIssues} issue${totalIssues === 1 ? '' : 's'}` : 'Valid'}
          </Badge>
        </CardHeader>
        <CardContent>
          <div className="mx-auto w-[95%]">
            <ManagedChartRuntime chart={draft} ctx={ctx} height={520} />
          </div>
          <p className="mt-3 text-xs text-fg-tertiary">
            {contextLabel}. This preview uses the unsaved draft and the production chart renderer.
          </p>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Diagnostics</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-2 text-xs">
          {errors.map((error) => (
            <div
              key={`${error.path}-${error.message}`}
              className="rounded-lg border border-danger/30 bg-danger/10 p-2"
            >
              <strong className="text-danger">{error.path}</strong>
              <p className="mt-1 text-fg-secondary">{error.message}</p>
            </div>
          ))}
          {totalIssues === 0 ? (
            <>
              <div className="flex items-center gap-2 text-success">
                <Check className="h-4 w-4" /> Production chart renderer
              </div>
              <div className="flex items-center gap-2 text-success">
                <Check className="h-4 w-4" /> {draft.config.series.length} curves
              </div>
            </>
          ) : null}
        </CardContent>
      </Card>
    </aside>
  );
}

function EditorCard({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>{title}</CardTitle>
          <p className="mt-1 text-sm text-fg-tertiary">{description}</p>
        </div>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}
function EditorField({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="grid gap-1">
      <span className="text-xs font-medium uppercase tracking-wide text-fg-tertiary">{label}</span>
      {children}
      {hint ? <span className="text-xs text-fg-tertiary">{hint}</span> : null}
    </label>
  );
}
function ToggleRow({
  label,
  description,
  checked,
  onCheckedChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex min-h-11 items-center justify-between gap-4 rounded-lg border border-border bg-bg-elevated/30 px-3 py-2">
      <div>
        <p className="text-sm font-medium text-fg">{label}</p>
        {description ? <p className="mt-0.5 text-xs text-fg-tertiary">{description}</p> : null}
      </div>
      <Switch checked={checked} onChange={onCheckedChange} aria-label={label} />
    </div>
  );
}
type SearchOption = {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
  disabledReason?: string;
};
function SearchPicker({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: string;
  options: SearchOption[];
  onChange: (value: string) => void;
  ariaLabel: string;
}) {
  const { query, open, selected, filtered, dispatch, choose } = useSearchPicker(options, value, onChange);
  return (
    <div className="relative">
      <button
        type="button"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => dispatch({ type: 'toggle' })}
        className="editor-input flex min-h-10 items-center justify-between gap-2 text-left"
      >
        <span className="min-w-0">
          <span className="block truncate text-sm text-fg">
            {selected?.label ?? 'Choose a field'}
          </span>
          {selected?.description ? (
            <span className="block truncate text-xs text-fg-tertiary">{selected.description}</span>
          ) : null}
        </span>
        <Search className="h-4 w-4 shrink-0 text-fg-tertiary" />
      </button>
      {open ? (
        <div className="absolute inset-x-0 top-full z-30 mt-1 overflow-hidden rounded-lg border border-border bg-bg-surface shadow-lg">
          <div className="border-b border-border p-2">
            <input
              autoFocus
              value={query}
              onChange={(event) => dispatch({ type: 'search', query: event.target.value })}
              onKeyDown={(event) => {
                if (event.key === 'Escape') dispatch({ type: 'close' });
              }}
              placeholder="Search…"
              aria-label={`Search ${ariaLabel}`}
              className="editor-input"
            />
          </div>
          <div role="listbox" aria-label={ariaLabel} className="max-h-64 overflow-y-auto p-1">
            {filtered.map((option) => (
              <button
                key={option.value}
                type="button"
                role="option"
                disabled={option.disabled}
                title={option.disabledReason}
                aria-selected={option.value === value}
                onClick={() => choose(option)}
                className={`block w-full rounded-md px-3 py-2 text-left disabled:cursor-not-allowed disabled:opacity-45 ${option.value === value ? 'bg-accent/10 text-accent' : 'text-fg hover:bg-bg-elevated'}`}
              >
                <span className="block text-sm font-medium">{option.label}</span>
                {option.description ? (
                  <span className="block text-xs text-fg-tertiary">{option.description}</span>
                ) : null}
                {option.disabledReason ? (
                  <span className="block text-xs text-warning">{option.disabledReason}</span>
                ) : null}
              </button>
            ))}
            {filtered.length === 0 ? (
              <p className="px-3 py-5 text-center text-sm text-fg-tertiary">No matching fields</p>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function choiceValueForSeries(series: ChartSeriesDefinition, definition: ChartDefinitionV1) {
  const binding = definition.sources.find((source) => source.id === series.y.sourceBindingId);
  return binding?.sourceId === 'metrics.series'
    ? `metric:${String(binding.params.metric ?? series.y.field)}`
    : `field:${binding?.sourceId ?? series.y.sourceBindingId}:${series.y.field}`;
}
function rendererCompatibleCatalog<T extends { sourceId: string; field?: string }>(
  slug: string,
  options: T[]
): T[] {
  return options.filter(
    (option) => supportsBundledRendererCurve(slug, option.sourceId, option.field) !== false
  );
}
function patchAxis(
  onChange: (mutator: (definition: ChartDefinitionV1) => ChartDefinitionV1) => void,
  axis: 'x' | 'y' | 'y2',
  patch: Partial<ChartAxisDefinition>
) {
  onChange((current) => editChartDefinition(current, { type: 'axis', key: axis, patch }));
}
function AxisRangeEditor({
  axis,
  definition,
  onChange,
}: {
  axis: 'x' | 'y' | 'y2';
  definition: ChartDefinitionV1;
  onChange: (mutator: (definition: ChartDefinitionV1) => ChartDefinitionV1) => void;
}) {
  const config = definition.axes[axis];
  if (!config) return null;
  const fixed = config.domain.mode === 'fixed';
  const fixedDomain = config.domain.mode === 'fixed' ? config.domain : null;
  return (
    <div className="grid gap-3 rounded-lg border border-border p-3">
      <div className="flex items-center justify-between gap-3">
        <span className="text-sm font-medium uppercase text-fg">{axis} range</span>
        <Switch
          checked={fixed}
          onChange={(checked) =>
            patchAxis(onChange, axis, {
              domain: checked ? { mode: 'fixed', min: 0, max: 100 } : { mode: 'auto' },
            })
          }
          aria-label={`Use fixed ${axis} range`}
        />
      </div>
      {fixedDomain ? (
        <div className="grid grid-cols-2 gap-3">
          <EditorField label="Minimum">
            <input
              type="number"
              value={fixedDomain.min}
              onChange={(event) =>
                patchAxis(onChange, axis, {
                  domain: { mode: 'fixed', min: Number(event.target.value), max: fixedDomain.max },
                })
              }
              className="editor-input"
            />
          </EditorField>
          <EditorField label="Maximum">
            <input
              type="number"
              value={fixedDomain.max}
              onChange={(event) =>
                patchAxis(onChange, axis, {
                  domain: { mode: 'fixed', min: fixedDomain.min, max: Number(event.target.value) },
                })
              }
              className="editor-input"
            />
          </EditorField>
        </div>
      ) : null}
    </div>
  );
}
function patchSeries(
  onChange: (mutator: (definition: ChartDefinitionV1) => ChartDefinitionV1) => void,
  index: number,
  patch: Partial<ChartSeriesDefinition>
) {
  onChange((current) => editChartDefinition(current, { type: 'series', index, patch }));
}

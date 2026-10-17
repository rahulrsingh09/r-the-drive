import { buildCurveCatalog } from '@panda/dashboards';
import { CHART_SERIES_TOKENS } from '@panda/ui/charts';
import type { ChartDefinitionV1, ChartSeriesDefinition, ChartSourceBinding, ChartSourceManifest, MetricCatalogEntry } from '@panda/types';

type CatalogCurve = ReturnType<typeof buildCurveCatalog>[number];
const unavailable = 'That curve is not available for this chart.';
const sourceLimit = 'This chart can combine up to four data groups. Remove a curve that uses another group before adding this one.';

function bindingIdentity(sourceId: string, field: unknown) {
  return sourceId === 'metrics.series' ? `${sourceId}:${String(field)}` : sourceId;
}

function resolveBinding(definition: ChartDefinitionV1, selected: CatalogCurve, manifests: ChartSourceManifest[], metrics: MetricCatalogEntry[]): ChartSourceBinding | string | null {
  const bindings = new Map<string, ChartSourceBinding>();
  for (const binding of definition.sources) {
    const identity = bindingIdentity(binding.sourceId, binding.params.metric);
    if (!bindings.has(identity)) bindings.set(identity, binding);
  }
  const existing = bindings.get(bindingIdentity(selected.sourceId, selected.field));
  if (existing) return existing;
  if (definition.sources.length >= 4) return sourceLimit;
  const ordinal = definition.sources.length + 1;
  if (selected.sourceId === 'metrics.series') {
    const metric = metrics.find((candidate) => candidate.id === selected.field && candidate.supports_series);
    return metric ? { id: `metric-${metric.id}-${ordinal}`, sourceId: 'metrics.series', params: { metric: metric.id }, filters: [], inherit: { vehicle: true, timeframe: true } } : null;
  }
  const manifest = manifests.find((candidate) => candidate.id === selected.sourceId);
  return { id: `source-${selected.sourceId}-${ordinal}`, sourceId: selected.sourceId, params: {}, filters: [], inherit: { vehicle: true, timeframe: true, ...(manifest?.supportsTripTagInheritance ? { tripTags: true } : {}) } };
}

function commitCurve(definition: ChartDefinitionV1, index: number, selected: CatalogCurve, binding: ChartSourceBinding) {
  const series = definition.series.slice();
  const previous = series[index];
  if (previous) series[index] = { ...previous, label: selected.label, x: { sourceBindingId: binding.id, field: selected.domainKey }, y: { sourceBindingId: binding.id, field: selected.field! } };
  let x = definition.x;
  if (series.length === 1) {
    const domain = { ...definition.x };
    delete domain.unit;
    x = { ...domain, field: { sourceBindingId: binding.id, field: selected.domainKey }, kind: selected.domainKind, ...(selected.domainUnit ? { unit: selected.domainUnit } : {}) };
  }
  const references = new Set([x.field.sourceBindingId]);
  for (const curve of series) {
    references.add(curve.y.sourceBindingId);
    if (curve.x) references.add(curve.x.sourceBindingId);
  }
  const sources = definition.sources.includes(binding) ? definition.sources : [...definition.sources, binding];
  return { ...definition, x, series, sources: sources.filter((source) => references.has(source.id)) };
}

export function applySeriesChoice(definition: ChartDefinitionV1, index: number, choice: string, manifests: ChartSourceManifest[], metrics: MetricCatalogEntry[], onNotice: (notice: string | null) => void): ChartDefinitionV1 {
  const context = definition.series.length === 1 ? { ...definition, series: [] } : definition;
  const selected = buildCurveCatalog(manifests, metrics, context).find((curve) => curve.id === choice);
  if (!selected?.enabled || !selected.field) {
    onNotice(selected?.disabledReason ?? unavailable);
    return definition;
  }
  const binding = resolveBinding(definition, selected, manifests, metrics);
  if (!binding) return definition;
  if (typeof binding === 'string') {
    onNotice(binding);
    return definition;
  }
  const result = commitCurve(definition, index, selected, binding);
  onNotice(null);
  return result;
}

export function addSeriesFromChoice(definition: ChartDefinitionV1, choice: string | undefined, manifests: ChartSourceManifest[], metrics: MetricCatalogEntry[], onNotice: (notice: string | null) => void) {
  if (!choice) return definition;
  const selected = buildCurveCatalog(manifests, metrics, definition).find((curve) => curve.id === choice);
  if (!selected?.enabled) {
    onNotice(selected?.disabledReason ?? unavailable);
    return definition;
  }
  const field = definition.series[0]?.y.field ?? 'value';
  const usedColors = new Set(definition.series.map((curve) => curve.color.mode === 'token' ? curve.color.token : null));
  const color = CHART_SERIES_TOKENS.find((token) => !usedColors.has(token)) ?? CHART_SERIES_TOKENS[definition.series.length % CHART_SERIES_TOKENS.length]!;
  const seed: ChartSeriesDefinition = { id: `series-${definition.series.length + 1}`, label: field, y: { sourceBindingId: definition.sources[0]?.id ?? 'main', field }, mark: 'line', fill: false, yAxis: 'y', color: { mode: 'token', token: color }, transforms: [], visibleInLegend: true };
  return applySeriesChoice({ ...definition, series: [...definition.series, seed] }, definition.series.length, choice, manifests, metrics, onNotice);
}

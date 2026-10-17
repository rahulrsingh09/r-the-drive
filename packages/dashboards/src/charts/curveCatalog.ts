import type { ChartDefinitionV1, ChartSourceManifest, MetricCatalogEntry } from '@panda/types';
import { BUNDLED_CHART_DEFINITIONS } from './defaults';

export interface CurveCatalogOption {
  id: string;
  label: string;
  category: string;
  description: string;
  sourceId: string;
  field?: string;
  domainKey: string;
  domainLabel: string;
  domainKind: 'time' | 'number' | 'category';
  domainUnit?: string;
  domainSignature: string;
  enabled: boolean;
  disabledReason?: string;
}

type FieldRef = ChartDefinitionV1['x']['field'];
function domainFor(reference: FieldRef, definition: ChartDefinitionV1, manifests: readonly ChartSourceManifest[]) {
  const sourceId = definition.sources.find(source => source.id === reference.sourceBindingId)?.sourceId;
  const source = manifests.find(manifest => manifest.id === sourceId);
  const field = source?.fields.find(field => field.id === reference.field);
  return { kind: field?.kind ?? definition.x.kind, field: field?.id ?? reference.field, label: field?.label ?? reference.field, unit: field?.unit };
}

function bundledDomainForCurve(sourceId: string, field: string) {
  for (const definition of BUNDLED_CHART_DEFINITIONS) {
    const bindings = new Map<string, string>();
    for (const source of definition.sources) if (!bindings.has(source.id)) bindings.set(source.id, source.sourceId);
    const curve = definition.series.find(series => bindings.get(series.y.sourceBindingId) === sourceId && series.y.field === field);
    if (!curve) continue;
    const reference = curve.x ?? definition.x.field;
    if (bindings.get(reference.sourceBindingId) === sourceId) return reference.field;
  }
  return null;
}

export function buildCurveCatalog(manifests: readonly ChartSourceManifest[], metrics: readonly MetricCatalogEntry[], definition: ChartDefinitionV1): CurveCatalogOption[] {
  const primary = sharedDomain(definition, manifests);
  const signature = domainSignature(primary.kind, primary.field, primary.unit);
  const usedSources = new Set<string>();
  const usedMetrics = new Set<unknown>();
  for (const source of definition.sources) {
    usedSources.add(source.sourceId);
    if (source.sourceId === 'metrics.series') usedMetrics.add(source.params.metric);
  }
  const options: CurveCatalogOption[] = [];
  const offer = (option: Omit<CurveCatalogOption, 'enabled'>, reusable: boolean, incompatible: string) => {
    const reason = definition.series.length >= 12 ? 'A chart can contain up to twelve curves.'
      : !reusable && definition.sources.length >= 4 ? 'This would exceed the four data groups limit.'
      : definition.series.length > 0 && signature !== option.domainSignature ? incompatible : undefined;
    options.push({ ...option, enabled: reason === undefined, ...(reason ? { disabledReason: reason } : {}) });
  };
  for (const metric of metrics) {
    if (!metric.supports_series) continue;
    offer({
      id: `metric:${metric.id}`, label: metric.label, category: metric.source,
      description: `${metric.source} · ${metric.unit ?? 'unitless'}`,
      sourceId: 'metrics.series', field: metric.id,
      domainKey: 'timestamp', domainLabel: 'Time', domainKind: 'time', domainSignature: 'time',
    }, usedMetrics.has(metric.id), 'This curve uses time values while the chart is plotted over another domain.');
  }
  for (const manifest of manifests) {
    if (manifest.id === 'metrics.series') continue;
    const xFields = manifest.fields.filter(field => field.roles.includes('x'));
    for (const field of manifest.fields) {
      if (!field.roles.includes('y')) continue;
      const preferred = definition.series.length === 0 ? bundledDomainForCurve(manifest.id, field.id) ?? primary.field : primary.field;
      const x = xFields.find(candidate => candidate.id === preferred) ?? xFields[0];
      const kind = x?.kind ?? 'time';
      const key = x?.id ?? 'timestamp';
      offer({
        id: `field:${manifest.id}:${field.id}`, label: field.label, category: manifest.category,
        description: `${manifest.label} · ${field.unit ?? 'unitless'}`,
        sourceId: manifest.id, field: field.id, domainKey: key,
        domainLabel: x?.label ?? 'Time', domainKind: kind,
        domainSignature: domainSignature(kind, key, x?.unit),
        ...(x?.unit ? { domainUnit: x.unit } : {}),
      }, usedSources.has(manifest.id), `This curve uses ${x?.label ?? 'a different'} values; the chart is plotted over ${primary.label}.`);
    }
  }
  return options.sort((a, b) => a.category.localeCompare(b.category) || a.label.localeCompare(b.label));
}

export function domainSignature(kind: 'time' | 'number' | 'category', field: string, unit?: string) {
  return kind === 'time' ? 'time' : `${kind}:${field}:${unit ?? ''}`;
}

export function sharedDomain(definition: ChartDefinitionV1, manifests: readonly ChartSourceManifest[]) {
  return domainFor(definition.x.field, definition, manifests);
}

export function isMixedDomain(definition: ChartDefinitionV1, manifests: readonly ChartSourceManifest[] = []) {
  const primary = sharedDomain(definition, manifests);
  const signature = domainSignature(primary.kind, primary.field, primary.unit);
  for (const series of definition.series) {
    if (!series.x) continue;
    const domain = domainFor(series.x, definition, manifests);
    if (domainSignature(domain.kind, domain.field, domain.unit) !== signature) return true;
  }
  return false;
}

export function removeCurveAndUnusedSources(definition: ChartDefinitionV1, index: number): ChartDefinitionV1 {
  const series: ChartDefinitionV1['series'] = [];
  const referenced = new Set<string>();
  let explicit = true;
  for (const [position, curve] of definition.series.entries()) {
    if (position === index) continue;
    series.push(curve);
    referenced.add(curve.y.sourceBindingId);
    if (curve.x) referenced.add(curve.x.sourceBindingId);
    else explicit = false;
  }
  const rebase = !definition.series[index]?.x && explicit && series.length > 0 ? series[0]!.x : undefined;
  const x = rebase ? { ...definition.x, field: rebase } : definition.x;
  if (series.length) referenced.add(x.field.sourceBindingId);
  return { ...definition, x, series, sources: definition.sources.filter(source => referenced.has(source.id)) };
}

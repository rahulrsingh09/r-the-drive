import sourceManifestData from './sources.json';
import type { ChartSourceManifest, MetricCatalogEntry } from '@panda/types';

export const CHART_SOURCE_MANIFESTS: ReadonlyArray<ChartSourceManifest> = sourceManifestData as ReadonlyArray<ChartSourceManifest>;

export function getChartSourceManifest(sourceId: string) {
  return CHART_SOURCE_MANIFESTS.find((manifest) => manifest.id === sourceId);
}

/**
 * Expands the generic metric-series source from the account-visible metric
 * catalog. The static manifest remains the offline/compatibility fallback;
 * the API metric registry is the owner of the complete searchable inventory.
 */
export function resolveChartSourceCapabilities(
  manifests: readonly ChartSourceManifest[],
  metrics: readonly MetricCatalogEntry[],
): ChartSourceManifest[] {
  const fields: ChartSourceManifest['fields'] = [];
  const ids: string[] = [];
  for (const metric of metrics) {
    if (!metric.supports_series) continue;
    ids.push(metric.id);
    const field: ChartSourceManifest['fields'][number] = {
      id: metric.id, label: metric.label, kind: 'number', roles: ['y', 'detail'],
    };
    if (metric.unit) field.unit = metric.unit;
    fields.push(field);
  }
  return manifests.map(manifest => {
    const copy = { ...manifest };
    if (manifest.id !== 'metrics.series' || !ids.length) return copy;
    const timestamp = manifest.fields.find(field => field.id === 'timestamp');
    copy.fields = [timestamp ?? { id: 'timestamp', label: 'Timestamp', kind: 'time', roles: ['x', 'detail'] }, ...fields.map(field => ({ ...field, roles: [...field.roles] }))];
    copy.parameters = manifest.parameters.map(parameter => {
      if (parameter.id !== 'metric') return parameter;
      return { ...parameter, options: [...ids] };
    });
    return copy;
  });
}

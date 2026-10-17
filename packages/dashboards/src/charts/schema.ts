import { z } from 'zod';
import type {
  ChartFieldRef,
  ChartDefinitionV1,
  ChartSourceManifest,
  ChartValidationError,
} from '@panda/types';

const finiteNumber = z.number().refine(Number.isFinite, 'Must be a finite number');
const identifier = z
  .string()
  .min(1)
  .max(80)
  .regex(/^[a-z0-9][a-z0-9._-]*$/, 'Use lowercase letters, numbers, dots, hyphens, or underscores');
const humanText = z.string().trim().min(1).max(160);
const cssColor = z
  .string()
  .trim()
  .regex(
    /^(?:#[0-9a-f]{3,4}|#[0-9a-f]{6}|#[0-9a-f]{8}|rgba?\([^)]{1,80}\)|hsla?\([^)]{1,80}\))$/i,
    'Use a hex, rgb, rgba, hsl, or hsla color',
  );

export const ChartPlacementSchema = z.object({
  dashboardSlug: identifier,
});

export const ChartTimeframePolicySchema = z.discriminatedUnion('mode', [
  z.object({ mode: z.literal('dashboard') }),
  z.object({
    mode: z.literal('relative'),
    preset: z.enum(['1h', '6h', '24h', '7d', '30d', '90d', '1y']),
  }),
  z.object({ mode: z.literal('lifetime') }),
]);

const sourceFilterValue = z.union([
  z.string().max(160),
  finiteNumber,
  z.boolean(),
  z.null(),
  z.array(z.union([z.string().max(160), finiteNumber])).max(100),
]);

export const ChartSourceFilterSchema = z.object({
  field: identifier,
  operator: z.enum(['eq', 'neq', 'gt', 'gte', 'lt', 'lte', 'in', 'not_in', 'is_null', 'is_not_null']),
  value: sourceFilterValue.optional(),
});

export const ChartSourceBindingSchema = z.object({
  id: identifier,
  sourceId: identifier,
  params: z.record(z.string().max(80), z.unknown()).default({}),
  filters: z.array(ChartSourceFilterSchema).max(20).default([]),
  inherit: z.object({
    vehicle: z.boolean(),
    timeframe: z.boolean(),
    tripTags: z.boolean().optional(),
  }),
});

export const ChartFieldRefSchema = z.object({
  sourceBindingId: identifier,
  field: identifier,
});

export const ChartFieldEncodingSchema = z.object({
  field: ChartFieldRefSchema,
  kind: z.enum(['time', 'number', 'category']),
  label: z.string().trim().max(120).optional(),
  unit: z.string().trim().max(40).optional(),
});

export const ChartColorDefinitionSchema = z.discriminatedUnion('mode', [
  z.object({
    mode: z.literal('token'),
    token: z.enum([
      'accent',
      'emerald',
      'amber',
      'sky',
      'violet',
      'rose',
      'teal',
      'indigo',
      'success',
      'warning',
      'danger',
      'muted',
      'series-01',
      'series-02',
      'series-03',
      'series-04',
      'series-05',
      'series-06',
      'series-07',
      'series-08',
      'series-09',
      'series-10',
      'series-11',
      'series-12',
      'series-13',
      'series-14',
      'series-15',
      'series-16',
    ]),
  }),
  z.object({
    mode: z.literal('custom'),
    light: cssColor,
    dark: cssColor,
  }),
]);

export const ChartTransformDefinitionSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('none') }),
  z.object({ kind: z.literal('scale'), factor: finiteNumber }),
  z.object({ kind: z.literal('offset'), amount: finiteNumber }),
  z.object({ kind: z.literal('delta') }),
  z.object({ kind: z.literal('cumulative') }),
  z.object({ kind: z.literal('rolling_average'), window: z.number().int().min(2).max(1000) }),
  z.object({ kind: z.literal('rolling_median'), window: z.number().int().min(2).max(1000) }),
  z.object({ kind: z.literal('fixed_time_bin'), milliseconds: z.number().int().min(1).max(31_536_000_000) }),
  z.object({ kind: z.literal('histogram_bin'), size: finiteNumber.refine((value) => value > 0, 'Must be greater than zero') }),
  z.object({
    kind: z.literal('expression'),
    formula: z.string().trim().min(1).max(512),
  }),
]);

export const ChartSeriesDefinitionSchema = z.object({
  id: identifier,
  label: humanText,
  y: ChartFieldRefSchema,
  x: ChartFieldRefSchema.optional(),
  mark: z.enum(['line', 'area', 'step', 'bar', 'scatter', 'histogram']),
  fill: z.boolean().optional(),
  yAxis: z.enum(['y', 'y2']),
  color: ChartColorDefinitionSchema,
  strokeWidth: finiteNumber.pipe(z.number().min(0.5).max(12)).optional(),
  pointSize: finiteNumber.pipe(z.number().min(1).max(24)).optional(),
  stackId: identifier.optional(),
  connectGaps: z.boolean().optional(),
  visibleInLegend: z.boolean().optional(),
  transforms: z.array(ChartTransformDefinitionSchema).max(12).default([]),
  valueFormat: z.enum(['number', 'integer', 'percent', 'currency', 'duration', 'unit']).optional(),
});

export const ChartAxisDefinitionSchema = z.object({
  label: z.string().trim().max(120).optional(),
  unit: z.string().trim().max(40).optional(),
  scale: z.enum(['linear', 'log']),
  domain: z.discriminatedUnion('mode', [
    z.object({
      mode: z.literal('auto'),
      includeZero: z.boolean().optional(),
      padding: finiteNumber.pipe(z.number().min(0).max(1)).optional(),
    }),
    z.object({
      mode: z.literal('fixed'),
      min: finiteNumber,
      max: finiteNumber,
    }),
  ]),
  tickFormat: z.enum(['number', 'integer', 'percent', 'currency', 'duration', 'unit']).optional(),
});

export const ChartAxesDefinitionSchema = z.object({
  x: ChartAxisDefinitionSchema,
  y: ChartAxisDefinitionSchema,
  y2: ChartAxisDefinitionSchema.optional(),
});

export const ChartDisplayDefaultsSchema = z.object({
  legend: z.enum(['auto', 'show', 'hide']),
  grid: z.boolean(),
  tooltip: z.boolean(),
  timeFilter: z.enum(['raw', '15m', '1h', '6h', '24h', '3d', '7d']),
  curveSmoothness: z.enum(['straight', 'gentle', 'smooth']),
  dataSmoothing: z.object({
    kind: z.enum(['rolling_average', 'rolling_median']),
    window: z.number().int().min(2).max(1000),
  }).optional(),
  showPoints: z.boolean().optional(),
  emptyTitle: z.string().trim().max(160).optional(),
  emptyDescription: z.string().trim().max(500).optional(),
});

export const ChartInteractionDefaultsSchema = z.object({
  panZoom: z.boolean(),
  touchExplore: z.boolean(),
  connectGaps: z.boolean(),
});

export const ChartAnnotationDefinitionSchema = z.object({
  kind: z.literal('horizontal_reference_line'),
  value: finiteNumber,
  label: z.string().trim().max(120).optional(),
  color: ChartColorDefinitionSchema,
});

const forbiddenKeys = new Set(['url', 'remoteurl', 'sql', 'javascript', 'script', 'templatehtml', 'css']);

function findForbiddenKeys(value: unknown, path: string, errors: ChartValidationError[]) {
  type Visit = { value: unknown; path: string; key?: string; exit?: object };
  const pending: Visit[] = [{ value, path }];
  const ancestors = new WeakSet<object>();
  while (pending.length) {
    const item = pending.pop()!;
    if (item.exit) { ancestors.delete(item.exit); continue; }
    if (item.key !== undefined && forbiddenKeys.has(item.key.replaceAll('_', '').toLowerCase())) {
      errors.push({ path: item.path, message: 'Executable, remote, SQL, HTML, or CSS fields are not allowed' });
    }
    if (!item.value || typeof item.value !== 'object') continue;
    if (ancestors.has(item.value)) throw new RangeError('Maximum call stack size exceeded');
    ancestors.add(item.value);
    pending.push({ value: undefined, path: item.path, exit: item.value });
    const entries = Array.isArray(item.value)
      ? Array.from(item.value.entries()).filter(([, child]) => child !== undefined)
      : Object.entries(item.value);
    for (let index = entries.length - 1; index >= 0; index--) {
      const [key, child] = entries[index]!;
      const next: Visit = { value: child, path: `${item.path}.${key}` };
      if (!Array.isArray(item.value)) next.key = String(key);
      pending.push(next);
    }
  }
}

function definitionReferences(definition: ChartDefinitionV1) {
  const references: Array<{ path: string; reference: ChartFieldRef; role?: 'x' | 'y' }> = [
    { path: 'config.x.field', reference: definition.x.field, role: 'x' },
  ];
  for (const [index, series] of definition.series.entries()) {
    references.push({ path: `config.series.${index}.y`, reference: series.y, role: 'y' });
    if (series.x) references.push({ path: `config.series.${index}.x`, reference: series.x });
  }
  return references;
}

function definitionErrors(definition: ChartDefinitionV1): ChartValidationError[] {
  const errors: ChartValidationError[] = [];
  const bindingIds = new Set<string>();
  const seriesIds = new Set<string>();
  for (const source of definition.sources) bindingIds.add(source.id);
  for (const series of definition.series) seriesIds.add(series.id);
  if (bindingIds.size !== definition.sources.length) errors.push({ path: 'config.sources', message: 'Source binding IDs must be unique' });
  if (seriesIds.size !== definition.series.length) errors.push({ path: 'config.series', message: 'Series IDs must be unique' });
  for (const { path, reference } of definitionReferences(definition)) {
    if (!bindingIds.has(reference.sourceBindingId)) errors.push({ path: `${path}.sourceBindingId`, message: `Unknown source binding '${reference.sourceBindingId}'` });
  }
  if (!definition.axes.y2 && definition.series.some(series => series.yAxis === 'y2')) {
    errors.push({ path: 'config.axes.y2', message: 'A right-axis series requires a y2 axis definition' });
  }
  for (const [index, source] of definition.sources.entries()) {
    for (const [filterIndex, filter] of source.filters.entries()) {
      const requiresValue = !['is_null', 'is_not_null'].includes(filter.operator);
      if (requiresValue === (filter.value !== undefined)) continue;
      errors.push({ path: `config.sources.${index}.filters.${filterIndex}.value`, message: requiresValue ? 'This filter operator requires a value' : 'Null checks do not accept a value' });
    }
  }
  for (const axis of [definition.axes.x, definition.axes.y, definition.axes.y2]) {
    if (axis?.domain.mode !== 'fixed') continue;
    const { min, max } = axis.domain;
    if (min >= max) errors.push({ path: 'config.axes', message: 'Fixed axis minimum must be less than maximum' });
    if (axis.scale === 'log' && (min <= 0 || max <= 0)) errors.push({ path: 'config.axes', message: 'Logarithmic axes require positive fixed ranges' });
  }
  findForbiddenKeys(definition, 'config', errors);
  return errors;
}

export const ChartDefinitionV1Schema = z.object({
  schemaVersion: z.literal(1),
  placements: z.array(ChartPlacementSchema).max(20),
  timeframe: ChartTimeframePolicySchema,
  sources: z.array(ChartSourceBindingSchema).min(1).max(4),
  x: ChartFieldEncodingSchema,
  series: z.array(ChartSeriesDefinitionSchema).min(1).max(32),
  axes: ChartAxesDefinitionSchema,
  display: ChartDisplayDefaultsSchema,
  interaction: ChartInteractionDefaultsSchema,
  annotations: z.array(ChartAnnotationDefinitionSchema).max(20).optional(),
  rendererId: identifier.optional(),
}).superRefine((definition, context) => {
  for (const { path, message } of definitionErrors(definition as ChartDefinitionV1)) {
    context.addIssue({ code: 'custom', path: path.split('.'), message });
  }
});

export const ChartRecordSchema = z.object({
  id: z.guid(),
  ownerId: z.guid().nullable(),
  slug: identifier,
  name: humanText,
  description: z.string().trim().max(500).optional(),
  isDefault: z.boolean(),
  isLocked: z.boolean(),
  isEnabled: z.boolean(),
  baselineRevision: z.number().int().nonnegative().nullable().optional(),
  config: ChartDefinitionV1Schema,
});

export const PortableChartDocumentSchema = z.object({
  kind: z.literal('panda-chart'),
  version: z.literal(1),
  chart: z.object({
    slug: identifier,
    name: humanText,
    description: z.string().trim().max(500).optional(),
    enabled: z.boolean(),
    definition: ChartDefinitionV1Schema,
  }),
});

export function parseChartDefinitionV1(value: unknown): ChartDefinitionV1 {
  return ChartDefinitionV1Schema.parse(value) as ChartDefinitionV1;
}

export function validateChartDefinitionAgainstSources(
  definition: ChartDefinitionV1,
  manifests: ChartSourceManifest[],
): ChartValidationError[] {
  const errors: ChartValidationError[] = [];
  const available = new Map<string, ChartSourceManifest>();
  for (const manifest of manifests) available.set(manifest.id, manifest);
  const bindings = new Map<string, ChartDefinitionV1['sources'][number]>();
  for (const binding of definition.sources) bindings.set(binding.id, binding);
  const resolved = new Map<string, ChartSourceManifest>();
  for (const [id, binding] of bindings) {
    const manifest = available.get(binding.sourceId);
    if (!manifest) {
      errors.push({ path: `config.sources.${id}.sourceId`, message: `Source '${binding.sourceId}' is not available on this API` });
      continue;
    }
    resolved.set(id, manifest);
    const filterRules = new Map(manifest.filterableFields.map(field => [field.field, field.operators]));
    for (const filter of binding.filters) {
      if (filterRules.get(filter.field)?.includes(filter.operator)) continue;
      errors.push({ path: `config.sources.${id}.filters.${filter.field}`, message: `Filter '${filter.field} ${filter.operator}' is not supported by ${manifest.label}` });
    }
  }
  const checkReference = (reference: ChartFieldRef, path: string, role?: 'x' | 'y') => {
    const manifest = resolved.get(reference.sourceBindingId);
    const field = manifest?.fields.find(candidate => candidate.id === reference.field);
    if (!field) errors.push({ path, message: `Field '${reference.field}' is not available from source binding '${reference.sourceBindingId}'` });
    else if (role && !field.roles.includes(role)) errors.push({ path, message: `Field '${reference.field}' cannot be used as ${role === 'x' ? 'an X' : 'a Y'} field` });
    return manifest;
  };
  checkReference(definition.x.field, 'config.x.field', 'x');
  for (const [index, series] of definition.series.entries()) {
    const manifest = checkReference(series.y, `config.series.${index}.y`, 'y');
    if (series.x) checkReference(series.x, `config.series.${index}.x`);
    const mark = series.mark === 'step' ? 'line' : series.mark === 'histogram' ? 'bar' : series.mark;
    if (manifest && !manifest.compatibleMarks.includes(mark)) errors.push({ path: `config.series.${index}.mark`, message: `Mark '${series.mark}' is not supported by source '${manifest.id}'` });
  }
  return errors;
}

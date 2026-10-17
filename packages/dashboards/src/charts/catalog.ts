import type { ChartDefinitionV1 } from '@panda/types';
import type { WidgetDef } from '../registry';
import bundledRendererData from './bundled-renderers.json';
import { BUNDLED_CHART_DEFINITIONS } from './defaults';
import { ChartDefinitionV1Schema } from './schema';

export type DashboardChartPage = 'overview' | 'battery' | 'charging' | 'efficiency' | 'trips';

export type DashboardChartSource =
  | 'soc_history'
  | 'charging_sessions_energy'
  | 'charging_weekly_energy'
  | 'charge_session_curve'
  | 'charging_curve_analysis'
  | 'efficiency_trend'
  | 'efficiency_temperature'
  | 'efficiency_mode'
  | 'efficiency_tags'
  | 'phantom_drain'
  | 'battery_degradation'
  | 'battery_capacity_mileage'
  | 'projected_range_mileage'
  | 'tire_pressure_trips';

export interface DashboardChartDefinition {
  id: string;
  title: string;
  description?: string;
  pages: DashboardChartPage[];
  source: DashboardChartSource;
  config: ChartDefinitionV1;
  mode?: 'line' | 'area' | 'bar' | 'scatter';
  yUnit?: string;
  yRange?: [number, number];
  stepInterpolation?: boolean;
  defaultSize?: WidgetDef['defaultSize'];
  minSize?: WidgetDef['minSize'];
  emptyTitle?: string;
}

export type DashboardChartAxisId = 'x' | 'y' | 'y2';
export type DashboardChartXDomainSource = 'dashboard-timeframe' | 'chart-local';

export interface DashboardChartAxisCapability {
  label: string;
  unit?: string;
}

export interface DashboardChartSettingsCapabilities {
  timeFilter: boolean;
  smoothness?: boolean;
  axes: Partial<Record<DashboardChartAxisId, DashboardChartAxisCapability>>;
  xDomainSource: DashboardChartXDomainSource;
}

export function supportsDashboardChartSmoothness(definition: DashboardChartDefinition): boolean {
  return definition.mode === 'line' || definition.mode === 'area';
}

type DashboardChartRouting = Pick<DashboardChartDefinition, 'source' | 'defaultSize' | 'minSize'>;

/**
 * Renderer routing and widget layout are application metadata, not a second
 * chart configuration. Every visual/data setting comes from ChartDefinitionV1.
 */
const CHART_ROUTING = new Map<string, DashboardChartRouting>(Object.entries({
  'battery-capacity-mileage': {
    source: 'battery_capacity_mileage',
    defaultSize: { w: 6, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'battery-degradation': {
    source: 'battery_degradation',
    defaultSize: { w: 6, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'charge-level': { source: 'soc_history', defaultSize: { w: 12, h: 4 }, minSize: { w: 4, h: 3 } },
  'charging-curve-analysis': {
    source: 'charging_curve_analysis',
    defaultSize: { w: 12, h: 6 },
    minSize: { w: 6, h: 5 },
  },
  'charging-sessions-energy': {
    source: 'charging_sessions_energy',
    defaultSize: { w: 12, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'charging-weekly-energy': {
    source: 'charging_weekly_energy',
    defaultSize: { w: 12, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'efficiency-mode': {
    source: 'efficiency_mode',
    defaultSize: { w: 12, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'efficiency-tags': {
    source: 'efficiency_tags',
    defaultSize: { w: 12, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'efficiency-temperature': {
    source: 'efficiency_temperature',
    defaultSize: { w: 12, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'efficiency-trend': {
    source: 'efficiency_trend',
    defaultSize: { w: 12, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'phantom-drain': {
    source: 'phantom_drain',
    defaultSize: { w: 6, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'projected-range-mileage': {
    source: 'projected_range_mileage',
    defaultSize: { w: 6, h: 4 },
    minSize: { w: 4, h: 3 },
  },
  'soc-history': { source: 'soc_history', defaultSize: { w: 6, h: 4 }, minSize: { w: 4, h: 3 } },
  'tire-pressure-trips': {
    source: 'tire_pressure_trips',
    defaultSize: { w: 12, h: 7 },
    minSize: { w: 6, h: 5 },
  },
}));

/**
 * Fields the established renderer for each bundled chart can consume. This is
 * an editor capability boundary, not visual configuration: it prevents a
 * valid database definition from offering a curve the selected production
 * component would have no semantic slot for.
 */
export type BundledRendererCapability = {
  slug: string;
  source: DashboardChartSource;
  renderer: string;
  editing: 'rich' | 'color';
  x: { sourceId: string; field: string; kind: ChartDefinitionV1['x']['kind'] };
  curves: readonly { sourceId: string; field: string }[];
};

export const BUNDLED_RENDERER_CAPABILITIES = bundledRendererData as BundledRendererCapability[];
const bundledCapabilityBySlug = new Map(
  BUNDLED_RENDERER_CAPABILITIES.map((capability) => [capability.slug, capability])
);

export function getBundledRendererFields(slug: string): readonly string[] | undefined {
  return bundledCapabilityBySlug.get(slug)?.curves.map((curve) => curve.field);
}

export function supportsBundledRendererCurve(
  slug: string,
  sourceId: string,
  field: string | undefined
): boolean | undefined {
  const capability = bundledCapabilityBySlug.get(slug);
  if (!capability) return undefined;
  if (field === undefined) return false;
  const fields = rendererCurveIndex(capability).get(sourceId);
  return fields?.has(field) ?? false;
}

export function supportsBundledRichEditing(slug: string): boolean | undefined {
  const capability = bundledCapabilityBySlug.get(slug);
  return capability ? capability.editing === 'rich' : undefined;
}

function sameConfigValue(left: unknown, right: unknown) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function validateBundledRendererDefinition(
  slug: string,
  definition: ChartDefinitionV1
): Array<{ path: string; message: string }> {
  const capability = bundledCapabilityBySlug.get(slug);
  if (!capability) return [];
  const errors: Array<{ path: string; message: string }> = [];
  const require = (valid: unknown, path: string, message: string) => {
    if (!valid) errors.push({ path, message });
  };
  const canonical = BUNDLED_CHART_DEFINITIONS.find(chart => chart.slug === slug);
  const sources = new Map<string, string>();
  for (const source of definition.sources) if (!sources.has(source.id)) sources.set(source.id, source.sourceId);
  const xSource = sources.get(definition.x.field.sourceBindingId);
  require(xSource === capability.x.sourceId && definition.x.field.field === capability.x.field && definition.x.kind === capability.x.kind,
    'config.x', `The ${slug} production renderer requires ${capability.x.kind} X data from ${capability.x.sourceId}.${capability.x.field}.`);
  if (canonical) {
    require(sameConfigValue(definition.timeframe, canonical.timeframe), 'config.timeframe', 'Bundled charts inherit the active dashboard timeframe so preview and dashboard data stay identical.');
    for (const axis of ['x', 'y', 'y2'] as const) require(definition.axes[axis]?.unit === canonical.axes[axis]?.unit,
      `config.axes.${axis}.unit`, 'Bundled renderer units are owned by the production data adapter.');
    require(definition.display.emptyTitle === canonical.display.emptyTitle && definition.display.emptyDescription === canonical.display.emptyDescription,
      'config.display.emptyTitle', 'Bundled empty-state copy is owned by the production renderer.');
    require(definition.rendererId === canonical.rendererId, 'config.rendererId', 'Bundled renderer selection cannot be changed.');
    if (capability.editing !== 'rich') {
      const comparableAxes = structuredClone(definition.axes);
      const canonicalAxes = structuredClone(canonical.axes);
      if (['daily-charge-sessions', 'daily-energy-bars', 'phantom-drain', 'tire-pressure-timeline'].includes(capability.renderer)) comparableAxes.y.domain = canonicalAxes.y.domain;
      const withoutColors = (series: ChartDefinitionV1['series']) => series.map(item => {
        const copy: Partial<typeof item> = { ...item };
        delete copy.color;
        return copy;
      });
      const contracts: Array<[string, unknown, unknown, string]> = [
        ['sources', definition.sources, canonical.sources, 'This bundled renderer owns its data bindings; duplicate the chart to build a custom source configuration.'],
        ['series', withoutColors(definition.series), withoutColors(canonical.series), 'This bundled renderer supports curve color edits only; its curve order, labels, marks, and axes are fixed.'],
        ['x', definition.x, canonical.x, 'This bundled renderer owns its X-axis encoding.'],
        ['axes', comparableAxes, canonicalAxes, 'This bundled renderer supports only its production axis contract; use dashboard chart settings for a temporary range.'],
        ['display', definition.display, canonical.display, 'This bundled renderer owns its legend, grid, tooltip, and smoothing presentation.'],
        ['interaction', definition.interaction, canonical.interaction, 'This bundled renderer owns its interaction behavior.'],
      ];
      for (const [path, value, expected, message] of contracts) require(sameConfigValue(value, expected), `config.${path}`, message);
    }
  }
  const supported = rendererCurveIndex(capability);
  for (const [index, series] of definition.series.entries()) {
    const sourceId = sources.get(series.y.sourceBindingId);
    const prefix = `config.series.${index}`;
    require(sourceId !== undefined && supported.get(sourceId)?.has(series.y.field), `${prefix}.y`, `The ${slug} production renderer does not support this curve.`);
    require(!series.x, `${prefix}.x`, 'Per-series X fields are supported only by independent custom charts.');
    require(series.transforms.every(transform => transform.kind === 'none'), `${prefix}.transforms`, 'Transforms are not supported by bundled production renderers.');
    require(!series.stackId || series.mark === 'bar' || series.mark === 'histogram', `${prefix}.stackId`, 'Stacking is supported only for bar or histogram curves.');
    require(series.connectGaps === undefined, `${prefix}.connectGaps`, 'Use the chart-level connectGaps setting for bundled charts.');
    require(series.valueFormat === undefined, `${prefix}.valueFormat`, 'Per-curve value formatting is not supported by bundled production renderers.');
  }
  require(!definition.annotations?.length, 'config.annotations', 'Annotations are supported only by independent custom charts.');
  require(!definition.display.dataSmoothing, 'config.display.dataSmoothing', 'Use curve smoothness for bundled production renderers.');
  require(definition.interaction.panZoom, 'config.interaction.panZoom', 'Bundled production charts require pan and zoom.');
  require(definition.interaction.touchExplore, 'config.interaction.touchExplore', 'Bundled production charts require touch exploration.');
  for (const [id, axis] of Object.entries(definition.axes)) {
    require(axis?.scale !== 'log', `config.axes.${id}.scale`, 'Logarithmic axes are not supported by bundled production renderers.');
    require(!axis?.tickFormat, `config.axes.${id}.tickFormat`, 'Custom tick formats are not supported by bundled production renderers.');
  }
  return errors;
}

function rendererCurveIndex(capability: BundledRendererCapability) {
  const supported = new Map<string, Set<string>>();
  for (const { sourceId, field } of capability.curves) {
    const fields = supported.get(sourceId);
    if (fields) fields.add(field);
    else supported.set(sourceId, new Set([field]));
  }
  return supported;
}

const SERIES_DISPLAY_MODES = new Map<string, NonNullable<DashboardChartDefinition['mode']>>([
  ['area', 'area'],
  ['bar', 'bar'],
  ['scatter', 'scatter'],
]);
const { definitions, definitionById } = compileChartCatalog(BUNDLED_CHART_DEFINITIONS);

export function getChartDefinitions(page?: DashboardChartPage): DashboardChartDefinition[] {
  if (!page) return definitions;
  return definitions.filter((definition) => definition.pages.includes(page));
}

export function getChartDefinition(
  id: string | null | undefined
): DashboardChartDefinition | undefined {
  return id ? definitionById.get(id) : undefined;
}

export function getChartOptions(page?: DashboardChartPage) {
  return getChartDefinitions(page).map((definition) => ({
    value: definition.id,
    label: definition.title,
  }));
}

const DEFINITION_UNIT = Symbol('definition-unit');
type SettingsAxisPolicy = readonly [DashboardChartAxisId, string, (string | typeof DEFINITION_UNIT)?];

const SETTINGS_AXES: Partial<Record<DashboardChartSource, readonly SettingsAxisPolicy[]>> = {
  soc_history: [['y', 'State of Charge', DEFINITION_UNIT]],
  charging_weekly_energy: [['y', 'Energy charged', DEFINITION_UNIT]],
  charge_session_curve: [['y', 'Charge rate', 'kW'], ['y2', 'Energy added', 'kWh']],
  charging_curve_analysis: [['x', 'Charge level', '%'], ['y', 'Charge rate', 'kW']],
  efficiency_trend: [['y', 'Efficiency']],
  phantom_drain: [['y', 'Drain rate', DEFINITION_UNIT]],
  battery_degradation: [['y', 'Battery health', DEFINITION_UNIT]],
  battery_capacity_mileage: [['y', 'Usable capacity', DEFINITION_UNIT], ['y2', 'Mileage', 'mi']],
  projected_range_mileage: [['y', 'Projected max range', DEFINITION_UNIT], ['y2', 'Mileage', 'mi']],
  tire_pressure_trips: [['y', 'Tire pressure', DEFINITION_UNIT], ['y2', 'Trips', 'Trips']],
};
const FILTERABLE_SOURCES = new Set<DashboardChartSource>([
  'soc_history', 'charge_session_curve', 'efficiency_trend', 'battery_degradation',
  'battery_capacity_mileage', 'projected_range_mileage',
]);
const LOCAL_DOMAIN_SOURCES = new Set<DashboardChartSource>([
  'charge_session_curve', 'charging_curve_analysis',
]);

export function getChartSettingsCapabilities(
  definition: DashboardChartDefinition
): DashboardChartSettingsCapabilities {
  const axes: DashboardChartSettingsCapabilities['axes'] = {};
  const policy = Object.hasOwn(SETTINGS_AXES, definition.source) ? SETTINGS_AXES[definition.source] : undefined;
  for (const [id, label, unit] of policy ?? []) {
    axes[id] = axisCapability(label, unit === DEFINITION_UNIT ? definition.yUnit : unit);
  }
  return {
    timeFilter: FILTERABLE_SOURCES.has(definition.source),
    axes,
    xDomainSource: LOCAL_DOMAIN_SOURCES.has(definition.source) ? 'chart-local' : 'dashboard-timeframe',
  };
}

function axisCapability(label: string, unit?: string): DashboardChartAxisCapability {
  return unit ? { label, unit } : { label };
}

function compileChartCatalog(charts: typeof BUNDLED_CHART_DEFINITIONS) {
  const definitions: DashboardChartDefinition[] = [];
  for (const chart of charts) {
    const routing = CHART_ROUTING.get(chart.slug);
    if (!routing) throw new Error(`Missing renderer routing for bundled chart: ${chart.slug}`);
    const config = ChartDefinitionV1Schema.parse(chart) as ChartDefinitionV1;
    const primary = config.series[0];
    const axis = config.axes[primary?.yAxis ?? 'y'];
    const pages: DashboardChartPage[] = [];
    for (const { dashboardSlug } of config.placements) {
      if (isDashboardChartPage(dashboardSlug)) pages.push(dashboardSlug);
    }
    const definition: DashboardChartDefinition = {
      id: chart.slug,
      title: chart.title,
      description: chart.description,
      pages,
      source: routing.source,
      config,
      mode: seriesMode(primary),
      stepInterpolation: primary?.mark === 'step',
      defaultSize: routing.defaultSize ?? { w: 12, h: 8 },
      minSize: routing.minSize ?? { w: 4, h: 6 },
    };
    if (axis?.unit) definition.yUnit = axis.unit;
    const range = fixedRange(axis);
    if (range) definition.yRange = range;
    if (config.display.emptyTitle) definition.emptyTitle = config.display.emptyTitle;
    definitions.push(definition);
  }
  definitions.sort((left, right) => left.title.localeCompare(right.title));
  const definitionById = new Map<string, DashboardChartDefinition>();
  for (const definition of definitions) definitionById.set(definition.id, definition);
  return { definitions, definitionById };
}

function isDashboardChartPage(value: string): value is DashboardChartPage {
  return (
    value === 'overview' ||
    value === 'battery' ||
    value === 'charging' ||
    value === 'efficiency' ||
    value === 'trips'
  );
}

function seriesMode(
  series: ChartDefinitionV1['series'][number] | undefined
): NonNullable<DashboardChartDefinition['mode']> {
  const mark = series?.fill ? 'area' : series?.mark;
  return SERIES_DISPLAY_MODES.get(mark ?? 'line') ?? 'line';
}

function fixedRange(
  axis: ChartDefinitionV1['axes']['y'] | undefined
): [number, number] | undefined {
  return axis?.domain.mode === 'fixed' ? [axis.domain.min, axis.domain.max] : undefined;
}

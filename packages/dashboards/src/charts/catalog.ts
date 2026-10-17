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
const CHART_ROUTING: Record<string, DashboardChartRouting> = {
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
};

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
  const supported = bundledCapabilityBySlug.get(slug)?.curves;
  return supported
    ? field !== undefined &&
        supported.some((curve) => curve.sourceId === sourceId && curve.field === field)
    : undefined;
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
  const supported = new Map<string, Set<string>>();
  for (const curve of capability.curves) {
    const fields = supported.get(curve.sourceId) ?? new Set<string>();
    fields.add(curve.field);
    supported.set(curve.sourceId, fields);
  }
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

const definitions = BUNDLED_CHART_DEFINITIONS.map((chart) => normalizeChartDefinition(chart)).sort(
  (a, b) => a.title.localeCompare(b.title)
);

const definitionById = new Map(definitions.map((definition) => [definition.id, definition]));

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

export function getChartSettingsCapabilities(
  definition: DashboardChartDefinition
): DashboardChartSettingsCapabilities {
  switch (definition.source) {
    case 'soc_history':
      return {
        timeFilter: true,
        axes: {
          y: axisCapability('State of Charge', definition.yUnit),
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'charging_weekly_energy':
      return {
        timeFilter: false,
        axes: {
          y: axisCapability('Energy charged', definition.yUnit),
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'charge_session_curve':
      return {
        timeFilter: true,
        axes: {
          y: { label: 'Charge rate', unit: 'kW' },
          y2: { label: 'Energy added', unit: 'kWh' },
        },
        xDomainSource: 'chart-local',
      };
    case 'charging_curve_analysis':
      return {
        timeFilter: false,
        axes: {
          x: { label: 'Charge level', unit: '%' },
          y: { label: 'Charge rate', unit: 'kW' },
        },
        xDomainSource: 'chart-local',
      };
    case 'efficiency_trend':
      return {
        timeFilter: true,
        axes: {
          y: { label: 'Efficiency' },
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'phantom_drain':
      return {
        timeFilter: false,
        axes: {
          y: axisCapability('Drain rate', definition.yUnit),
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'battery_degradation':
      return {
        timeFilter: true,
        axes: {
          y: axisCapability('Battery health', definition.yUnit),
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'battery_capacity_mileage':
      return {
        timeFilter: true,
        axes: {
          y: axisCapability('Usable capacity', definition.yUnit),
          y2: { label: 'Mileage', unit: 'mi' },
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'projected_range_mileage':
      return {
        timeFilter: true,
        axes: {
          y: axisCapability('Projected max range', definition.yUnit),
          y2: { label: 'Mileage', unit: 'mi' },
        },
        xDomainSource: 'dashboard-timeframe',
      };
    case 'tire_pressure_trips':
      return {
        timeFilter: false,
        axes: {
          y: axisCapability('Tire pressure', definition.yUnit),
          y2: { label: 'Trips', unit: 'Trips' },
        },
        xDomainSource: 'dashboard-timeframe',
      };
    default:
      return {
        timeFilter: false,
        axes: {},
        xDomainSource: 'dashboard-timeframe',
      };
  }
}

function axisCapability(label: string, unit?: string): DashboardChartAxisCapability {
  return unit ? { label, unit } : { label };
}

function normalizeChartDefinition(
  chart: (typeof BUNDLED_CHART_DEFINITIONS)[number]
): DashboardChartDefinition {
  const routing = CHART_ROUTING[chart.slug];
  if (!routing) throw new Error(`Missing renderer routing for bundled chart: ${chart.slug}`);
  const primarySeries = chart.series[0];
  const primaryAxis = primarySeries?.yAxis === 'y2' ? chart.axes.y2 : chart.axes.y;
  const yRange = fixedRange(primaryAxis);
  const config = ChartDefinitionV1Schema.parse(chart) as unknown as ChartDefinitionV1;
  return {
    id: chart.slug,
    title: chart.title,
    description: chart.description,
    pages: chart.placements
      .map((placement) => placement.dashboardSlug)
      .filter(isDashboardChartPage),
    source: routing.source,
    config,
    mode: seriesMode(primarySeries),
    ...(primaryAxis?.unit ? { yUnit: primaryAxis.unit } : {}),
    ...(yRange ? { yRange } : {}),
    stepInterpolation: primarySeries?.mark === 'step',
    defaultSize: routing.defaultSize ?? { w: 12, h: 8 },
    minSize: routing.minSize ?? { w: 4, h: 6 },
    ...(chart.display.emptyTitle ? { emptyTitle: chart.display.emptyTitle } : {}),
  };
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
  if (!series) return 'line';
  if (series.mark === 'area' || series.fill) return 'area';
  if (series.mark === 'bar' || series.mark === 'scatter') return series.mark;
  return 'line';
}

function fixedRange(
  axis: ChartDefinitionV1['axes']['y'] | undefined
): [number, number] | undefined {
  return axis?.domain.mode === 'fixed' ? [axis.domain.min, axis.domain.max] : undefined;
}

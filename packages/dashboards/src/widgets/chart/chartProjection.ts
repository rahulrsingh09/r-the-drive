import type { ChartColorDefinition, ChartDataset, ChartDefinitionV1 } from '@panda/types';
import { getChartColor, type RichSeries } from '@panda/ui/charts';

type DomainKind = ChartDefinitionV1['x']['kind'];
export function chartColor(color: ChartColorDefinition, dark: boolean) {
  switch (color.mode) {
    case 'token':
      return getChartColor(color.token);
    default:
      return color[dark ? 'dark' : 'light'];
  }
}
export function chartMark(
  series: ChartDefinitionV1['series'][number]
): NonNullable<RichSeries['mode']> {
  const modes = {
    histogram: 'bar',
    bar: 'bar',
    scatter: 'scatter',
    area: 'area',
    line: series.fill ? 'area' : 'line',
    step: series.fill ? 'area' : 'line',
  } as const;
  return modes[series.mark] ?? 'line';
}
export function chartDomain(
  axis: ChartDefinitionV1['axes'][keyof ChartDefinitionV1['axes']]
): [number, number] | undefined {
  if (axis?.domain.mode !== 'fixed') return undefined;
  const { min, max } = axis.domain;
  return [min, max];
}
function domainIdentity(value: string | number, kind: DomainKind) {
  const numeric = typeof value === 'number';
  const parsed = numeric ? value : kind === 'number' ? Number(value) : Date.parse(value);
  const normalizable = kind !== 'category' && Number.isFinite(parsed);
  if (!normalizable) return `${numeric ? 'n' : 's'}:${value}`;
  return kind === 'number'
    ? `n:${parsed}`
    : `t:${parsed > 10_000_000_000 ? parsed : parsed * 1000}`;
}
function sortValue(value: string | number, kind: DomainKind) {
  const number = typeof value === 'number' ? value : Date.parse(value);
  return kind === 'time' && number > 10_000_000_000 ? number / 1000 : number;
}

export function projectDefinition(
  definition: ChartDefinitionV1,
  datasets: ChartDataset[],
  dark: boolean
) {
  const union = new Map<string, string | number>();
  const sources = new Map<string, ChartDataset>();
  for (const dataset of datasets) {
    if (!sources.has(dataset.sourceBindingId)) sources.set(dataset.sourceBindingId, dataset);
    for (const value of dataset.domain.values)
      union.set(domainIdentity(value, definition.x.kind), value);
  }
  const domain = [...union.values()];
  if (definition.x.kind !== 'category') {
    const sortable = domain.map((value) => ({ value, order: sortValue(value, definition.x.kind) }));
    sortable.sort((a, b) =>
      Number.isFinite(a.order) && Number.isFinite(b.order)
        ? a.order - b.order
        : String(a.value).localeCompare(String(b.value))
    );
    domain.splice(0, domain.length, ...sortable.map((entry) => entry.value));
  }
  const series: RichSeries[] = [];
  definition.series.forEach((spec, position) => {
    const source = sources.get(spec.y.sourceBindingId);
    const field = source?.fields[spec.y.field];
    if (!source || !field) return;
    const xField = spec.x?.sourceBindingId === source.sourceBindingId ? spec.x.field : undefined;
    const xValues = xField ? source.fields[xField]?.values : source.domain.values;
    const values = new Map<string, unknown>();
    (xValues ?? []).forEach((x, row) => {
      if (x != null) values.set(domainIdentity(x, source.domain.kind), field.values[row]);
    });
    series.push({
      key: spec.id,
      label: spec.label,
      color: chartColor(spec.color, dark),
      values: domain.map((x) => {
        const value = values.get(domainIdentity(x, source.domain.kind));
        return typeof value === 'number' && Number.isFinite(value) ? value : null;
      }),
      mode: chartMark(spec),
      pointSize: spec.pointSize,
      strokeWidth: spec.strokeWidth,
      yScale: spec.yAxis,
      stackId: spec.stackId,
      showInLegend: spec.visibleInLegend,
      interpolation: spec.mark === 'step' ? 'step' : undefined,
      dash:
        position < 16
          ? undefined
          : [
              [8, 4],
              [3, 3],
              [10, 3, 2, 3],
            ][Math.floor(position / 16 - 1) % 3],
    } as RichSeries);
  });
  return {
    points: domain.map((ts) => ({ ts })),
    series,
    referenceLines: (definition.annotations ?? []).flatMap((annotation) =>
      annotation.kind !== 'horizontal_reference_line'
        ? []
        : [
            {
              value: annotation.value,
              ...(annotation.label ? { label: annotation.label } : {}),
              color: chartColor(annotation.color, dark),
            },
          ]
    ),
  };
}

export function canonicalSeriesStyle(
  spec: ChartDefinitionV1['series'][number],
  dark: boolean,
  defaultToken: string,
  defaultColor: string
) {
  const style: Partial<RichSeries> & {
    color: string;
    label: string;
    mode: NonNullable<RichSeries['mode']>;
  } = {
    label: spec.label,
    mode: chartMark(spec),
    color:
      spec.color.mode === 'token' && spec.color.token === defaultToken
        ? defaultColor
        : chartColor(spec.color, dark),
  };
  const optional = {
    visibleInLegend: 'showInLegend',
    pointSize: 'pointSize',
    strokeWidth: 'strokeWidth',
    stackId: 'stackId',
  } as const;
  for (const key of Object.keys(optional) as Array<keyof typeof optional>) {
    if (spec[key] !== undefined) Object.assign(style, { [optional[key]]: spec[key] });
  }
  if (spec.mark === 'step') style.interpolation = 'step';
  return style;
}

export function projectTimestampValues(
  primary: Array<{ ts: string; value: number | null }>,
  secondary: Array<{ ts: string; value: number | null }> = []
) {
  const points: Array<{ ts: string }> = [],
    values: Array<number | null> = [],
    related: Array<number | null> = [];
  const positions = new Map<string, number[]>();
  primary.forEach((row, index) => {
    points.push({ ts: row.ts });
    values.push(row.value);
    related.push(null);
    const slots = positions.get(row.ts) ?? [];
    slots.push(index);
    positions.set(row.ts, slots);
  });
  for (const row of secondary)
    for (const index of positions.get(row.ts) ?? []) related[index] = row.value ?? null;
  return { points, values, related };
}

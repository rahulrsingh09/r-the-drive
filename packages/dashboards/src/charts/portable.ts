import { ChartRecordSchema, PortableChartDocumentSchema } from './schema';
import type { ChartDefinitionV1, ChartRecord, PortableChartDocument } from '@panda/types';

export const CHART_EXPORT_KIND = 'panda-chart' as const;
export const CHART_EXPORT_VERSION = 1 as const;
export const MAX_CHART_EXPORT_BYTES = 256 * 1024;

type ChartImportDraft = Omit<ChartRecord, 'id' | 'ownerId' | 'baselineRevision'>;
const ChartImportDraftSchema = ChartRecordSchema.omit({
  id: true,
  ownerId: true,
  baselineRevision: true,
});

function portableChartDocument(record: ChartRecord): PortableChartDocument {
  return {
    kind: CHART_EXPORT_KIND,
    version: CHART_EXPORT_VERSION,
    chart: {
      slug: record.slug,
      name: record.name,
      ...(record.description ? { description: record.description } : {}),
      enabled: record.isEnabled,
      definition: record.config,
    },
  };
}

export function exportChartJson(record: ChartRecord): string {
  return JSON.stringify(portableChartDocument(record), null, 2);
}

function assertPortableSize(text: string) {
  let bytes = 0;
  for (const character of text) {
    const code = character.codePointAt(0)!;
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    if (bytes > MAX_CHART_EXPORT_BYTES) {
      throw new Error('Chart export exceeds the 256 KiB portable file limit');
    }
  }
}

export function parseChartJson(text: string): PortableChartDocument {
  assertPortableSize(text);
  const parsed = PortableChartDocumentSchema.parse(JSON.parse(text));
  const chart: PortableChartDocument['chart'] = {
    slug: parsed.chart.slug,
    name: parsed.chart.name,
    enabled: parsed.chart.enabled,
    definition: parsed.chart.definition as ChartDefinitionV1,
  };
  if (parsed.chart.description !== undefined) chart.description = parsed.chart.description;
  return { kind: parsed.kind, version: parsed.version, chart };
}

export function importChartJson(text: string): Omit<ChartRecord, 'id' | 'ownerId' | 'baselineRevision'> {
  return toChartImportDraft(parseChartJson(text).chart);
}

function toChartImportDraft(chart: PortableChartDocument['chart']): ChartImportDraft {
  const parsed = ChartImportDraftSchema.parse({
    slug: chart.slug,
    name: chart.name,
    ...(chart.description ? { description: chart.description } : {}),
    isDefault: false,
    isLocked: false,
    isEnabled: chart.enabled,
    config: chart.definition,
  });
  const draft: ChartImportDraft = {
    slug: parsed.slug,
    name: parsed.name,
    isDefault: parsed.isDefault,
    isLocked: parsed.isLocked,
    isEnabled: parsed.isEnabled,
    config: parsed.config as ChartDefinitionV1,
  };
  if (parsed.description !== undefined) draft.description = parsed.description;
  return draft;
}

export function downloadChartJson(record: ChartRecord) {
  const blob = new Blob([exportChartJson(record)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  try {
    const anchor = Object.assign(document.createElement('a'), { href: url, download: `${record.slug}.panda-chart.json` });
    anchor.click();
  } finally {
    URL.revokeObjectURL(url);
  }
}

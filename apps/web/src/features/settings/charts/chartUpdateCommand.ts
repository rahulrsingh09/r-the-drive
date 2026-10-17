import type { ChartDefinitionV1, ChartManagerEntry } from '@panda/types';

type ChartPatch = { isEnabled: boolean };
export function chartUpdateCommand(entry: ChartManagerEntry, patch: ChartPatch) {
  const chart = entry.effective;
  if (entry.origin !== 'system') {
    return { kind: 'enabled' as const, body: { id: chart.id, isEnabled: patch.isEnabled } };
  }
  return {
    kind: 'create' as const,
    body: { slug: chart.slug, name: chart.name, ...(chart.description !== undefined ? { description: chart.description } : {}), isEnabled: patch.isEnabled, config: chart.config },
  };
}

export function unassignedChartCopy(chart: { slug: string; name: string; description?: string | undefined; isEnabled: boolean; config: ChartDefinitionV1 }) {
  const { slug, name, description, isEnabled, config } = chart;
  return { slug, name, ...(description !== undefined ? { description } : {}), isEnabled, config: { ...structuredClone(config), placements: [] } };
}

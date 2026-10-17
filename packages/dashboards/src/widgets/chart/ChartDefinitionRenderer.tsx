import React from 'react';
import type { ChartDataset, ChartDefinitionV1 } from '@panda/types';
import { RichTimeSeriesChart } from '@panda/ui/charts';
import { useDocumentTheme } from '@panda/ui/hooks';
import { chartDomain, projectDefinition } from './chartProjection';

export interface ChartDefinitionRendererProps {
  definition: ChartDefinitionV1;
  datasets: ChartDataset[];
  height: number;
  loading?: boolean;
  partial?: boolean;
  refreshing?: boolean;
  error?: boolean;
  presentation?: 'embedded' | 'mobile-viewer';
}

export function ChartDefinitionRenderer({ definition, datasets, height, loading = false, partial = false, refreshing = false, error = false, presentation = 'embedded' }: ChartDefinitionRendererProps) {
  const isDark = useDocumentTheme();
  const primary = datasets[0];
  const { points, series, referenceLines } = projectDefinition(definition, datasets, isDark);
  if (!primary || series.length === 0) {
    return <div role={error ? 'alert' : 'status'} className="flex items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary" style={{ height }}>{loading ? 'Loading chart data…' : error ? 'Unable to load chart data.' : definition.display.emptyTitle ?? 'No chart data available'}</div>;
  }
  return <div className="relative"><RichTimeSeriesChart
    points={points}
    series={series}
    height={height}
    loading={loading}
    mode="line"
    xTime={definition.x.kind === 'time'}
    xUnit={definition.axes.x.unit}
    xAxisLabel={definition.axes.x.label}
    yAxisLabel={definition.axes.y.label}
    yRightAxisLabel={definition.axes.y2?.label}
    yUnit={definition.axes.y.unit}
    yRightUnit={definition.axes.y2?.unit}
    xRange={chartDomain(definition.axes.x)}
    yRange={chartDomain(definition.axes.y)}
    yRightRange={definition.axes.y2 ? chartDomain(definition.axes.y2) : undefined}
    smoothness={definition.display.curveSmoothness}
    showLegend={definition.display.legend !== 'hide'}
    showGrid={definition.display.grid}
    showTooltip={definition.display.tooltip}
    showPoints={definition.display.showPoints ?? false}
    emptyTitle={definition.display.emptyTitle}
    emptyDescription={definition.display.emptyDescription}
    connectGaps={definition.interaction.connectGaps}
    referenceLines={referenceLines}
    interactionMode={presentation === 'mobile-viewer' ? 'touch-explore' : 'standard'}
  />{partial || refreshing ? <div role="status" className="pointer-events-none absolute right-2 top-2 rounded-md border border-border bg-bg-surface/90 px-2 py-1 text-[11px] text-fg-tertiary shadow-sm">{partial ? 'Some data is unavailable' : 'Updating chart…'}</div> : null}</div>;
}

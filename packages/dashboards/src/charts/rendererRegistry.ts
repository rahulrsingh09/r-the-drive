import type { ChartDataset, ChartDefinitionV1 } from '@panda/types';
import * as React from 'react';
import { ChartDefinitionRenderer } from '../widgets/chart/ChartDefinitionRenderer';

export interface ChartRendererProps {
  definition: ChartDefinitionV1;
  datasets: ChartDataset[];
  height: number;
  loading?: boolean;
  partial?: boolean;
  refreshing?: boolean;
  error?: boolean;
  presentation?: 'embedded' | 'mobile-viewer';
}

export interface ChartRendererPlugin {
  id: string;
  supports: (definition: ChartDefinitionV1) => boolean;
  render: (props: ChartRendererProps) => React.ReactNode;
}

const definitionRenderer: ChartRendererPlugin = {
  id: 'rich-time-series',
  supports: () => true,
  render: (props) => React.createElement(ChartDefinitionRenderer, props),
};

export const CHART_RENDERER_PLUGINS: ReadonlyArray<ChartRendererPlugin> = [definitionRenderer];

export function getChartRenderer(_definition: ChartDefinitionV1) {
  return definitionRenderer;
}

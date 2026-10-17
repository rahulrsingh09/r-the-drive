import type { ChartRecord } from '@panda/types';
import {
  getChartSettingsCapabilities,
  supportsDashboardChartSmoothness,
  type DashboardChartAxisId,
  type DashboardChartDefinition,
  type DashboardChartSettingsCapabilities,
} from '../../charts/catalog';

export function widgetChartCapabilities(
  bundled?: DashboardChartDefinition,
  record?: ChartRecord
): DashboardChartSettingsCapabilities {
  if (bundled) {
    return {
      ...getChartSettingsCapabilities(bundled),
      smoothness: supportsDashboardChartSmoothness(bundled),
    };
  }
  const result: DashboardChartSettingsCapabilities = {
    timeFilter: false,
    axes: {},
    xDomainSource: 'dashboard-timeframe',
  };
  if (!record) return result;
  const { config } = record;
  for (const id of ['x', 'y', 'y2'] as DashboardChartAxisId[]) {
    const axis = config.axes[id];
    if (!axis) continue;
    result.axes[id] = { label: axis.label ?? `${id.toUpperCase()} axis` };
    if (axis.unit) result.axes[id]!.unit = axis.unit;
  }
  result.timeFilter = config.x.kind === 'time';
  result.smoothness = config.series.some((series) => ['line', 'area', 'step'].includes(series.mark));
  result.xDomainSource = config.timeframe.mode === 'dashboard' ? 'dashboard-timeframe' : 'chart-local';
  return result;
}

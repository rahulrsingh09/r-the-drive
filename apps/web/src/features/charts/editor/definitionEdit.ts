import type { ChartAxisDefinition, ChartDefinitionV1, ChartSeriesDefinition, ChartSourceBinding } from '@panda/types';

type DefinitionEdit =
  | { type: 'timeframe'; value: string }
  | { type: 'inheritance'; bindingId: string; key: keyof ChartSourceBinding['inherit']; value: boolean }
  | { type: 'right-axis'; enabled: boolean }
  | { type: 'axis'; key: 'x' | 'y' | 'y2'; patch: Partial<ChartAxisDefinition> }
  | { type: 'display'; patch: Partial<ChartDefinitionV1['display']> }
  | { type: 'move-series'; from: number; to: number }
  | { type: 'mark'; index: number; mark: ChartSeriesDefinition['mark'] }
  | { type: 'fill'; index: number; enabled: boolean }
  | { type: 'series'; index: number; patch: Partial<ChartSeriesDefinition> };

export function editChartDefinition(current: ChartDefinitionV1, edit: DefinitionEdit): ChartDefinitionV1 {
  if (edit.type === 'axis' && !current.axes[edit.key]) return current;
  const next = structuredClone(current);
  switch (edit.type) {
    case 'timeframe':
      next.timeframe = edit.value === 'dashboard' || edit.value === 'lifetime' ? { mode: edit.value } : { mode: 'relative', preset: edit.value as Extract<ChartDefinitionV1['timeframe'], { mode: 'relative' }>['preset'] };
      break;
    case 'inheritance':
      for (const binding of next.sources) {
        if (binding.id === edit.bindingId) binding.inherit[edit.key] = edit.value;
      }
      break;
    case 'right-axis':
      if (edit.enabled) next.axes.y2 ??= { scale: 'linear', domain: { mode: 'auto' } };
      else {
        next.axes = { x: next.axes.x, y: next.axes.y };
        for (const curve of next.series) curve.yAxis = 'y';
      }
      break;
    case 'axis': Object.assign(next.axes[edit.key]!, edit.patch); break;
    case 'display': Object.assign(next.display, edit.patch); break;
    case 'move-series': {
      const curve = next.series[edit.from];
      if (!curve || edit.to < 0 || edit.to >= next.series.length) return current;
      const step = Math.sign(edit.to - edit.from);
      for (let index = edit.from; index !== edit.to; index += step) next.series[index] = next.series[index + step]!;
      next.series[edit.to] = curve;
      break;
    }
    case 'mark': {
      const curve = next.series[edit.index];
      if (!curve) return current;
      if (edit.mark === 'line' || edit.mark === 'step') curve.fill = curve.mark === 'area' || curve.fill === true;
      curve.mark = edit.mark;
      break;
    }
    case 'fill': {
      const curve = next.series[edit.index];
      if (!curve) return current;
      if (curve.mark === 'area') curve.mark = 'line';
      curve.fill = edit.enabled;
      break;
    }
    case 'series': if (next.series[edit.index]) Object.assign(next.series[edit.index]!, edit.patch); break;
  }
  return next;
}

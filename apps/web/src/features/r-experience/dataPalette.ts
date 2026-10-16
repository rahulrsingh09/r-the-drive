import { CHART_COLOR_TOKENS, CHART_SERIES_TOKENS } from '@panda/ui/charts';
import type { DataPaletteOverride } from '@panda/ui/hooks';

export function readRDataPalette(): DataPaletteOverride {
  const chartColorPairs: NonNullable<DataPaletteOverride['chartColorPairs']> = {};
  for (const mode of ['light', 'dark'] as const) {
    const probe = document.createElement('div');
    probe.className = `r-palette-probe ${mode}`;
    probe.hidden = true;
    document.body.append(probe);
    const style = getComputedStyle(probe);
    for (const token of [...CHART_COLOR_TOKENS, ...CHART_SERIES_TOKENS, 'yellow', 'orange']) {
      const variable = token.startsWith('series-') ? `--rm-${token}` : `--rm-chart-${token}`;
      const color = style.getPropertyValue(variable).trim();
      chartColorPairs[token] ??= { light: '', dark: '' };
      chartColorPairs[token][mode] = color;
    }
    probe.remove();
  }
  // Categorical selection stays fixed while account theme records remain untouched.
  return { palette: 'rad', chartColorPairs };
}

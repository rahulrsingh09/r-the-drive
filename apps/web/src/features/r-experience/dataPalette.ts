import { CHART_COLOR_TOKENS, CHART_SERIES_TOKENS } from '@panda/ui/charts';
import type { DataPaletteOverride } from '@panda/ui/hooks';

export function readRDataPalette(): DataPaletteOverride {
  const probes = ['light', 'dark'].map(mode => Object.assign(document.createElement('div'), {
    className: `r-palette-probe ${mode}`, hidden: true,
  }));
  document.body.append(...probes);
  try {
    const styles = probes.map(probe => getComputedStyle(probe));
    const entries = [...CHART_COLOR_TOKENS, ...CHART_SERIES_TOKENS, 'yellow', 'orange'].map(token => {
      const variable = `--rm-${token.startsWith('series-') ? '' : 'chart-'}${token}`;
      const [light, dark] = styles.map(style => style.getPropertyValue(variable).trim());
      return [token, { light: light!, dark: dark! }];
    });
    return { palette: 'rad', chartColorPairs: Object.fromEntries(entries) };
  } finally {
    for (const probe of probes) probe.remove();
  }
}

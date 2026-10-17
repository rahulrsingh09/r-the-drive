import type { resolveTheme, ThemeColorPair } from '@panda/themes';
import { contrastRatio } from '@panda/ui/lib/color';

export function auditThemeContrast(theme: ReturnType<typeof resolveTheme>) {
  const primary: string[] = [];
  const charts: string[] = [];
  const palette = Object.entries(theme.series) as Array<[string, ThemeColorPair]>;
  const ratios = new Map<string, number>();
  const ratio = (foreground: string, background: string) => {
    const key = `${foreground}:${background}`;
    let value = ratios.get(key);
    if (value === undefined) { value = contrastRatio(foreground, background); ratios.set(key, value); }
    return value;
  };
  for (const mode of ['light', 'dark'] as const) {
    const tokens = theme.tokens[mode];
    for (const surface of ['bg-page', 'bg-surface'] as const) {
      if (ratio(tokens['text-primary'], tokens[surface]) < 4.5) primary.push(`Primary text on ${surface.replace('bg-', '')} in ${mode} mode is below 4.5:1.`);
    }
    palette.forEach(([key, pair], index) => {
      const background = ratio(pair[mode], tokens['bg-page']);
      const predecessor = palette[index - 1]?.[1][mode];
      if (background < 1.5) charts.push(`${key} has weak ${mode}-mode chart-background contrast.`);
      else if (predecessor && ratio(pair[mode], predecessor) < 1.12) charts.push(`${key} is very similar to the preceding ${mode}-mode series.`);
    });
  }
  return { primary, charts };
}

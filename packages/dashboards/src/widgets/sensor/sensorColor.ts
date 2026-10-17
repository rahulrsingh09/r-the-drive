import type { ChartColorDefinition, ChartColorToken } from '@panda/types';
import { getChartColor } from '@panda/ui/charts';

export function sensorColorSelection(
  color: ChartColorToken | ChartColorDefinition | undefined,
  automatic: ChartColorToken,
  dark: boolean
) {
  const descriptor: ChartColorDefinition =
    typeof color === 'string'
      ? { mode: 'token', token: color }
      : (color ?? { mode: 'token', token: automatic });
  const token = descriptor.mode === 'token' ? descriptor.token : automatic;
  return {
    descriptor,
    token,
    color:
      descriptor.mode === 'token'
        ? getChartColor(descriptor.token)
        : descriptor[dark ? 'dark' : 'light'],
  };
}
export function sensorTextColor(
  color: ChartColorToken | ChartColorDefinition | undefined,
  automatic: ChartColorToken,
  graphic: string,
  dark: boolean
) {
  if (dark) return graphic;
  const selected = sensorColorSelection(color, automatic, dark);
  const slot = /^series-(0[1-9]|1[0-6])$/.exec(selected.token);
  if (slot) return `var(--rm-${selected.token}-text)`;
  return selected.descriptor.mode === 'custom'
    ? `color-mix(in srgb, ${graphic} 50%, var(--rm-text-primary) 50%)`
    : graphic;
}

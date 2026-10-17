type ChartPayloadLike = { elapsed_s?: number | null };
export type TripChartMouseState<T> = {
  activePayload?: Array<{ payload?: T }>;
  activeLabel?: string | number | null;
  activeTooltipIndex?: number | null;
};
type HasElapsed = { elapsed_s: number };

function finiteElapsed(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function measuredFloor(samples: readonly HasElapsed[], elapsed: number) {
  let offset = 0;
  let remaining = samples.length;
  let found: number | null = null;
  while (remaining > 0) {
    const half = Math.floor((remaining - 1) / 2);
    const index = offset + half;
    const point = samples[index];
    if (!point) break;
    const value = finiteElapsed(point.elapsed_s);
    if (value == null || value <= elapsed) {
      if (value != null) found = value;
      offset = index + 1;
      remaining -= half + 1;
    } else {
      remaining = half;
    }
  }
  return found ?? finiteElapsed(samples[0]?.elapsed_s);
}

/** Resolve hover coordinates before attempting the ordinal fallback. */
export function getActiveElapsedSFromChartState<T extends object>(
  state: TripChartMouseState<T> | null | undefined,
  fallbackData?: readonly HasElapsed[],
  previousElapsed?: number | null,
) {
  const payload = state?.activePayload?.[0]?.payload as ChartPayloadLike | undefined;
  const label = state?.activeLabel;
  const coordinate = finiteElapsed(payload?.elapsed_s)
    ?? finiteElapsed(typeof label === 'string' ? Number(label) : label);
  const samples = fallbackData ?? [];
  if (coordinate != null) return samples.length ? measuredFloor(samples, coordinate) : coordinate;
  const ordinal = finiteElapsed(state?.activeTooltipIndex);
  const sample = ordinal == null ? undefined : samples[Math.round(ordinal)];
  return finiteElapsed(sample?.elapsed_s) ?? previousElapsed ?? null;
}

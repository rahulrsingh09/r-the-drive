import * as React from 'react';
import type { ChargeCurvePoint } from '@panda/types';

/** Lowest rate that only a DC charger gives (matches the API rule). */
const DC_MIN_RATE_KW = 20;
/** A rise of this share of the peak after a drop is a charger recovery, not noise. */
const RECOVERY_SHARE_OF_PEAK = 0.15;
/** Power at the end at or below this share of the peak counts as a taper. */
const TAPER_END_SHARE_OF_PEAK = 0.85;
/** A taper needs at least this much state-of-charge gain after the peak. */
const TAPER_MIN_SOC_GAIN = 10;

export type ChargeCurvePattern = 'battery_taper' | 'charger_limited' | 'steady';

export interface ChargeCurveInsight {
  peakKw: number;
  peakSoc: number | null;
  endKw: number;
  endSoc: number | null;
  minAfterRampKw: number;
  recoveries: number;
  pattern: ChargeCurvePattern;
}

function socOf(point: ChargeCurvePoint) {
  return Number.isFinite(point.soc_pct) && point.soc_pct > 0 ? point.soc_pct : null;
}

/**
 * Read a DC session's recorded power curve. A battery slows down steadily as
 * it fills, so power that falls and then recovers points to the charger. The
 * result is an estimate from the curve shape; Rivian does not send a reason.
 */
export function describeChargeCurve(points: ChargeCurvePoint[]): ChargeCurveInsight | null {
  const recorded = points
    .filter((point) => point.power_method === 'recorded' && Number.isFinite(point.power_kw) && point.power_kw > 0.5)
    .sort((left, right) => (left.minutes_elapsed ?? 0) - (right.minutes_elapsed ?? 0));
  if (recorded.length < 3) return null;

  let peakIndex = 0;
  recorded.forEach((point, index) => {
    if (point.power_kw > recorded[peakIndex]!.power_kw) peakIndex = index;
  });
  const peak = recorded[peakIndex]!;
  if (peak.power_kw < DC_MIN_RATE_KW) return null;

  // Ignore the ramp-up: start where power first reaches 80% of the peak.
  const rampEnd = recorded.findIndex((point) => point.power_kw >= peak.power_kw * 0.8);
  const settled = recorded.slice(Math.max(0, rampEnd));
  // A recovery is a drop of at least RECOVERY_SHARE_OF_PEAK below the last
  // high, then a rise of the same size. A rising ramp has no drop, so it is
  // not a recovery.
  const swing = peak.power_kw * RECOVERY_SHARE_OF_PEAK;
  let recoveries = 0;
  let high = settled[0]!.power_kw;
  let low = high;
  let minAfterRamp = high;
  for (const point of settled) {
    minAfterRamp = Math.min(minAfterRamp, point.power_kw);
    if (high - low >= swing && point.power_kw - low >= swing) {
      recoveries += 1;
      high = point.power_kw;
      low = point.power_kw;
    } else if (point.power_kw > high) {
      high = point.power_kw;
      low = point.power_kw;
    } else {
      low = Math.min(low, point.power_kw);
    }
  }

  const end = recorded[recorded.length - 1]!;
  const peakSoc = socOf(peak);
  const endSoc = socOf(end);
  const tapered =
    end.power_kw <= peak.power_kw * TAPER_END_SHARE_OF_PEAK &&
    peakSoc != null &&
    endSoc != null &&
    endSoc - peakSoc >= TAPER_MIN_SOC_GAIN;
  const pattern: ChargeCurvePattern = recoveries > 0 ? 'charger_limited' : tapered ? 'battery_taper' : 'steady';

  return {
    peakKw: peak.power_kw,
    peakSoc,
    endKw: end.power_kw,
    endSoc,
    minAfterRampKw: minAfterRamp,
    recoveries,
    pattern,
  };
}

function kw(value: number) {
  return `${Number(value.toFixed(1))} kW`;
}

function atSoc(soc: number | null) {
  return soc == null ? '' : ` at ${Math.round(soc)}%`;
}

export function ChargeCurveInsightNote({ points }: { points: ChargeCurvePoint[] }) {
  const insight = React.useMemo(() => describeChargeCurve(points), [points]);
  if (!insight) return null;

  const reason =
    insight.pattern === 'charger_limited'
      ? `Power went up and down between ${kw(insight.minAfterRampKw)} and ${kw(insight.peakKw)}. A battery slows down steadily as it fills, so the charger probably limited the rate, for example by sharing power with another car.`
      : insight.pattern === 'battery_taper'
        ? `Power fell from ${kw(insight.peakKw)}${atSoc(insight.peakSoc)} to ${kw(insight.endKw)}${atSoc(insight.endSoc)} as the battery filled. This slowdown is normal.`
        : `Power stayed near ${kw(insight.peakKw)}.`;

  return (
    <div className="mt-4 grid gap-2 rounded-xl border border-border bg-bg-elevated/35 p-3 text-sm" data-testid="charge-curve-insight">
      <p className="font-medium text-fg">
        Peak {kw(insight.peakKw)}
        {atSoc(insight.peakSoc)} · Ended at {kw(insight.endKw)}
        {atSoc(insight.endSoc)}
      </p>
      <p className="text-fg-secondary">
        {insight.pattern === 'steady' ? null : <span className="font-medium text-fg">Why the power changed: </span>}
        {reason}
      </p>
      {insight.pattern === 'steady' ? null : (
        <p className="text-xs text-fg-tertiary">Estimated from the shape of the curve. Rivian does not report a reason.</p>
      )}
    </div>
  );
}

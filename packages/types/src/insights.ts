/**
 * Contract for `GET /v1/vehicles/{vehicle_id}/insights`.
 *
 * Each insight is the typed result of one deterministic rule. The front end
 * turns each kind into one fixed sentence template. A rule whose gate is not
 * met returns nothing, so the list can be empty.
 * See docs/architecture/insights.md.
 */

export type InsightScope = 'now' | `trip:${string}` | `charge:${string}`;

export type InsightKind =
  | 'route_comparison'
  | 'days_of_range'
  | 'high_parked_loss'
  | 'charge_shape'
  | 'usual_dc_band'
  | 'missing_price'
  | 'stale_data';

export type InsightSeverity = 'info' | 'attention';

export interface InsightEvidence {
  type: 'trip' | 'charge' | 'parked_period' | 'vehicle';
  /** A trip, charge or vehicle id, or `{start}/{end}` (RFC 3339) for a parked period. */
  id: string;
}

export interface InsightBaseline {
  method: 'median' | 'p10_p90';
  window:
    | 'earlier_drives_same_route'
    | 'last_30_days'
    | 'earlier_parks_90_days'
    | 'earlier_dc_charges';
  count: number;
}

export interface RouteComparisonValues {
  this_mi_per_kwh: number;
  usual_mi_per_kwh: number;
  /** Percent above (+) or below (-) the usual efficiency. */
  pct_diff: number;
  /** 1 is the best of `count + 1` drives, this drive included. */
  rank: number;
  /** Earlier drives on this route. */
  count: number;
}

export interface DaysOfRangeValues {
  range_mi: number;
  median_daily_mi: number;
  days: number;
  days_with_data: number;
}

export interface HighParkedLossValues {
  /** Battery percentage points lost in the latest parked period. */
  loss_pct: number;
  usual_loss_pct: number;
  /** Latest drain rate divided by the usual drain rate; null when the usual rate is 0. */
  ratio: number | null;
  /** Battery percentage points lost per hour in the latest parked period. Additive; older servers omit it. */
  drain_pct_per_hour?: number;
  /** Median drain rate per hour of the earlier parked periods. Additive; older servers omit it. */
  usual_drain_pct_per_hour?: number;
  duration_hours: number;
  started_at: string;
  ended_at: string;
}

export interface ChargePhase {
  name: 'climb' | 'peak' | 'taper';
  start_min: number;
  end_min: number;
  from_kw: number;
  to_kw: number;
}

export interface ChargeShapeValues {
  peak_kw: number;
  peak_soc: number | null;
  peak_minute: number;
  /** Lowest power at or after the peak. */
  low_kw: number;
  low_soc: number | null;
  phases: ChargePhase[];
}

export interface DcSocBand {
  /** Whole battery percent. */
  soc: number;
  p10_kw: number;
  p90_kw: number;
  sessions: number;
}

export interface UsualDcBandValues {
  bands: DcSocBand[];
  /** Earlier DC charges that gave a battery-% curve. */
  sessions: number;
  /** Share of this charge's battery-% bins inside the band. */
  within_band_pct: number | null;
  compared_bins: number;
}

export interface MissingPriceValues {
  count: number;
  window_days: number;
}

export interface StaleDataValues {
  hours: number;
  last_seen_at: string;
}

interface InsightOf<K extends InsightKind, V> {
  /** Stable id: `{kind}:{scope}`. */
  id: string;
  kind: K;
  severity: InsightSeverity;
  values: V;
  baseline: InsightBaseline | null;
  sample_size: number;
  evidence: InsightEvidence[];
}

export type Insight =
  | InsightOf<'route_comparison', RouteComparisonValues>
  | InsightOf<'days_of_range', DaysOfRangeValues>
  | InsightOf<'high_parked_loss', HighParkedLossValues>
  | InsightOf<'charge_shape', ChargeShapeValues>
  | InsightOf<'usual_dc_band', UsualDcBandValues>
  | InsightOf<'missing_price', MissingPriceValues>
  | InsightOf<'stale_data', StaleDataValues>;

export interface InsightsResponse {
  insights: Insight[];
}

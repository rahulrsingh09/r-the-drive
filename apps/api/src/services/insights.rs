//! Deterministic insight rules (release 1).
//!
//! Each rule reads existing data and returns one typed result, or nothing when
//! its gate is not met. No rule guesses: a rule without enough data stays
//! silent. The front end turns each result into one fixed sentence template.
//! See `docs/architecture/insights.md` for the rule catalog and the gates.
//!
//! The rules are pure functions over plain inputs so each one has a unit test
//! with fixed data. The loaders below read the database with runtime sqlx
//! queries and feed those functions.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use futures::future::try_join_all;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{errors::AppError, services::charging::PLAUSIBLE_MAX_CHARGE_KW};

/// Earlier drives on the same route needed before a route comparison shows.
pub const ROUTE_MIN_EARLIER_DRIVES: usize = 5;
/// Most recent earlier drives on a route read for the route baseline.
pub const ROUTE_MAX_EARLIER_DRIVES: i64 = 200;
/// Decimal places kept when a trip end has no saved place (about 110 m).
pub const ROUTE_COORDINATE_DECIMALS: i32 = 3;
/// Days of history read for the daily-miles median.
pub const RANGE_WINDOW_DAYS: i64 = 30;
/// Days with data needed before days of range shows.
pub const RANGE_MIN_DATA_DAYS: usize = 14;
/// Earlier parked periods needed before high parked loss shows.
pub const PARKED_MIN_EARLIER: usize = 7;
/// A parked loss must be at least this many battery percentage points.
pub const PARKED_MIN_LOSS_PCT: f64 = 2.0;
/// A parked drain rate must be at least this multiple of the usual rate.
pub const PARKED_MIN_RATIO: f64 = 2.0;
/// The latest park must have ended in the last this many days.
pub const PARKED_MAX_AGE_DAYS: i64 = 3;
/// Days of parked periods read for the parked-loss baseline.
pub const PARKED_WINDOW_DAYS: i64 = 90;
/// Parked periods shorter than this are ignored (the Parked page default).
pub const PARKED_MIN_DURATION_HOURS: f64 = 6.0;
/// Most recent parked periods read for the parked-loss baseline.
pub const PARKED_MAX_PERIODS: usize = 100;
/// Earlier DC charge curves read at the same time.
pub const DC_BAND_CONCURRENT_LOADS: usize = 4;
/// Measured curve points needed before a charge shape shows.
pub const CHARGE_SHAPE_MIN_POINTS: usize = 8;
/// A point within this share of the peak kW is part of the peak phase.
pub const CHARGE_PEAK_BAND: f64 = 0.03;
/// Earlier DC charges with a battery-% curve needed before the usual band shows.
pub const DC_BAND_MIN_SESSIONS: usize = 3;
/// Most recent earlier DC charges read for the usual band.
pub const DC_BAND_MAX_SESSIONS: i64 = 12;
/// Days of charges checked for a missing price.
pub const MISSING_PRICE_WINDOW_DAYS: i64 = 90;
/// Most charge ids returned as evidence for a missing price.
pub const MISSING_PRICE_MAX_EVIDENCE: i64 = 50;
/// Hours without telemetry before stale data shows.
pub const STALE_DATA_HOURS: f64 = 6.0;

// ---------------------------------------------------------------------------
// Response model
// ---------------------------------------------------------------------------

/// The data scope of an insights request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightScope {
    Now,
    Trip(Uuid),
    Charge(Uuid),
}

impl InsightScope {
    /// Parse `now`, `trip:{id}` or `charge:{id}`. A missing scope is `now`.
    pub fn parse(raw: Option<&str>) -> Result<Self, AppError> {
        let raw = raw.map(str::trim).unwrap_or("now");
        if raw.is_empty() || raw == "now" {
            return Ok(Self::Now);
        }
        let invalid = || AppError::Validation("scope must be now, trip:{id} or charge:{id}".into());
        let (kind, id) = raw.split_once(':').ok_or_else(invalid)?;
        let id = Uuid::parse_str(id.trim()).map_err(|_| invalid())?;
        match kind {
            "trip" => Ok(Self::Trip(id)),
            "charge" => Ok(Self::Charge(id)),
            _ => Err(invalid()),
        }
    }

    fn key(self) -> String {
        match self {
            Self::Now => "now".to_string(),
            Self::Trip(id) => format!("trip:{id}"),
            Self::Charge(id) => format!("charge:{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InsightKind {
    RouteComparison,
    DaysOfRange,
    HighParkedLoss,
    ChargeShape,
    UsualDcBand,
    MissingPrice,
    StaleData,
}

impl InsightKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::RouteComparison => "route_comparison",
            Self::DaysOfRange => "days_of_range",
            Self::HighParkedLoss => "high_parked_loss",
            Self::ChargeShape => "charge_shape",
            Self::UsualDcBand => "usual_dc_band",
            Self::MissingPrice => "missing_price",
            Self::StaleData => "stale_data",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InsightSeverity {
    Info,
    Attention,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InsightEvidence {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
}

impl InsightEvidence {
    fn trip(id: Uuid) -> Self {
        Self {
            kind: "trip",
            id: id.to_string(),
        }
    }
    fn charge(id: Uuid) -> Self {
        Self {
            kind: "charge",
            id: id.to_string(),
        }
    }
    fn vehicle(id: Uuid) -> Self {
        Self {
            kind: "vehicle",
            id: id.to_string(),
        }
    }
}

/// How a rule computed its baseline.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InsightBaseline {
    pub method: &'static str,
    pub window: &'static str,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Insight {
    pub id: String,
    pub kind: InsightKind,
    pub severity: InsightSeverity,
    pub values: serde_json::Value,
    pub baseline: Option<InsightBaseline>,
    pub sample_size: i64,
    pub evidence: Vec<InsightEvidence>,
}

fn insight<V: Serialize>(
    kind: InsightKind,
    scope: InsightScope,
    severity: InsightSeverity,
    values: &V,
    baseline: Option<InsightBaseline>,
    sample_size: usize,
    evidence: Vec<InsightEvidence>,
) -> Insight {
    Insight {
        id: format!("{}:{}", kind.as_str(), scope.key()),
        kind,
        severity,
        // Typed value structs always serialize to a JSON object.
        values: serde_json::to_value(values).unwrap_or(serde_json::Value::Null),
        baseline,
        sample_size: i64::try_from(sample_size).unwrap_or(i64::MAX),
        evidence,
    }
}

// ---------------------------------------------------------------------------
// Statistics helpers
// ---------------------------------------------------------------------------

fn sorted_finite(values: impl IntoIterator<Item = f64>) -> Vec<f64> {
    let mut values: Vec<f64> = values.into_iter().filter(|v| v.is_finite()).collect();
    values.sort_by(f64::total_cmp);
    values
}

/// Linear-interpolation percentile (same method as Postgres PERCENTILE_CONT).
fn percentile_sorted(sorted: &[f64], fraction: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = fraction.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let low = rank.floor() as usize;
    let high = rank.ceil() as usize;
    Some(sorted[low] + (sorted[high] - sorted[low]) * (rank - low as f64))
}

#[cfg(test)]
fn median(values: &[f64]) -> Option<f64> {
    percentile_sorted(&sorted_finite(values.iter().copied()), 0.5)
}

// ---------------------------------------------------------------------------
// Rule: route comparison (scope trip)
// ---------------------------------------------------------------------------

/// One end of a route: a saved place, or rounded coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RouteEndpoint {
    Place(Uuid),
    Coordinates { lat: f64, lng: f64 },
}

impl RouteEndpoint {
    pub fn from_trip(place: Option<Uuid>, lat: Option<f64>, lng: Option<f64>) -> Option<Self> {
        if let Some(place) = place {
            return Some(Self::Place(place));
        }
        match (lat, lng) {
            (Some(lat), Some(lng)) if lat.is_finite() && lng.is_finite() => {
                Some(Self::Coordinates { lat, lng })
            }
            _ => None,
        }
    }

    fn binds(self) -> (Option<Uuid>, Option<f64>, Option<f64>) {
        match self {
            Self::Place(id) => (Some(id), None, None),
            Self::Coordinates { lat, lng } => (None, Some(lat), Some(lng)),
        }
    }
}

/// Convert stored Wh/mi to mi/kWh. Returns nothing for a missing or zero value.
pub fn mi_per_kwh(wh_per_mile: Option<f64>) -> Option<f64> {
    let wh = wh_per_mile?;
    (wh.is_finite() && wh > 0.0).then(|| 1000.0 / wh)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RouteComparisonValues {
    pub this_mi_per_kwh: f64,
    pub usual_mi_per_kwh: f64,
    /// Percent above (+) or below (-) the usual efficiency.
    pub pct_diff: f64,
    /// 1 is the best of `count + 1` drives, this drive included.
    pub rank: usize,
    /// Earlier drives on this route.
    pub count: usize,
}

pub fn route_comparison(this_mi_per_kwh: f64, earlier: &[f64]) -> Option<RouteComparisonValues> {
    if !this_mi_per_kwh.is_finite() || this_mi_per_kwh <= 0.0 {
        return None;
    }
    let earlier = sorted_finite(earlier.iter().copied().filter(|v| *v > 0.0));
    if earlier.len() < ROUTE_MIN_EARLIER_DRIVES {
        return None;
    }
    let usual = percentile_sorted(&earlier, 0.5)?;
    if usual <= 0.0 {
        return None;
    }
    let rank = 1 + earlier.iter().filter(|v| **v > this_mi_per_kwh).count();
    Some(RouteComparisonValues {
        this_mi_per_kwh,
        usual_mi_per_kwh: usual,
        pct_diff: (this_mi_per_kwh - usual) / usual * 100.0,
        rank,
        count: earlier.len(),
    })
}

// ---------------------------------------------------------------------------
// Rule: days of range (scope now)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DaysOfRangeValues {
    pub range_mi: f64,
    pub median_daily_mi: f64,
    pub days: f64,
    pub days_with_data: usize,
}

/// `daily_miles` has one entry per day with data in the window; a day with
/// telemetry but no drive is 0.
pub fn days_of_range(range_mi: Option<f64>, daily_miles: &[f64]) -> Option<DaysOfRangeValues> {
    let range_mi = range_mi.filter(|r| r.is_finite() && *r > 0.0)?;
    let days = sorted_finite(daily_miles.iter().copied().map(|m| m.max(0.0)));
    if days.len() < RANGE_MIN_DATA_DAYS {
        return None;
    }
    let median_daily_mi = percentile_sorted(&days, 0.5)?;
    if median_daily_mi <= 0.0 {
        return None;
    }
    Some(DaysOfRangeValues {
        range_mi,
        median_daily_mi,
        days: range_mi / median_daily_mi,
        days_with_data: days.len(),
    })
}

/// Start (inclusive) and end (exclusive) of the last `days` whole local days
/// before today in `tz`.
pub fn local_day_window(now: DateTime<Utc>, tz: Tz, days: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let today = now.with_timezone(&tz).date_naive();
    let midnight = |date: chrono::NaiveDate| {
        let naive = date.and_time(NaiveTime::MIN);
        tz.from_local_datetime(&naive)
            .earliest()
            .or_else(|| {
                tz.from_local_datetime(&(naive + Duration::hours(1)))
                    .earliest()
            })
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|| Utc.from_utc_datetime(&naive))
    };
    (midnight(today - Duration::days(days)), midnight(today))
}

// ---------------------------------------------------------------------------
// Rule: high parked loss (scope now)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParkedLoss {
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_hours: f64,
    pub loss_pct: f64,
    /// Battery percentage points lost per hour (the Parked page rate).
    pub drain_pct_per_hour: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HighParkedLossValues {
    pub loss_pct: f64,
    /// Median total loss of the earlier parks. For context only; the rule
    /// compares rates.
    pub usual_loss_pct: f64,
    pub drain_pct_per_hour: f64,
    /// Median drain rate of the earlier parks.
    pub usual_drain_pct_per_hour: f64,
    /// Latest drain rate divided by the usual rate; null when the usual rate is 0.
    pub ratio: Option<f64>,
    pub duration_hours: f64,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

/// Flag the latest park when it drained at 2 or more times the usual rate
/// and lost 2 or more battery points. A park that ended more than
/// `PARKED_MAX_AGE_DAYS` before `now` is not current, so it is not flagged.
pub fn high_parked_loss(
    latest: &ParkedLoss,
    earlier: &[ParkedLoss],
    now: DateTime<Utc>,
) -> Option<HighParkedLossValues> {
    if latest.ended_at < now - Duration::days(PARKED_MAX_AGE_DAYS)
        || !latest.loss_pct.is_finite()
        || !latest.drain_pct_per_hour.is_finite()
    {
        return None;
    }
    let earlier: Vec<&ParkedLoss> = earlier
        .iter()
        .filter(|p| p.loss_pct.is_finite() && p.drain_pct_per_hour.is_finite())
        .collect();
    if earlier.len() < PARKED_MIN_EARLIER {
        return None;
    }
    let usual_rate = percentile_sorted(
        &sorted_finite(earlier.iter().map(|p| p.drain_pct_per_hour)),
        0.5,
    )?;
    let usual_loss = percentile_sorted(&sorted_finite(earlier.iter().map(|p| p.loss_pct)), 0.5)?;
    if latest.loss_pct < PARKED_MIN_LOSS_PCT
        || latest.drain_pct_per_hour < PARKED_MIN_RATIO * usual_rate
    {
        return None;
    }
    Some(HighParkedLossValues {
        loss_pct: latest.loss_pct,
        usual_loss_pct: usual_loss,
        drain_pct_per_hour: latest.drain_pct_per_hour,
        usual_drain_pct_per_hour: usual_rate,
        ratio: (usual_rate > 0.0).then(|| latest.drain_pct_per_hour / usual_rate),
        duration_hours: latest.duration_hours,
        started_at: latest.started_at,
        ended_at: latest.ended_at,
    })
}

// ---------------------------------------------------------------------------
// Rules: charge shape and usual DC band (scope charge)
// ---------------------------------------------------------------------------

/// One measured charge-curve point. `minute` counts from the session start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub minute: f64,
    pub kw: f64,
    pub soc: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChargePhaseName {
    Climb,
    Peak,
    Taper,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChargePhase {
    pub name: ChargePhaseName,
    pub start_min: f64,
    pub end_min: f64,
    pub from_kw: f64,
    pub to_kw: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChargeShapeValues {
    pub peak_kw: f64,
    pub peak_soc: Option<f64>,
    pub peak_minute: f64,
    /// Lowest power at or after the peak.
    pub low_kw: f64,
    pub low_soc: Option<f64>,
    pub phases: Vec<ChargePhase>,
}

fn clean_curve(points: &[CurvePoint]) -> Vec<CurvePoint> {
    let mut points: Vec<CurvePoint> = points
        .iter()
        .copied()
        .filter(|p| {
            p.minute.is_finite()
                && p.kw.is_finite()
                && p.kw >= 0.0
                && p.kw <= PLAUSIBLE_MAX_CHARGE_KW
        })
        .map(|p| CurvePoint {
            soc: p.soc.filter(|s| s.is_finite()),
            ..p
        })
        .collect();
    points.sort_by(|a, b| a.minute.total_cmp(&b.minute));
    points
}

fn phase(name: ChargePhaseName, from: &CurvePoint, to: &CurvePoint) -> ChargePhase {
    ChargePhase {
        name,
        start_min: from.minute,
        end_min: to.minute,
        from_kw: from.kw,
        to_kw: to.kw,
    }
}

/// Describe a DC curve as climb, peak and taper. The peak phase is the
/// unbroken run of points around the peak within 3% of the peak kW.
pub fn charge_shape(points: &[CurvePoint]) -> Option<ChargeShapeValues> {
    let points = clean_curve(points);
    if points.len() < CHARGE_SHAPE_MIN_POINTS {
        return None;
    }
    let (peak_idx, peak) = points.iter().enumerate().fold(
        None::<(usize, CurvePoint)>,
        |best, (idx, point)| match best {
            Some((_, b)) if b.kw >= point.kw => best,
            _ => Some((idx, *point)),
        },
    )?;
    if peak.kw <= 0.0 {
        return None;
    }

    let threshold = peak.kw * (1.0 - CHARGE_PEAK_BAND);
    let mut start = peak_idx;
    while start > 0 && points[start - 1].kw >= threshold {
        start -= 1;
    }
    let mut end = peak_idx;
    while end + 1 < points.len() && points[end + 1].kw >= threshold {
        end += 1;
    }

    let last = points.len() - 1;
    let mut phases = Vec::with_capacity(3);
    if start > 0 {
        phases.push(phase(ChargePhaseName::Climb, &points[0], &points[start]));
    }
    phases.push(phase(ChargePhaseName::Peak, &points[start], &points[end]));
    if end < last {
        phases.push(phase(ChargePhaseName::Taper, &points[end], &points[last]));
    }

    let low = points[peak_idx..].iter().fold(
        peak,
        |low, point| if point.kw < low.kw { *point } else { low },
    );

    Some(ChargeShapeValues {
        peak_kw: peak.kw,
        peak_soc: peak.soc,
        peak_minute: peak.minute,
        low_kw: low.kw,
        low_soc: low.soc,
        phases,
    })
}

/// Mean kW per whole battery percent. Points without a battery % are skipped.
pub fn soc_bins(points: &[CurvePoint]) -> BTreeMap<i32, f64> {
    let mut sums: BTreeMap<i32, (f64, usize)> = BTreeMap::new();
    for point in clean_curve(points) {
        let Some(soc) = point.soc.filter(|s| (0.0..=100.0).contains(s)) else {
            continue;
        };
        let entry = sums.entry(soc.floor() as i32).or_insert((0.0, 0));
        entry.0 += point.kw;
        entry.1 += 1;
    }
    sums.into_iter()
        .map(|(soc, (sum, count))| (soc, sum / count as f64))
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SocBand {
    pub soc: i32,
    pub p10_kw: f64,
    pub p90_kw: f64,
    pub sessions: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UsualDcBandValues {
    pub bands: Vec<SocBand>,
    /// Earlier DC charges that gave a battery-% curve.
    pub sessions: usize,
    /// Share of this charge's battery-% bins inside the band; null when the
    /// charge has no battery-% point inside a band.
    pub within_band_pct: Option<f64>,
    pub compared_bins: usize,
    #[serde(skip)]
    pub used_sessions: Vec<Uuid>,
}

/// The usual DC power for each battery percent: p10 to p90 across earlier
/// DC charges. A battery percent shows only when at least 3 charges cover it.
pub fn usual_dc_band(
    this: &[CurvePoint],
    earlier: &[(Uuid, Vec<CurvePoint>)],
) -> Option<UsualDcBandValues> {
    let earlier_bins: Vec<(Uuid, BTreeMap<i32, f64>)> = earlier
        .iter()
        .map(|(id, points)| (*id, soc_bins(points)))
        .filter(|(_, bins)| !bins.is_empty())
        .collect();
    if earlier_bins.len() < DC_BAND_MIN_SESSIONS {
        return None;
    }

    let mut per_bin: BTreeMap<i32, Vec<f64>> = BTreeMap::new();
    for (_, bins) in &earlier_bins {
        for (soc, kw) in bins {
            per_bin.entry(*soc).or_default().push(*kw);
        }
    }
    let bands: Vec<SocBand> = per_bin
        .into_iter()
        .filter(|(_, values)| values.len() >= DC_BAND_MIN_SESSIONS)
        .filter_map(|(soc, values)| {
            let sorted = sorted_finite(values);
            Some(SocBand {
                soc,
                p10_kw: percentile_sorted(&sorted, 0.1)?,
                p90_kw: percentile_sorted(&sorted, 0.9)?,
                sessions: sorted.len(),
            })
        })
        .collect();
    if bands.is_empty() {
        return None;
    }

    let this_bins = soc_bins(this);
    let (compared, inside) = bands
        .iter()
        .fold(
            (0usize, 0usize),
            |(compared, inside), band| match this_bins.get(&band.soc) {
                Some(kw) => (
                    compared + 1,
                    inside + usize::from(*kw >= band.p10_kw && *kw <= band.p90_kw),
                ),
                None => (compared, inside),
            },
        );

    Some(UsualDcBandValues {
        bands,
        sessions: earlier_bins.len(),
        within_band_pct: (compared > 0).then(|| inside as f64 / compared as f64 * 100.0),
        compared_bins: compared,
        used_sessions: earlier_bins.into_iter().map(|(id, _)| id).collect(),
    })
}

// ---------------------------------------------------------------------------
// Rules: missing price and stale data (scope now)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MissingPriceValues {
    pub count: i64,
    pub window_days: i64,
}

pub fn missing_price(count: i64) -> Option<MissingPriceValues> {
    (count > 0).then_some(MissingPriceValues {
        count,
        window_days: MISSING_PRICE_WINDOW_DAYS,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StaleDataValues {
    pub hours: f64,
    pub last_seen_at: DateTime<Utc>,
}

pub fn stale_data(
    last_seen_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<StaleDataValues> {
    let last_seen_at = last_seen_at?;
    let hours = (now - last_seen_at).num_seconds() as f64 / 3600.0;
    (hours > STALE_DATA_HOURS).then_some(StaleDataValues {
        hours,
        last_seen_at,
    })
}

// ---------------------------------------------------------------------------
// Loaders
// ---------------------------------------------------------------------------

/// Evaluate every release-1 rule for `scope`. A trip or charge outside the
/// vehicle is `NotFound`. The caller has already authorized `user_id` for the
/// vehicle; `user_id` selects that member's own trip place corrections.
pub async fn load_insights(
    pool: &PgPool,
    vehicle_id: Uuid,
    user_id: Uuid,
    scope: InsightScope,
    now: DateTime<Utc>,
) -> Result<Vec<Insight>, AppError> {
    let results = match scope {
        InsightScope::Now => {
            let (range, parked, price, stale) = tokio::try_join!(
                load_days_of_range(pool, vehicle_id, now),
                load_high_parked_loss(pool, vehicle_id, now),
                load_missing_price(pool, vehicle_id, now),
                load_stale_data(pool, vehicle_id, now),
            )?;
            vec![stale, parked, price, range]
        }
        InsightScope::Trip(trip_id) => {
            vec![load_route_comparison(pool, vehicle_id, user_id, trip_id).await?]
        }
        InsightScope::Charge(charge_id) => {
            load_charge_insights(pool, vehicle_id, charge_id).await?
        }
    };
    Ok(results.into_iter().flatten().collect())
}

#[derive(Debug, sqlx::FromRow)]
struct TripRouteRow {
    started_at: DateTime<Utc>,
    start_geofence_id: Option<Uuid>,
    end_geofence_id: Option<Uuid>,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
    end_lat: Option<f64>,
    end_lng: Option<f64>,
    efficiency_wh_per_mile: Option<f64>,
}

async fn load_route_comparison(
    pool: &PgPool,
    vehicle_id: Uuid,
    user_id: Uuid,
    trip_id: Uuid,
) -> Result<Option<Insight>, AppError> {
    // Places follow the member's own corrections, as on the trip pages
    // (`trip_user_annotations`, see `routes::trips`).
    let trip = sqlx::query_as::<_, TripRouteRow>(
        "SELECT t.started_at, \
                COALESCE(tua.start_geofence_id, t.start_geofence_id) AS start_geofence_id, \
                COALESCE(tua.end_geofence_id, t.end_geofence_id) AS end_geofence_id, \
                t.start_lat, t.start_lng, t.end_lat, t.end_lng, t.efficiency_wh_per_mile \
         FROM panda.trips t \
         LEFT JOIN panda.trip_user_annotations tua ON tua.trip_id = t.id AND tua.user_id = $3 \
         WHERE t.id = $1 AND t.vehicle_id = $2",
    )
    .bind(trip_id)
    .bind(vehicle_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let Some(this_mi_per_kwh) = mi_per_kwh(trip.efficiency_wh_per_mile) else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (
        RouteEndpoint::from_trip(trip.start_geofence_id, trip.start_lat, trip.start_lng),
        RouteEndpoint::from_trip(trip.end_geofence_id, trip.end_lat, trip.end_lng),
    ) else {
        return Ok(None);
    };
    let (start_place, start_lat, start_lng) = start.binds();
    let (end_place, end_lat, end_lng) = end.binds();

    // A saved place matches by id. Without a place, an end matches by
    // coordinates rounded to ROUTE_COORDINATE_DECIMALS on both sides.
    let earlier = sqlx::query_as::<_, (Uuid, f64)>(
        r#"SELECT t.id, t.efficiency_wh_per_mile
           FROM panda.trips t
           LEFT JOIN panda.trip_user_annotations tua ON tua.trip_id = t.id AND tua.user_id = $12
           WHERE t.vehicle_id = $1
             AND t.id <> $2
             AND t.started_at < $3
             AND t.efficiency_wh_per_mile > 0
             AND CASE WHEN $4::uuid IS NOT NULL
                      THEN COALESCE(tua.start_geofence_id, t.start_geofence_id) = $4::uuid
                      ELSE round(t.start_lat::numeric, $10) = round($5::float8::numeric, $10)
                       AND round(t.start_lng::numeric, $10) = round($6::float8::numeric, $10) END
             AND CASE WHEN $7::uuid IS NOT NULL
                      THEN COALESCE(tua.end_geofence_id, t.end_geofence_id) = $7::uuid
                      ELSE round(t.end_lat::numeric, $10) = round($8::float8::numeric, $10)
                       AND round(t.end_lng::numeric, $10) = round($9::float8::numeric, $10) END
           ORDER BY t.started_at DESC
           LIMIT $11"#,
    )
    .bind(vehicle_id)
    .bind(trip_id)
    .bind(trip.started_at)
    .bind(start_place)
    .bind(start_lat)
    .bind(start_lng)
    .bind(end_place)
    .bind(end_lat)
    .bind(end_lng)
    .bind(ROUTE_COORDINATE_DECIMALS)
    .bind(ROUTE_MAX_EARLIER_DRIVES)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let efficiencies: Vec<f64> = earlier
        .iter()
        .filter_map(|(_, wh)| mi_per_kwh(Some(*wh)))
        .collect();
    let Some(values) = route_comparison(this_mi_per_kwh, &efficiencies) else {
        return Ok(None);
    };
    let mut evidence = vec![InsightEvidence::trip(trip_id)];
    evidence.extend(earlier.iter().map(|(id, _)| InsightEvidence::trip(*id)));
    Ok(Some(insight(
        InsightKind::RouteComparison,
        InsightScope::Trip(trip_id),
        InsightSeverity::Info,
        &values,
        Some(InsightBaseline {
            method: "median",
            window: "earlier_drives_same_route",
            count: values.count as i64,
        }),
        values.count,
        evidence,
    )))
}

async fn load_days_of_range(
    pool: &PgPool,
    vehicle_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<Insight>, AppError> {
    let tz_name = crate::services::app_settings::load_app_timezone_name(pool).await?;
    let tz: Tz = tz_name.parse().unwrap_or(chrono_tz::UTC);
    let (from, to) = local_day_window(now, tz, RANGE_WINDOW_DAYS);

    let range_mi = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT distance_to_empty_mi FROM timeseries.telemetry \
         WHERE vehicle_id = $1 AND ts >= $2 AND ts <= $3 AND distance_to_empty_mi IS NOT NULL \
         ORDER BY ts DESC LIMIT 1",
    )
    .bind(vehicle_id)
    .bind(now - Duration::days(RANGE_WINDOW_DAYS))
    .bind(now)
    .fetch_optional(pool)
    .await?
    .flatten();

    // A day with data has telemetry or a drive. A day with telemetry and no
    // drive counts as 0 miles.
    let daily_miles = sqlx::query_scalar::<_, f64>(
        r#"WITH data_days AS (
               SELECT DISTINCT (bucket AT TIME ZONE $4)::date AS day
               FROM timeseries.telemetry_1min
               WHERE vehicle_id = $1 AND bucket >= $2 AND bucket < $3
           ),
           drive_days AS (
               SELECT (started_at AT TIME ZONE $4)::date AS day, SUM(distance_miles)::float8 AS miles
               FROM panda.trips
               WHERE vehicle_id = $1 AND started_at >= $2 AND started_at < $3 AND distance_miles > 0
               GROUP BY 1
           )
           SELECT COALESCE(dr.miles, 0)::float8
           FROM (SELECT day FROM data_days UNION SELECT day FROM drive_days) d
           LEFT JOIN drive_days dr ON dr.day = d.day
           ORDER BY d.day"#,
    )
    .bind(vehicle_id)
    .bind(from)
    .bind(to)
    .bind(tz.name())
    .fetch_all(pool)
    .await?;

    let Some(values) = days_of_range(range_mi, &daily_miles) else {
        return Ok(None);
    };
    Ok(Some(insight(
        InsightKind::DaysOfRange,
        InsightScope::Now,
        InsightSeverity::Info,
        &values,
        Some(InsightBaseline {
            method: "median",
            window: "last_30_days",
            count: values.days_with_data as i64,
        }),
        values.days_with_data,
        vec![InsightEvidence::vehicle(vehicle_id)],
    )))
}

async fn load_high_parked_loss(
    pool: &PgPool,
    vehicle_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<Insight>, AppError> {
    // A parked period ends when a completed drive or charge starts. Without
    // such a start in the last PARKED_MAX_AGE_DAYS no park is current, so the
    // rule is silent and the heavy 90-day period query does not run.
    let recent_park_end = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS (
               SELECT 1 FROM panda.trips
               WHERE vehicle_id = $1 AND ended_at IS NOT NULL AND started_at >= $2 AND started_at <= $3
           ) OR EXISTS (
               SELECT 1 FROM panda.charge_sessions
               WHERE vehicle_id = $1 AND ended_at IS NOT NULL AND started_at >= $2 AND started_at <= $3
           )"#,
    )
    .bind(vehicle_id)
    .bind(now - Duration::days(PARKED_MAX_AGE_DAYS))
    .bind(now)
    .fetch_one(pool)
    .await?;
    if !recent_park_end {
        return Ok(None);
    }

    // Same validated parked periods as the Parked page, newest first.
    let periods = crate::routes::idle_drain::fetch_idle_drain_periods(
        pool,
        vehicle_id,
        now - Duration::days(PARKED_WINDOW_DAYS),
        now,
        PARKED_MIN_DURATION_HOURS,
        PARKED_MAX_PERIODS,
        false,
    )
    .await?;
    let losses: Vec<ParkedLoss> = periods
        .iter()
        .filter_map(|period| {
            Some(ParkedLoss {
                started_at: period.period_start?,
                ended_at: period.period_end?,
                duration_hours: period.duration_hours.unwrap_or_default(),
                loss_pct: period.soc_lost_pct.filter(|v| v.is_finite())?,
                drain_pct_per_hour: period.drain_pct_per_hour.filter(|v| v.is_finite())?,
            })
        })
        .collect();
    let Some((latest, earlier)) = losses.split_first() else {
        return Ok(None);
    };
    let Some(values) = high_parked_loss(latest, earlier, now) else {
        return Ok(None);
    };
    let count = earlier.len();
    Ok(Some(insight(
        InsightKind::HighParkedLoss,
        InsightScope::Now,
        InsightSeverity::Attention,
        &values,
        Some(InsightBaseline {
            method: "median",
            window: "earlier_parks_90_days",
            count: count as i64,
        }),
        count,
        vec![InsightEvidence {
            kind: "parked_period",
            id: format!(
                "{}/{}",
                latest.started_at.to_rfc3339(),
                latest.ended_at.to_rfc3339()
            ),
        }],
    )))
}

async fn load_missing_price(
    pool: &PgPool,
    vehicle_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<Insight>, AppError> {
    // `unknown` is the method the cost service stores when no price applies.
    // Older rows without a method and without a cost count the same way.
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        r#"SELECT id, COUNT(*) OVER ()::int8
           FROM panda.charge_sessions
           WHERE vehicle_id = $1
             AND started_at >= $2
             AND ended_at IS NOT NULL
             AND (cost_method = 'unknown' OR (cost_method IS NULL AND cost_usd IS NULL))
           ORDER BY started_at DESC
           LIMIT $3"#,
    )
    .bind(vehicle_id)
    .bind(now - Duration::days(MISSING_PRICE_WINDOW_DAYS))
    .bind(MISSING_PRICE_MAX_EVIDENCE)
    .fetch_all(pool)
    .await?;
    let count = rows.first().map(|(_, count)| *count).unwrap_or(0);
    let Some(values) = missing_price(count) else {
        return Ok(None);
    };
    Ok(Some(insight(
        InsightKind::MissingPrice,
        InsightScope::Now,
        InsightSeverity::Info,
        &values,
        None,
        usize::try_from(count).unwrap_or_default(),
        rows.iter()
            .map(|(id, _)| InsightEvidence::charge(*id))
            .collect(),
    )))
}

async fn load_stale_data(
    pool: &PgPool,
    vehicle_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<Insight>, AppError> {
    let last_seen_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT ts FROM timeseries.telemetry WHERE vehicle_id = $1 ORDER BY ts DESC LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;
    let Some(values) = stale_data(last_seen_at, now) else {
        return Ok(None);
    };
    Ok(Some(insight(
        InsightKind::StaleData,
        InsightScope::Now,
        InsightSeverity::Attention,
        &values,
        None,
        1,
        vec![InsightEvidence::vehicle(vehicle_id)],
    )))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ChargeBoundsRow {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<String>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
}

async fn load_charge_insights(
    pool: &PgPool,
    vehicle_id: Uuid,
    charge_id: Uuid,
) -> Result<Vec<Option<Insight>>, AppError> {
    let session = sqlx::query_as::<_, ChargeBoundsRow>(concat!(
        "SELECT cs.id, cs.started_at, cs.ended_at, ",
        crate::charger_type_sql!("cs"),
        " AS charger_type, cs.soc_start, cs.soc_end \
         FROM panda.charge_sessions cs WHERE cs.id = $1 AND cs.vehicle_id = $2"
    ))
    .bind(charge_id)
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;

    // An open charge has no end, so its curve span is not bounded. The shape
    // rules wait until the charge ends.
    let Some(ended_at) = session.ended_at else {
        return Ok(Vec::new());
    };
    if session.charger_type.as_deref() != Some("dc") {
        return Ok(Vec::new());
    }
    let scope = InsightScope::Charge(charge_id);
    let this_curve = load_measured_curve(pool, vehicle_id, &session, ended_at).await?;

    let shape = charge_shape(&this_curve).map(|values| {
        insight(
            InsightKind::ChargeShape,
            scope,
            InsightSeverity::Info,
            &values,
            None,
            clean_curve(&this_curve).len(),
            vec![InsightEvidence::charge(charge_id)],
        )
    });

    let earlier_sessions = sqlx::query_as::<_, ChargeBoundsRow>(concat!(
        "SELECT cs.id, cs.started_at, cs.ended_at, ",
        crate::charger_type_sql!("cs"),
        " AS charger_type, cs.soc_start, cs.soc_end \
         FROM panda.charge_sessions cs \
         WHERE cs.vehicle_id = $1 AND cs.id <> $2 AND cs.started_at < $3 AND cs.ended_at IS NOT NULL \
           AND ",
        crate::charger_type_sql!("cs"),
        " = 'dc' ORDER BY cs.started_at DESC LIMIT $4"
    ))
    .bind(vehicle_id)
    .bind(charge_id)
    .bind(session.started_at)
    .bind(DC_BAND_MAX_SESSIONS)
    .fetch_all(pool)
    .await?;

    // The query reads ended charges only. Load DC_BAND_CONCURRENT_LOADS
    // curves at a time so one request does not hold many pool connections.
    let mut earlier: Vec<(Uuid, Vec<CurvePoint>)> = Vec::with_capacity(earlier_sessions.len());
    for batch in earlier_sessions.chunks(DC_BAND_CONCURRENT_LOADS) {
        let curves = try_join_all(batch.iter().filter_map(|earlier| {
            let ended_at = earlier.ended_at?;
            Some(async move {
                Ok::<_, AppError>((
                    earlier.id,
                    load_measured_curve(pool, vehicle_id, earlier, ended_at).await?,
                ))
            })
        }))
        .await?;
        earlier.extend(curves);
    }

    let band = usual_dc_band(&this_curve, &earlier).map(|values| {
        let mut evidence = vec![InsightEvidence::charge(charge_id)];
        evidence.extend(
            values
                .used_sessions
                .iter()
                .map(|id| InsightEvidence::charge(*id)),
        );
        insight(
            InsightKind::UsualDcBand,
            scope,
            InsightSeverity::Info,
            &values,
            Some(InsightBaseline {
                method: "p10_p90",
                window: "earlier_dc_charges",
                count: values.sessions as i64,
            }),
            values.sessions,
            evidence,
        )
    });

    Ok(vec![shape, band])
}

/// Measured charge power for a session, in the session chart's source order:
/// telemetry power when the vehicle sends it, else the recorded Parallax and
/// Rivian points. The SoC-delta estimate is not used, because an insight
/// describes measured power only. Recorded points without a battery % get
/// one interpolated between the session start and end battery %.
async fn load_measured_curve(
    pool: &PgPool,
    vehicle_id: Uuid,
    session: &ChargeBoundsRow,
    ended_at: DateTime<Utc>,
) -> Result<Vec<CurvePoint>, AppError> {
    let telemetry = sqlx::query_as::<_, (f64, f64, Option<f64>)>(
        r#"SELECT EXTRACT(EPOCH FROM (time_bucket('1 minute', ts) - $3))::float8 / 60.0,
                  AVG(ABS(power_kw))::float8,
                  AVG(battery_level)::float8
           FROM timeseries.telemetry
           WHERE vehicle_id = $1 AND charge_session_id = $2 AND ts >= $3 AND ts <= $4
             AND power_kw IS NOT NULL
           GROUP BY time_bucket('1 minute', ts)
           ORDER BY 1"#,
    )
    .bind(vehicle_id)
    .bind(session.id)
    .bind(session.started_at)
    .bind(ended_at)
    .fetch_all(pool)
    .await?;
    if telemetry.len() >= 2 {
        return Ok(telemetry
            .into_iter()
            .map(|(minute, kw, soc)| CurvePoint { minute, kw, soc })
            .collect());
    }

    let recorded = sqlx::query_as::<_, (DateTime<Utc>, f64, Option<f64>)>(
        r#"WITH candidates AS (
               SELECT ts, power_kw, NULL::float8 AS soc, 1 AS precedence
               FROM panda.rivian_charge_curve_points
               WHERE vehicle_id = $1 AND charge_session_id = $2 AND power_kw > 0
               UNION ALL
               SELECT source_at AS ts, power_kw, soc, 2 AS precedence
               FROM timeseries.parallax_charge_curve_points
               WHERE vehicle_id = $1 AND charge_session_id = $2 AND power_kw > 0
           )
           SELECT DISTINCT ON (ts) ts, power_kw, soc
           FROM candidates
           WHERE ts >= $3 - interval '2 minutes' AND ts <= $4 + interval '2 minutes'
           ORDER BY ts, precedence DESC"#,
    )
    .bind(vehicle_id)
    .bind(session.id)
    .bind(session.started_at)
    .bind(ended_at)
    .fetch_all(pool)
    .await?;

    Ok(recorded_curve_points(
        session.started_at,
        session.soc_start.zip(session.soc_end),
        &recorded,
    ))
}

/// Turn recorded points into curve points. A point without a battery % gets
/// one interpolated by time between the session start and end battery %.
pub fn recorded_curve_points(
    started_at: DateTime<Utc>,
    soc_bounds: Option<(f64, f64)>,
    recorded: &[(DateTime<Utc>, f64, Option<f64>)],
) -> Vec<CurvePoint> {
    let (Some(first), Some(last)) = (recorded.first(), recorded.last()) else {
        return Vec::new();
    };
    let span_seconds = (last.0 - first.0).num_seconds().max(1) as f64;
    recorded
        .iter()
        .map(|(ts, kw, soc)| {
            let soc = soc.or_else(|| {
                let (soc_start, soc_end) = soc_bounds?;
                let ratio = ((*ts - first.0).num_seconds() as f64 / span_seconds).clamp(0.0, 1.0);
                Some(soc_start + (soc_end - soc_start) * ratio)
            });
            CurvePoint {
                minute: (*ts - started_at).num_milliseconds() as f64 / 60_000.0,
                kw: *kw,
                soc,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(hours: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap() + Duration::hours(hours)
    }

    fn curve(rows: &[(f64, f64, f64)]) -> Vec<CurvePoint> {
        rows.iter()
            .map(|(minute, kw, soc)| CurvePoint {
                minute: *minute,
                kw: *kw,
                soc: Some(*soc),
            })
            .collect()
    }

    #[test]
    fn scope_parses_now_trip_and_charge() {
        let id = Uuid::new_v4();
        assert_eq!(InsightScope::parse(None).unwrap(), InsightScope::Now);
        assert_eq!(InsightScope::parse(Some("now")).unwrap(), InsightScope::Now);
        assert_eq!(
            InsightScope::parse(Some(&format!("trip:{id}"))).unwrap(),
            InsightScope::Trip(id)
        );
        assert_eq!(
            InsightScope::parse(Some(&format!("charge:{id}"))).unwrap(),
            InsightScope::Charge(id)
        );
        assert!(InsightScope::parse(Some("trip:not-a-uuid")).is_err());
        assert!(InsightScope::parse(Some(&format!("park:{id}"))).is_err());
        assert!(InsightScope::parse(Some("later")).is_err());
    }

    #[test]
    fn insight_serializes_contract_fields() {
        let values = missing_price(2).unwrap();
        let item = insight(
            InsightKind::MissingPrice,
            InsightScope::Now,
            InsightSeverity::Info,
            &values,
            None,
            2,
            vec![InsightEvidence::charge(Uuid::nil())],
        );
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "missing_price:now");
        assert_eq!(json["kind"], "missing_price");
        assert_eq!(json["severity"], "info");
        assert_eq!(json["values"]["count"], 2);
        assert!(json["baseline"].is_null());
        assert_eq!(json["sample_size"], 2);
        assert_eq!(json["evidence"][0]["type"], "charge");
    }

    #[test]
    fn route_comparison_ranks_against_median_of_earlier_drives() {
        let values = route_comparison(3.3, &[2.8, 3.0, 3.0, 3.1, 3.5]).unwrap();
        assert_eq!(values.usual_mi_per_kwh, 3.0);
        assert!((values.pct_diff - 10.0).abs() < 1e-9);
        assert_eq!(values.rank, 2);
        assert_eq!(values.count, 5);
    }

    #[test]
    fn route_comparison_needs_five_earlier_drives() {
        assert_eq!(route_comparison(3.3, &[2.8, 3.0, 3.1, 3.5]), None);
        assert_eq!(
            route_comparison(3.3, &[2.8, 3.0, 3.1, 3.5, f64::NAN, 0.0]),
            None
        );
        assert_eq!(route_comparison(0.0, &[2.8, 3.0, 3.0, 3.1, 3.5]), None);
    }

    #[test]
    fn route_endpoint_prefers_saved_place() {
        let place = Uuid::new_v4();
        assert_eq!(
            RouteEndpoint::from_trip(Some(place), Some(1.0), Some(2.0)),
            Some(RouteEndpoint::Place(place))
        );
        assert_eq!(
            RouteEndpoint::from_trip(None, Some(1.0), Some(2.0)),
            Some(RouteEndpoint::Coordinates { lat: 1.0, lng: 2.0 })
        );
        assert_eq!(RouteEndpoint::from_trip(None, None, Some(2.0)), None);
        assert_eq!(mi_per_kwh(Some(400.0)), Some(2.5));
        assert_eq!(mi_per_kwh(Some(0.0)), None);
    }

    #[test]
    fn days_of_range_divides_range_by_median_daily_miles() {
        let mut days = vec![30.0; 10];
        days.extend([0.0; 4]);
        days.push(80.0);
        let values = days_of_range(Some(90.0), &days).unwrap();
        assert_eq!(values.median_daily_mi, 30.0);
        assert!((values.days - 3.0).abs() < 1e-9);
        assert_eq!(values.days_with_data, 15);
    }

    #[test]
    fn days_of_range_needs_fourteen_days_and_driving() {
        assert_eq!(days_of_range(Some(90.0), &[30.0; 13]), None);
        assert_eq!(days_of_range(Some(90.0), &[0.0; 20]), None);
        assert_eq!(days_of_range(None, &[30.0; 20]), None);
    }

    #[test]
    fn local_day_window_uses_whole_local_days() {
        let tz: Tz = "America/Los_Angeles".parse().unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 7, 18, 0, 0).unwrap();
        let (from, to) = local_day_window(now, tz, 30);
        assert_eq!(to, Utc.with_ymd_and_hms(2026, 11, 7, 8, 0, 0).unwrap());
        assert_eq!(from, Utc.with_ymd_and_hms(2026, 10, 8, 7, 0, 0).unwrap());
    }

    /// A park that ends at `ts(end_hour)` after `hours` hours.
    fn park(hours: f64, loss_pct: f64, end_hour: i64) -> ParkedLoss {
        ParkedLoss {
            started_at: ts(end_hour) - Duration::minutes((hours * 60.0) as i64),
            ended_at: ts(end_hour),
            duration_hours: hours,
            loss_pct,
            drain_pct_per_hour: loss_pct / hours,
        }
    }

    /// Seven earlier 10-hour parks at about 0.1 points per hour.
    fn usual_parks() -> Vec<ParkedLoss> {
        [1.0, 1.0, 1.0, 1.2, 0.8, 1.0, 1.1]
            .iter()
            .map(|loss| park(10.0, *loss, -24))
            .collect()
    }

    #[test]
    fn high_parked_loss_flags_twice_the_usual_drain_rate() {
        // 3 points in 10 hours: 0.3 per hour against a usual 0.1 per hour.
        let values = high_parked_loss(&park(10.0, 3.0, 0), &usual_parks(), ts(1)).unwrap();
        assert!((values.usual_drain_pct_per_hour - 0.1).abs() < 1e-9);
        assert!((values.drain_pct_per_hour - 0.3).abs() < 1e-9);
        assert!((values.ratio.unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(values.loss_pct, 3.0);
        assert_eq!(values.usual_loss_pct, 1.0);
        assert_eq!(values.duration_hours, 10.0);
    }

    #[test]
    fn high_parked_loss_compares_rate_not_total_loss() {
        // A long park at the usual rate loses many points but is normal.
        assert_eq!(
            high_parked_loss(&park(100.0, 10.0, 0), &usual_parks(), ts(1)),
            None
        );
        // A short park with a fast drain is flagged although its total loss is
        // close to the usual total.
        let fast = high_parked_loss(&park(6.0, 2.4, 0), &usual_parks(), ts(1)).unwrap();
        assert!((fast.ratio.unwrap() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn high_parked_loss_gates() {
        let earlier = usual_parks();
        // Six earlier parks are not enough.
        assert_eq!(
            high_parked_loss(&park(10.0, 3.0, 0), &earlier[..6], ts(1)),
            None
        );
        // Below 2 percentage points, even at a fast rate.
        assert_eq!(high_parked_loss(&park(6.0, 1.9, 0), &earlier, ts(1)), None);
        // Exactly 2 points at a fast rate is flagged.
        assert!(high_parked_loss(&park(6.0, 2.0, 0), &earlier, ts(1)).is_some());
        // 3 points in 20 hours is 0.15 per hour, less than twice the usual rate.
        assert_eq!(high_parked_loss(&park(20.0, 3.0, 0), &earlier, ts(1)), None);
        // Exactly twice the usual rate is flagged.
        assert!(high_parked_loss(&park(10.0, 2.0, 0), &earlier, ts(1)).is_some());
        // A zero usual rate still flags, with no ratio.
        let still: Vec<ParkedLoss> = (0..7).map(|_| park(10.0, 0.0, -24)).collect();
        assert_eq!(
            high_parked_loss(&park(10.0, 2.5, 0), &still, ts(1))
                .unwrap()
                .ratio,
            None
        );
    }

    #[test]
    fn high_parked_loss_skips_a_park_that_ended_more_than_three_days_ago() {
        let latest = park(10.0, 3.0, 0);
        let earlier = usual_parks();
        let limit = ts(0) + Duration::days(PARKED_MAX_AGE_DAYS);
        assert!(high_parked_loss(&latest, &earlier, limit).is_some());
        assert_eq!(
            high_parked_loss(&latest, &earlier, limit + Duration::minutes(1)),
            None
        );
    }

    fn dc_curve() -> Vec<CurvePoint> {
        curve(&[
            (0.0, 40.0, 10.0),
            (1.0, 90.0, 12.0),
            (2.0, 150.0, 15.0),
            (3.0, 186.0, 19.0),
            (4.0, 189.5, 23.0),
            (5.0, 185.0, 27.0),
            (6.0, 160.0, 31.0),
            (8.0, 120.0, 38.0),
            (10.0, 80.0, 44.0),
        ])
    }

    #[test]
    fn charge_shape_finds_climb_peak_and_taper() {
        let values = charge_shape(&dc_curve()).unwrap();
        assert_eq!(values.peak_kw, 189.5);
        assert_eq!(values.peak_soc, Some(23.0));
        assert_eq!(values.peak_minute, 4.0);
        assert_eq!(values.low_kw, 80.0);
        assert_eq!(values.low_soc, Some(44.0));
        let names: Vec<_> = values.phases.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            vec![
                ChargePhaseName::Climb,
                ChargePhaseName::Peak,
                ChargePhaseName::Taper
            ]
        );
        // 186 and 185 kW are within 3% of 189.5 kW (>= 183.8 kW).
        assert_eq!(values.phases[1].start_min, 3.0);
        assert_eq!(values.phases[1].end_min, 5.0);
        assert_eq!(values.phases[0].from_kw, 40.0);
        assert_eq!(values.phases[2].to_kw, 80.0);
    }

    #[test]
    fn charge_shape_needs_eight_plausible_points() {
        let short = &dc_curve()[..7];
        assert_eq!(charge_shape(short), None);
        let mut with_spike = dc_curve()[..7].to_vec();
        with_spike.push(CurvePoint {
            minute: 11.0,
            kw: 900.0,
            soc: None,
        });
        assert_eq!(charge_shape(&with_spike), None);
    }

    #[test]
    fn recorded_points_interpolate_missing_battery_percent() {
        let started = ts(0);
        let points = recorded_curve_points(
            started,
            Some((20.0, 60.0)),
            &[
                (ts(0), 100.0, None),
                (ts(1), 50.0, None),
                (ts(2), 20.0, Some(70.0)),
            ],
        );
        assert_eq!(points[0].soc, Some(20.0));
        assert_eq!(points[1].soc, Some(40.0));
        assert_eq!(points[2].soc, Some(70.0));
        assert_eq!(points[1].minute, 60.0);
    }

    #[test]
    fn usual_dc_band_builds_p10_p90_per_battery_percent() {
        let earlier: Vec<(Uuid, Vec<CurvePoint>)> = [100.0, 110.0, 120.0, 130.0]
            .iter()
            .map(|kw| (Uuid::new_v4(), curve(&[(0.0, *kw, 20.0), (1.0, *kw, 21.5)])))
            .collect();
        let this = curve(&[(0.0, 115.0, 20.2), (1.0, 200.0, 21.1)]);
        let values = usual_dc_band(&this, &earlier).unwrap();
        assert_eq!(values.sessions, 4);
        assert_eq!(values.bands.len(), 2);
        let band = &values.bands[0];
        assert_eq!(band.soc, 20);
        assert!((band.p10_kw - 103.0).abs() < 1e-9);
        assert!((band.p90_kw - 127.0).abs() < 1e-9);
        assert_eq!(values.compared_bins, 2);
        assert_eq!(values.within_band_pct, Some(50.0));
        assert_eq!(values.used_sessions.len(), 4);
    }

    #[test]
    fn usual_dc_band_needs_three_earlier_charges_with_battery_percent() {
        let mut earlier: Vec<(Uuid, Vec<CurvePoint>)> = (0..2)
            .map(|_| (Uuid::new_v4(), curve(&[(0.0, 100.0, 20.0)])))
            .collect();
        // A third charge without battery % does not count.
        earlier.push((
            Uuid::new_v4(),
            vec![CurvePoint {
                minute: 0.0,
                kw: 100.0,
                soc: None,
            }],
        ));
        assert_eq!(usual_dc_band(&[], &earlier), None);
    }

    #[test]
    fn missing_price_counts_unpriced_charges() {
        assert_eq!(missing_price(2).unwrap().count, 2);
        assert_eq!(missing_price(0), None);
    }

    #[test]
    fn stale_data_flags_more_than_six_hours() {
        let values = stale_data(Some(ts(0)), ts(7)).unwrap();
        assert!((values.hours - 7.0).abs() < 1e-9);
        assert_eq!(stale_data(Some(ts(0)), ts(6)), None);
        assert_eq!(stale_data(None, ts(7)), None);
    }

    #[test]
    fn statistics_helpers_match_percentile_cont() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
        assert_eq!(
            percentile_sorted(&[100.0, 110.0, 120.0, 130.0], 0.9),
            Some(127.0)
        );
    }
}

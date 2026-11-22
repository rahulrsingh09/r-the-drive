//! Find probable missed charges: the vehicle was parked (no trip and no charge
//! session) for at least 30 minutes, and stored telemetry shows the battery
//! level rose by more than 2 percentage points during that time.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// How far back stored telemetry is searched.
pub const LOOKBACK_DAYS: i64 = 30;
/// A parked period must last at least this long.
const MIN_PARKED_MINUTES: i64 = 30;
/// The battery level must rise by more than this many percentage points.
const MIN_RISE_PERCENT: f64 = 2.0;
/// Readings settle after a drive; a charge session can end before the last
/// reported battery level arrives. These margins keep both out of a gap.
const TRIP_MARGIN_MINUTES: i64 = 10;
const CHARGE_MARGIN_MINUTES: i64 = 15;
/// Live capture may still be opening a session for a charge that just began.
const SETTLE_MINUTES: i64 = 15;
/// An open session without an end counts as charging for at most this long.
const OPEN_SESSION_MAX_HOURS: i64 = 48;
/// Readings are averaged per bucket so a month of telemetry stays small.
const BUCKET_MINUTES: i64 = 5;
const MAX_GAPS: usize = 50;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChargeGap {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub battery_from: f64,
    pub battery_to: f64,
}

/// The `GET /vehicles/{id}/charge-gaps` response.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChargeGapReport {
    pub vehicle_id: Uuid,
    pub lookback_days: i64,
    /// Parked periods of at least 30 minutes that the scan examined.
    pub parked_periods_checked: usize,
    /// Newest first.
    pub gaps: Vec<ChargeGap>,
    /// Stored telemetry rows per local day (app timezone), oldest first.
    pub daily_samples: Vec<DailySamples>,
}

#[derive(Debug, Clone, Serialize, PartialEq, sqlx::FromRow)]
pub struct DailySamples {
    /// Local calendar day, `YYYY-MM-DD`.
    pub day: String,
    pub samples: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub parked_periods_checked: usize,
    pub gaps: Vec<ChargeGap>,
}

#[derive(Debug, Clone, Copy, PartialEq, sqlx::FromRow)]
pub struct BatterySample {
    pub ts: DateTime<Utc>,
    pub battery_level: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

pub async fn find_charge_gaps(
    pool: &PgPool,
    vehicle_id: Uuid,
    timezone: &str,
) -> Result<ChargeGapReport> {
    let now = Utc::now();
    let window_start = now - Duration::days(LOOKBACK_DAYS);
    let window_end = now - Duration::minutes(SETTLE_MINUTES);

    let samples = sqlx::query_as::<_, BatterySample>(
        "SELECT time_bucket(make_interval(mins => $4::int), ts) AS ts,
                avg(battery_level) AS battery_level
         FROM timeseries.telemetry
         WHERE vehicle_id = $1 AND ts >= $2 AND ts <= $3
           AND battery_level BETWEEN 0 AND 100
         GROUP BY 1
         ORDER BY 1",
    )
    .bind(vehicle_id)
    .bind(window_start)
    .bind(window_end)
    .bind(BUCKET_MINUTES as i32)
    .fetch_all(pool)
    .await?;

    let trips: Vec<(DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT started_at, ended_at FROM panda.trips
         WHERE vehicle_id = $1 AND ended_at >= $2 AND started_at <= $3",
    )
    .bind(vehicle_id)
    .bind(window_start - Duration::minutes(TRIP_MARGIN_MINUTES))
    .bind(window_end + Duration::minutes(TRIP_MARGIN_MINUTES))
    .fetch_all(pool)
    .await?;

    let charges: Vec<(DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT started_at,
                COALESCE(ended_at, LEAST(started_at + make_interval(hours => $4::int), $5))
         FROM panda.charge_sessions
         WHERE vehicle_id = $1
           AND started_at <= $3
           AND COALESCE(ended_at, LEAST(started_at + make_interval(hours => $4::int), $5)) >= $2",
    )
    .bind(vehicle_id)
    .bind(window_start - Duration::minutes(CHARGE_MARGIN_MINUTES))
    .bind(window_end + Duration::minutes(CHARGE_MARGIN_MINUTES))
    .bind(OPEN_SESSION_MAX_HOURS as i32)
    .bind(now)
    .fetch_all(pool)
    .await?;

    let daily_samples = sqlx::query_as::<_, DailySamples>(
        "SELECT to_char((ts AT TIME ZONE $3)::date, 'YYYY-MM-DD') AS day,
                count(*)::int8 AS samples
         FROM timeseries.telemetry
         WHERE vehicle_id = $1 AND ts >= $2
         GROUP BY 1
         ORDER BY 1",
    )
    .bind(vehicle_id)
    .bind(window_start)
    .bind(timezone)
    .fetch_all(pool)
    .await?;

    let busy = busy_intervals(&trips, &charges);
    let detection = detect_charge_gaps(
        &samples,
        &busy,
        Interval {
            start: window_start,
            end: window_end,
        },
    );
    Ok(ChargeGapReport {
        vehicle_id,
        lookback_days: LOOKBACK_DAYS,
        parked_periods_checked: detection.parked_periods_checked,
        gaps: detection.gaps,
        daily_samples,
    })
}

/// Trips and charge sessions, widened by their margins.
pub fn busy_intervals(
    trips: &[(DateTime<Utc>, DateTime<Utc>)],
    charges: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Vec<Interval> {
    let widen = |rows: &[(DateTime<Utc>, DateTime<Utc>)], minutes: i64| {
        let margin = Duration::minutes(minutes);
        rows.iter()
            .map(move |(start, end)| Interval {
                start: *start - margin,
                end: (*end).max(*start) + margin,
            })
            .collect::<Vec<_>>()
    };
    let mut busy = widen(trips, TRIP_MARGIN_MINUTES);
    busy.extend(widen(charges, CHARGE_MARGIN_MINUTES));
    busy
}

/// The parked periods are the parts of `window` that no busy interval covers.
/// Each parked period of at least 30 minutes yields at most one gap: its
/// largest battery-level rise, if that rise is more than 2 points.
pub fn detect_charge_gaps(
    samples: &[BatterySample],
    busy: &[Interval],
    window: Interval,
) -> Detection {
    let parked: Vec<Interval> = parked_periods(busy, window)
        .into_iter()
        .filter(|parked| parked.end - parked.start >= Duration::minutes(MIN_PARKED_MINUTES))
        .collect();
    let mut gaps: Vec<ChargeGap> = parked
        .iter()
        .filter_map(|parked| {
            largest_rise(
                samples
                    .iter()
                    .filter(|sample| sample.ts >= parked.start && sample.ts <= parked.end),
            )
        })
        .filter(|gap| gap.battery_to - gap.battery_from > MIN_RISE_PERCENT)
        .map(|gap| ChargeGap {
            battery_from: round_tenth(gap.battery_from),
            battery_to: round_tenth(gap.battery_to),
            ..gap
        })
        .collect();
    gaps.sort_by_key(|gap| std::cmp::Reverse(gap.start));
    gaps.truncate(MAX_GAPS);
    Detection {
        parked_periods_checked: parked.len(),
        gaps,
    }
}

fn parked_periods(busy: &[Interval], window: Interval) -> Vec<Interval> {
    let mut busy: Vec<Interval> = busy
        .iter()
        .filter(|interval| interval.end > window.start && interval.start < window.end)
        .copied()
        .collect();
    busy.sort_by_key(|interval| interval.start);
    let mut parked = Vec::new();
    let mut cursor = window.start;
    for interval in busy {
        if interval.start > cursor {
            parked.push(Interval {
                start: cursor,
                end: interval.start,
            });
        }
        cursor = cursor.max(interval.end);
    }
    if cursor < window.end {
        parked.push(Interval {
            start: cursor,
            end: window.end,
        });
    }
    parked
}

/// The largest rise from an earlier low reading to a later high reading.
fn largest_rise<'a>(samples: impl Iterator<Item = &'a BatterySample>) -> Option<ChargeGap> {
    let mut low: Option<BatterySample> = None;
    let mut best: Option<ChargeGap> = None;
    for sample in samples {
        let Some(current_low) = low else {
            low = Some(*sample);
            continue;
        };
        if sample.battery_level < current_low.battery_level {
            low = Some(*sample);
            continue;
        }
        let rise = sample.battery_level - current_low.battery_level;
        if best
            .as_ref()
            .is_none_or(|gap| rise > gap.battery_to - gap.battery_from)
        {
            best = Some(ChargeGap {
                start: current_low.ts,
                end: sample.ts,
                battery_from: current_low.battery_level,
                battery_to: sample.battery_level,
            });
        }
    }
    best.filter(|gap| gap.end > gap.start)
}

fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, hour, minute, 0).unwrap()
    }

    fn sample(hour: u32, minute: u32, battery_level: f64) -> BatterySample {
        BatterySample {
            ts: at(hour, minute),
            battery_level,
        }
    }

    fn window() -> Interval {
        Interval {
            start: at(0, 0),
            end: at(23, 0),
        }
    }

    #[test]
    fn reports_a_rise_while_parked_with_no_charge_session() {
        let samples = [
            sample(1, 0, 40.0),
            sample(1, 30, 39.8),
            sample(2, 0, 45.0),
            sample(3, 0, 55.04),
            sample(3, 30, 54.9),
        ];
        let gaps = detect_charge_gaps(&samples, &[], window()).gaps;
        assert_eq!(
            gaps,
            [ChargeGap {
                start: at(1, 30),
                end: at(3, 0),
                battery_from: 39.8,
                battery_to: 55.0,
            }]
        );
    }

    #[test]
    fn ignores_rises_covered_by_a_charge_session_or_its_margin() {
        let samples = [sample(1, 0, 40.0), sample(2, 0, 60.0), sample(2, 10, 61.0)];
        let charges = [(at(1, 5), at(1, 58))];
        let busy = busy_intervals(&[], &charges);
        // The 15-minute margins cover 1:00 and 2:10, so no parked reading
        // pair spans the rise.
        assert!(detect_charge_gaps(&samples, &busy, window())
            .gaps
            .is_empty());
    }

    #[test]
    fn ignores_rises_that_span_a_trip() {
        let samples = [sample(1, 0, 40.0), sample(3, 0, 60.0)];
        let busy = busy_intervals(&[(at(2, 0), at(2, 20))], &[]);
        assert!(detect_charge_gaps(&samples, &busy, window())
            .gaps
            .is_empty());
    }

    #[test]
    fn needs_more_than_two_points_and_thirty_parked_minutes() {
        let small = [sample(1, 0, 40.0), sample(2, 0, 42.0)];
        assert!(detect_charge_gaps(&small, &[], window()).gaps.is_empty());

        // Parked only from 4:10 to 4:35 between two trips: not checked.
        let short = [sample(4, 15, 40.0), sample(4, 30, 50.0)];
        let busy = busy_intervals(&[(at(3, 0), at(4, 0)), (at(4, 45), at(5, 0))], &[]);
        let detection = detect_charge_gaps(&short, &busy, window());
        assert!(detection.gaps.is_empty());
        // 0:00-2:50 and 5:10-23:00 are checked; 4:10-4:35 is too short.
        assert_eq!(detection.parked_periods_checked, 2);
    }

    #[test]
    fn a_falling_battery_is_not_a_gap() {
        let samples = [sample(1, 0, 60.0), sample(5, 0, 55.0), sample(9, 0, 50.0)];
        assert!(detect_charge_gaps(&samples, &[], window()).gaps.is_empty());
    }

    #[test]
    fn open_sessions_and_overlaps_merge_into_one_busy_period() {
        let busy = busy_intervals(
            &[(at(1, 0), at(2, 0))],
            &[(at(1, 30), at(3, 0)), (at(2, 50), at(2, 40))],
        );
        let parked = parked_periods(&busy, window());
        assert_eq!(
            parked,
            [
                Interval {
                    start: at(0, 0),
                    end: at(0, 50),
                },
                Interval {
                    start: at(3, 15),
                    end: at(23, 0),
                },
            ]
        );
    }

    #[test]
    fn lists_newest_gaps_first_one_per_parked_period() {
        let samples = [
            sample(1, 0, 20.0),
            sample(2, 0, 30.0),
            sample(6, 0, 30.0),
            sample(8, 0, 50.0),
        ];
        let busy = busy_intervals(&[(at(3, 0), at(4, 0))], &[]);
        let detection = detect_charge_gaps(&samples, &busy, window());
        assert_eq!(detection.parked_periods_checked, 2);
        let gaps = detection.gaps;
        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0].start, at(6, 0));
        assert_eq!(gaps[1].start, at(1, 0));
    }
}

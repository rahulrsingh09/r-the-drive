//! Find drives that stored telemetry shows but no trip records, and recover
//! one on request. Both replay telemetry through the live trip-signal fusion
//! and detector, so a listed gap is exactly what recovery saves.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Instant,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, DurationRound, Utc};
use futures::TryStreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::ingestion::{
    replay::{
        replay_power_is_change_only, row_to_event, ReplayTelemetryRow, REPLAY_TELEMETRY_COLUMNS,
    },
    trip_detector::{
        compute_distance_odometer_or_gps, CompletedTripData, TripDetectorState, TripEvent,
    },
    trip_signals::TripSignalFusion,
    worker::RecoveredSave,
};

/// How far back stored telemetry is searched.
pub const LOOKBACK_DAYS: i64 = 7;
/// Replay starts this much earlier so a drive near the window start sees the
/// same earlier readings each time, and keeps a stable ID.
const WARMUP_HOURS: i64 = 6;
/// Listing replays a week of telemetry; repeat views within this reuse it.
const LIST_CACHE_SECONDS: u64 = 60;
/// Live capture owns recent drives; it may still be finishing them.
const SETTLE_MINUTES: i64 = 15;
/// Two whole-kilometre odometer steps. A single step can be a late final
/// reading of the previous trip.
const MIN_ODOMETER_MILES: f64 = 1.0;
/// A recorded trip this close to a candidate is treated as the same drive.
const OVERLAP_MARGIN_MINUTES: i64 = 2;
const MAX_GAPS: usize = 10;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TripGap {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub distance_miles: f64,
    pub duration_seconds: i64,
    pub soc_start: Option<f64>,
    pub soc_end: Option<f64>,
}

#[derive(Debug)]
pub enum RecoverError {
    /// The gap is no longer listed: a trip covers it, or the readings changed.
    NotFound,
    /// A live trip completion for this vehicle is still pending.
    Busy,
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for RecoverError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed(error)
    }
}

impl From<sqlx::Error> for RecoverError {
    fn from(error: sqlx::Error) -> Self {
        Self::Failed(error.into())
    }
}

/// Parallax stopped sending frames, and the odometer later shows movement.
/// Power state then comes from no source, so drives rely on odometer motion.
#[derive(Debug, Clone, Serialize, PartialEq, sqlx::FromRow)]
pub struct ParallaxQuiet {
    pub last_frame_at: DateTime<Utc>,
    pub moved_at: DateTime<Utc>,
}

pub async fn parallax_quiet_while_moving(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<Option<ParallaxQuiet>> {
    // Half a mile: whole-kilometre steps are 0.62 mi.
    let quiet = sqlx::query_as::<_, ParallaxQuiet>(
        r#"WITH p AS (
               SELECT last_frame_at FROM panda.parallax_collector_state
               WHERE vehicle_id = $1 AND status <> 'disabled'
                 AND last_frame_at < now() - interval '15 minutes'
           ), base AS (
               SELECT t.odometer_miles FROM timeseries.telemetry t, p
               WHERE t.vehicle_id = $1 AND t.ts <= p.last_frame_at AND t.odometer_miles IS NOT NULL
               ORDER BY t.ts DESC LIMIT 1
           ), moved AS (
               SELECT t.ts AS moved_at FROM timeseries.telemetry t, p, base
               WHERE t.vehicle_id = $1
                 AND t.ts > greatest(p.last_frame_at, now() - interval '7 days')
                 AND t.odometer_miles >= base.odometer_miles + 0.5
               ORDER BY t.ts LIMIT 1
           )
           SELECT p.last_frame_at, moved.moved_at FROM p, moved"#,
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;
    Ok(quiet)
}

struct Candidate {
    gap: TripGap,
    trip: CompletedTripData,
    distance: f64,
}

#[derive(Default)]
struct ListCache {
    /// Increases on every recovery attempt, so a listing that started before
    /// it is not stored afterwards.
    generation: u64,
    listed: Option<(Instant, Vec<TripGap>)>,
}

static LIST_CACHE: LazyLock<Mutex<HashMap<Uuid, ListCache>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub async fn find_trip_gaps(pool: &PgPool, vehicle_id: Uuid) -> Result<Vec<TripGap>> {
    let generation = {
        let mut cache = LIST_CACHE.lock().unwrap();
        let entry = cache.entry(vehicle_id).or_default();
        if let Some((at, gaps)) = &entry.listed {
            if at.elapsed().as_secs() < LIST_CACHE_SECONDS {
                return Ok(gaps.clone());
            }
        }
        entry.generation
    };
    let gaps: Vec<TripGap> = candidates(pool, vehicle_id, Utc::now())
        .await?
        .into_iter()
        .map(|candidate| candidate.gap)
        .collect();
    let mut cache = LIST_CACHE.lock().unwrap();
    let entry = cache.entry(vehicle_id).or_default();
    if entry.generation == generation {
        entry.listed = Some((Instant::now(), gaps.clone()));
    }
    Ok(gaps)
}

fn invalidate_list(vehicle_id: Uuid) {
    let mut cache = LIST_CACHE.lock().unwrap();
    let entry = cache.entry(vehicle_id).or_default();
    entry.generation += 1;
    entry.listed = None;
}

/// Saves the gap as a trip and links its telemetry. Returns the new trip ID.
/// Runs to completion even when the requesting client goes away.
pub async fn recover_trip_gap(
    pool: &PgPool,
    vehicle_id: Uuid,
    gap_id: &str,
) -> Result<Uuid, RecoverError> {
    let pool = pool.clone();
    let gap_id = gap_id.to_owned();
    tokio::spawn(async move { recover(&pool, vehicle_id, &gap_id).await })
        .await
        .map_err(|error| RecoverError::Failed(error.into()))?
}

async fn recover(pool: &PgPool, vehicle_id: Uuid, gap_id: &str) -> Result<Uuid, RecoverError> {
    invalidate_list(vehicle_id);
    let candidate = candidates(pool, vehicle_id, Utc::now())
        .await?
        .into_iter()
        .find(|candidate| candidate.gap.id == gap_id)
        .ok_or(RecoverError::NotFound)?;
    let saved = crate::ingestion::worker::persist_recovered_trip(
        pool,
        &candidate.trip,
        candidate.distance,
        Duration::minutes(OVERLAP_MARGIN_MINUTES),
    )
    .await
    .context("save recovered trip")?;
    invalidate_list(vehicle_id);
    match saved {
        RecoveredSave::Saved => Ok(candidate.trip.trip_id),
        RecoveredSave::Pending => Err(RecoverError::Busy),
        RecoveredSave::Overlap | RecoveredSave::TooShort => Err(RecoverError::NotFound),
    }
}

async fn candidates(pool: &PgPool, vehicle_id: Uuid, now: DateTime<Utc>) -> Result<Vec<Candidate>> {
    let window_start = (now - Duration::days(LOOKBACK_DAYS))
        .duration_trunc(Duration::hours(1))
        .context("lookback start")?;
    let mut window_end = now - Duration::minutes(SETTLE_MINUTES);
    let active_start: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT (snapshot #>> '{detector,trip_started_at}')::timestamptz \
         FROM panda.active_trip_checkpoints \
         WHERE vehicle_id = $1 AND snapshot #>> '{detector,active_trip_id}' IS NOT NULL",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    if let Some(active_start) = active_start {
        window_end = window_end.min(active_start);
    }
    if window_end <= window_start {
        return Ok(Vec::new());
    }

    let detected: Vec<CompletedTripData> = replay_window(
        pool,
        vehicle_id,
        window_start - Duration::hours(WARMUP_HOURS),
        window_end,
    )
    .await?
    .into_iter()
    .filter(|trip| trip.started_at >= window_start)
    .collect();
    let recorded: Vec<(DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT started_at, ended_at FROM panda.trips \
         WHERE vehicle_id = $1 AND ended_at >= $2 AND started_at <= $3",
    )
    .bind(vehicle_id)
    .bind(window_start - Duration::minutes(OVERLAP_MARGIN_MINUTES))
    .bind(window_end + Duration::minutes(OVERLAP_MARGIN_MINUTES))
    .fetch_all(pool)
    .await?;
    Ok(select_gaps(vehicle_id, detected, &recorded))
}

async fn replay_window(
    pool: &PgPool,
    vehicle_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<CompletedTripData>> {
    let query = format!(
        "SELECT {REPLAY_TELEMETRY_COLUMNS} FROM timeseries.telemetry \
         WHERE vehicle_id = $1 AND ts >= $2 AND ts <= $3 ORDER BY ts"
    );
    let mut rows = sqlx::query_as::<_, ReplayTelemetryRow>(sqlx::AssertSqlSafe(query))
        .bind(vehicle_id)
        .bind(from)
        .bind(to)
        .fetch(pool);
    let mut fusion = TripSignalFusion::default();
    let mut detector = TripDetectorState::new(vehicle_id);
    let mut trips = Vec::new();
    while let Some(row) = rows.try_next().await? {
        let event = row_to_event(row);
        let sample = fusion
            .fuse_from(&event, replay_power_is_change_only(&event))
            .0;
        if let TripEvent::TripEnded { trip } = detector.process(&sample) {
            trips.push(trip);
        }
    }
    Ok(trips)
}

fn select_gaps(
    vehicle_id: Uuid,
    detected: Vec<CompletedTripData>,
    recorded: &[(DateTime<Utc>, DateTime<Utc>)],
) -> Vec<Candidate> {
    let margin = Duration::minutes(OVERLAP_MARGIN_MINUTES);
    let mut gaps: Vec<Candidate> = detected
        .into_iter()
        .filter(
            |trip| match (trip.start_odometer_mi, trip.end_odometer_mi) {
                (Some(start), Some(end)) => end - start >= MIN_ODOMETER_MILES,
                _ => false,
            },
        )
        .filter(|trip| {
            !recorded.iter().any(|(started_at, ended_at)| {
                *started_at - margin < trip.ended_at && trip.started_at < *ended_at + margin
            })
        })
        .map(|trip| {
            let distance = compute_distance_odometer_or_gps(
                trip.start_odometer_mi,
                trip.end_odometer_mi,
                &trip.points,
            );
            Candidate {
                gap: TripGap {
                    id: gap_id(vehicle_id, trip.started_at, trip.ended_at),
                    started_at: trip.started_at,
                    ended_at: trip.ended_at,
                    distance_miles: distance,
                    duration_seconds: (trip.ended_at - trip.started_at).num_seconds(),
                    soc_start: trip.soc_start,
                    soc_end: trip.soc_end,
                },
                trip,
                distance,
            }
        })
        .collect();
    gaps.sort_by(|a, b| b.gap.started_at.cmp(&a.gap.started_at));
    gaps.truncate(MAX_GAPS);
    gaps
}

/// Stable for the same detected drive, so a listed gap can be recovered later.
fn gap_id(vehicle_id: Uuid, started_at: DateTime<Utc>, ended_at: DateTime<Utc>) -> String {
    let mut hash = Sha256::new();
    hash.update(vehicle_id.as_bytes());
    hash.update(started_at.timestamp_micros().to_be_bytes());
    hash.update(ended_at.timestamp_micros().to_be_bytes());
    hex::encode(&hash.finalize()[..16])
}

/// A gap ID is 32 lowercase hexadecimal characters.
pub fn is_gap_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trip(start_min: i64, end_min: i64, odometer: (f64, f64)) -> CompletedTripData {
        let at: DateTime<Utc> = "2026-10-26T04:00:00Z".parse().unwrap();
        CompletedTripData {
            trip_id: Uuid::new_v4(),
            vehicle_id: Uuid::nil(),
            started_at: at + Duration::minutes(start_min),
            ended_at: at + Duration::minutes(end_min),
            points: Vec::new(),
            soc_start: Some(70.2),
            soc_end: Some(51.9),
            battery_capacity_wh: Some(90_000.0),
            start_odometer_mi: Some(odometer.0),
            end_odometer_mi: Some(odometer.1),
            range_start_mi: None,
            range_end_mi: None,
            power_max_kw: None,
            power_min_kw: None,
            elevation_gain_m: None,
            elevation_loss_m: None,
            inside_temp_avg_c: None,
            outside_temp_avg_c: None,
            regen_wh: None,
            dominant_drive_mode: None,
        }
    }

    #[test]
    fn keeps_untracked_drives_and_skips_recorded_or_tiny_ones() {
        let recorded_trip = trip(0, 20, (100.0, 110.0));
        let recorded = vec![(recorded_trip.started_at, recorded_trip.ended_at)];
        let detected = vec![
            // The same drive as a recorded trip, replayed with slightly
            // different boundaries.
            trip(1, 21, (100.0, 110.0)),
            // A lone late odometer step after that trip.
            trip(40, 46, (110.0, 110.62)),
            // A drive with no trip.
            trip(130, 199, (809.03, 851.9)),
        ];
        let gaps = select_gaps(Uuid::nil(), detected, &recorded);
        assert_eq!(gaps.len(), 1);
        let gap = &gaps[0].gap;
        assert!((gap.distance_miles - 42.87).abs() < 0.01);
        assert_eq!(gap.duration_seconds, 69 * 60);
        assert_eq!(gap.soc_start, Some(70.2));
        assert!(is_gap_id(&gap.id));
    }

    #[test]
    fn gap_ids_are_stable_and_distinct() {
        let at: DateTime<Utc> = "2026-10-26T04:13:21.893Z".parse().unwrap();
        let end = at + Duration::minutes(82);
        let vehicle = Uuid::new_v4();
        assert_eq!(gap_id(vehicle, at, end), gap_id(vehicle, at, end));
        assert_ne!(gap_id(vehicle, at, end), gap_id(Uuid::new_v4(), at, end));
        assert_ne!(
            gap_id(vehicle, at, end),
            gap_id(vehicle, at, end + Duration::seconds(1))
        );
    }

    #[test]
    fn gap_id_validation_rejects_other_text() {
        assert!(is_gap_id("0123456789abcdef0123456789abcdef"));
        for value in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "../etc",
            "0123456789abcdef0123456789abcde",
        ] {
            assert!(!is_gap_id(value), "{value}");
        }
    }

    /// Run against a disposable restored copy only: it saves recovered trips.
    #[tokio::test]
    #[ignore = "requires TRIP_GAPS_TEST_DATABASE_URL for a disposable restored copy"]
    async fn restored_copy_lists_and_recovers_missed_drives() {
        let url = std::env::var("TRIP_GAPS_TEST_DATABASE_URL").expect("database URL");
        let vehicle: Uuid = std::env::var("TRIP_GAPS_TEST_VEHICLE_ID")
            .expect("vehicle id")
            .parse()
            .unwrap();
        let pool = PgPool::connect(&url).await.unwrap();
        let gaps = find_trip_gaps(&pool, vehicle).await.unwrap();
        for gap in &gaps {
            eprintln!(
                "GAP {} {} -> {} {:.2} mi soc {:?} -> {:?}",
                gap.id,
                gap.started_at,
                gap.ended_at,
                gap.distance_miles,
                gap.soc_start,
                gap.soc_end
            );
        }
        assert!(
            !gaps.is_empty(),
            "expected missed drives in the restored copy"
        );
        for gap in &gaps {
            let trip_id = recover_trip_gap(&pool, vehicle, &gap.id).await.unwrap();
            let (distance, linked): (f64, i64) = sqlx::query_as(
                "SELECT t.distance_miles, (SELECT count(*) FROM timeseries.telemetry m WHERE m.trip_id = t.id) \
                 FROM panda.trips t WHERE t.id = $1",
            )
            .bind(trip_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            eprintln!(
                "RECOVERED {} {trip_id} {distance:.2} mi linked {linked}",
                gap.id
            );
            assert!((distance - gap.distance_miles).abs() < 1e-9);
            assert!(linked > 0);
            assert!(matches!(
                recover_trip_gap(&pool, vehicle, &gap.id).await,
                Err(RecoverError::NotFound)
            ));
        }
        assert!(find_trip_gaps(&pool, vehicle).await.unwrap().is_empty());
    }

    #[test]
    fn newest_gaps_come_first() {
        let detected = vec![trip(0, 30, (1.0, 5.0)), trip(100, 130, (5.0, 9.0))];
        let gaps = select_gaps(Uuid::nil(), detected, &[]);
        assert!(gaps[0].gap.started_at > gaps[1].gap.started_at);
    }
}

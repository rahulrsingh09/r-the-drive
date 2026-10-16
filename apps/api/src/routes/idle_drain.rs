//! Idle (phantom) drain endpoint built from validated parked sessions instead
//! of raw gaps between anchors.
//!
//! Canonical parked-session boundaries intentionally follow TeslaMate's model:
//! completed trips and completed charge sessions define the lifecycle anchors.
//! Vehicle state periods and raw telemetry act only as overlays and validators
//! so noisy state rows cannot collapse an otherwise valid parked-gap list.

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{require_vehicle_access, AppState, AuthUser},
};

mod evidence;
mod workflow;

const DEFAULT_MIN_DURATION_HOURS: f64 = 6.0;
const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: usize = 500;
const CHART_MIN_DURATION_HOURS: f64 = 0.25;
const ACTIVE_PADDING_HOURS: i64 = 48;
const MAX_SOC_BOUNDARY_GAP_MINUTES: f64 = 45.0;
// Very long parked windows can go days without telemetry near the exact edge,
// so the SoC/range anchor search widens before we fall back to exclusion.
const LONG_SOC_BOUNDARY_GAP_MINUTES: f64 = 12.0 * 60.0;
const VERY_LONG_SOC_BOUNDARY_GAP_MINUTES: f64 = 72.0 * 60.0;
const MAX_ODOMETER_DELTA_MILES: f64 = 2.0;
const LONG_SESSION_HOURS: f64 = 18.0;
const VERY_LONG_SESSION_HOURS: f64 = 48.0;
const LARGE_SOC_LOSS_THRESHOLD_PCT: f64 = 8.0;
const MIN_SLEEP_COVERAGE_PCT: f64 = 0.5;
const SOC_WINDOW_MISMATCH_TOLERANCE_PCT: f64 = 3.0;
const RANGE_EFFICIENCY_DRIFT_TOLERANCE: f64 = 0.35;

pub fn router() -> Router<AppState> {
    Router::new().route("/vehicles/{vehicle_id}/idle-drain", get(idle_drain))
}

#[derive(Deserialize)]
struct IdleDrainParams {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: Option<bool>,
    #[serde(default = "default_min_duration_hours")]
    min_duration_hours: f64,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    include_excluded: bool,
}

fn default_limit() -> i64 {
    DEFAULT_LIMIT
}

fn default_min_duration_hours() -> f64 {
    DEFAULT_MIN_DURATION_HOURS
}

fn resolve_time_bounds(
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: bool,
    default_days: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let resolved_to = to.unwrap_or_else(Utc::now);
    let resolved_from = if lifetime {
        DateTime::<Utc>::from_timestamp(0, 0).unwrap_or(resolved_to - chrono::Duration::days(3650))
    } else {
        from.unwrap_or_else(|| Utc::now() - chrono::Duration::days(default_days))
    };
    (resolved_from, resolved_to)
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Validated,
    Excluded,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationReason {
    OverlapsCharge,
    OverlapsTrip,
    MovementDetected,
    NetGainDetected,
    InsufficientTelemetry,
    SocAnchorUnreliable,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhantomPeriod {
    pub period_start: Option<DateTime<Utc>>,
    pub period_end: Option<DateTime<Utc>>,
    pub duration_hours: Option<f64>,
    pub sleep_share_pct: Option<f64>,
    pub state_coverage_pct: Option<f64>,
    pub soc_start: Option<f64>,
    pub soc_end: Option<f64>,
    pub soc_lost_pct: Option<f64>,
    pub drain_pct_per_hour: Option<f64>,
    pub range_start_mi: Option<f64>,
    pub range_end_mi: Option<f64>,
    pub range_lost_mi: Option<f64>,
    pub range_lost_per_hour_mi: Option<f64>,
    pub energy_drained_kwh: Option<f64>,
    pub avg_power_w: Option<f64>,
    pub has_reduced_range: Option<bool>,
    pub validation_status: ValidationStatus,
    pub validation_reason: Option<ValidationReason>,
    pub sample_count: i64,
    pub start_sample_at: Option<DateTime<Utc>>,
    pub end_sample_at: Option<DateTime<Utc>>,
    pub movement_detected: bool,
    pub overlaps_trip: bool,
    pub overlaps_charge: bool,
}

#[derive(Serialize)]
struct IdleDrainResponse {
    vehicle_id: Uuid,
    periods: Vec<PhantomPeriod>,
}

#[derive(Debug, sqlx::FromRow)]
struct PhantomCandidateRow {
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    duration_hours: f64,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    start_sample_at: Option<DateTime<Utc>>,
    end_sample_at: Option<DateTime<Utc>>,
    start_sample_gap_minutes: Option<f64>,
    end_sample_gap_minutes: Option<f64>,
    range_start_mi: Option<f64>,
    range_end_mi: Option<f64>,
    start_range_gap_minutes: Option<f64>,
    end_range_gap_minutes: Option<f64>,
    capacity_wh: Option<f64>,
    sample_count: i64,
    battery_sample_count: i64,
    interior_battery_sample_count: i64,
    min_soc_in_window: Option<f64>,
    max_soc_in_window: Option<f64>,
    odometer_delta_mi: Option<f64>,
    drive_sample_count: i64,
    trip_overlap_count: i64,
    charge_overlap_count: i64,
    sleep_seconds: Option<f64>,
    covered_state_seconds: Option<f64>,
    has_reduced_range: Option<bool>,
}

async fn idle_drain(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
    Query(params): Query<IdleDrainParams>,
) -> Result<Json<IdleDrainResponse>, AppError> {
    require_vehicle_access(&auth, vehicle_id)?;
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let (from, to) =
        resolve_time_bounds(params.from, params.to, params.lifetime.unwrap_or(false), 30);
    let min_duration_hours = params.min_duration_hours.max(0.0);
    let limit = params.limit.clamp(1, MAX_LIMIT as i64) as usize;

    let periods = fetch_idle_drain_periods(
        &state.pool,
        vehicle_id,
        from,
        to,
        min_duration_hours,
        limit,
        params.include_excluded,
    )
    .await?;

    Ok(Json(IdleDrainResponse {
        vehicle_id,
        periods,
    }))
}

pub(crate) async fn fetch_idle_drain_periods(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    min_duration_hours: f64,
    limit: usize,
    include_excluded: bool,
) -> Result<Vec<PhantomPeriod>, AppError> {
    workflow::ParkedHistory {
        vehicle: vehicle_id,
        from,
        to,
        minimum_hours: min_duration_hours,
        limit,
        include_excluded,
    }
    .execute(pool)
    .await
}

pub(crate) async fn fetch_validated_idle_drain_periods_for_chart(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<PhantomPeriod>, AppError> {
    fetch_idle_drain_periods(
        pool,
        vehicle_id,
        from,
        to,
        CHART_MIN_DURATION_HOURS,
        MAX_LIMIT,
        false,
    )
    .await
}

fn derive_phantom_period(row: PhantomCandidateRow) -> PhantomPeriod {
    let duration_hours = row.duration_hours.max(0.0);
    let duration_seconds = duration_hours * 3600.0;
    let overlaps_trip = row.trip_overlap_count > 0;
    let overlaps_charge = row.charge_overlap_count > 0;
    // Tiny odometer drift is common in parked windows; only larger mileage
    // changes or explicit trip/charge overlaps should mark movement.
    let movement_detected = overlaps_trip
        || overlaps_charge
        || row.odometer_delta_mi.unwrap_or(0.0) >= MAX_ODOMETER_DELTA_MILES
        || row.drive_sample_count >= 10;

    let soc_start = finite_optional(row.soc_start);
    let soc_end = finite_optional(row.soc_end);
    let soc_gain_detected = match (soc_start, soc_end) {
        (Some(start), Some(end)) => start < end,
        _ => false,
    };
    let soc_lost_pct = match (soc_start, soc_end) {
        (Some(start), Some(end)) if start >= end => Some(start - end),
        _ => None,
    };
    let drain_pct_per_hour = match (soc_lost_pct, duration_hours > 0.0) {
        (Some(loss), true) => Some(loss / duration_hours),
        _ => None,
    };

    let sleep_seconds = row
        .sleep_seconds
        .unwrap_or(0.0)
        .clamp(0.0, duration_seconds);
    let covered_state_seconds = row
        .covered_state_seconds
        .unwrap_or(0.0)
        .clamp(0.0, duration_seconds);
    let state_coverage_pct = if duration_seconds > 0.0 {
        Some((covered_state_seconds / duration_seconds).clamp(0.0, 1.0))
    } else {
        None
    };
    let sleep_share_pct = match state_coverage_pct {
        Some(coverage) if coverage >= MIN_SLEEP_COVERAGE_PCT && duration_seconds > 0.0 => {
            Some((sleep_seconds / duration_seconds).clamp(0.0, 1.0))
        }
        _ => None,
    };

    let validation_reason = classify_validation_reason(
        &row,
        duration_hours,
        soc_lost_pct,
        movement_detected,
        soc_gain_detected,
    );
    let validation_status = if validation_reason.is_some() {
        ValidationStatus::Excluded
    } else {
        ValidationStatus::Validated
    };

    let energy_drained_kwh = match (soc_lost_pct, finite_optional(row.capacity_wh)) {
        (Some(loss), Some(capacity_wh)) if capacity_wh > 10000.0 => {
            Some((loss / 100.0) * (capacity_wh / 1000.0))
        }
        _ => None,
    };
    let avg_power_w = match (energy_drained_kwh, duration_hours > 0.0) {
        (Some(kwh), true) => Some((kwh / duration_hours) * 1000.0),
        _ => None,
    };

    let range_lost_mi = compute_range_loss(&row, soc_start, soc_end, soc_lost_pct);
    let range_lost_per_hour_mi = match (range_lost_mi, duration_hours > 0.0) {
        (Some(loss), true) => Some(loss / duration_hours),
        _ => None,
    };

    PhantomPeriod {
        period_start: Some(row.period_start),
        period_end: Some(row.period_end),
        duration_hours: Some(duration_hours),
        sleep_share_pct,
        state_coverage_pct,
        soc_start,
        soc_end,
        soc_lost_pct,
        drain_pct_per_hour,
        range_start_mi: finite_optional(row.range_start_mi),
        range_end_mi: finite_optional(row.range_end_mi),
        range_lost_mi,
        range_lost_per_hour_mi,
        energy_drained_kwh,
        avg_power_w,
        has_reduced_range: row.has_reduced_range,
        validation_status,
        validation_reason,
        sample_count: row.sample_count,
        start_sample_at: row.start_sample_at,
        end_sample_at: row.end_sample_at,
        movement_detected,
        overlaps_trip,
        overlaps_charge,
    }
}

fn classify_validation_reason(
    row: &PhantomCandidateRow,
    duration_hours: f64,
    soc_lost_pct: Option<f64>,
    movement_detected: bool,
    soc_gain_detected: bool,
) -> Option<ValidationReason> {
    if row.charge_overlap_count > 0 {
        return Some(ValidationReason::OverlapsCharge);
    }
    if row.trip_overlap_count > 0 {
        return Some(ValidationReason::OverlapsTrip);
    }
    if soc_gain_detected {
        return Some(ValidationReason::NetGainDetected);
    }
    if movement_detected {
        return Some(ValidationReason::MovementDetected);
    }

    if !boundary_gaps_are_valid(
        duration_hours,
        row.start_sample_gap_minutes,
        row.end_sample_gap_minutes,
    ) {
        return Some(ValidationReason::SocAnchorUnreliable);
    }

    let required_samples = required_battery_samples(duration_hours, soc_lost_pct);
    if row.battery_sample_count < required_samples {
        return Some(ValidationReason::InsufficientTelemetry);
    }

    if requires_interior_soc_support(duration_hours, soc_lost_pct)
        && row.interior_battery_sample_count < 1
    {
        return Some(ValidationReason::InsufficientTelemetry);
    }

    if let (Some(loss), Some(min_soc), Some(max_soc)) = (
        soc_lost_pct,
        finite_optional(row.min_soc_in_window),
        finite_optional(row.max_soc_in_window),
    ) {
        let observed_window_span = (max_soc - min_soc).max(0.0);
        if row.battery_sample_count >= 3
            && loss - observed_window_span > SOC_WINDOW_MISMATCH_TOLERANCE_PCT
        {
            return Some(ValidationReason::SocAnchorUnreliable);
        }
    }

    None
}

fn required_battery_samples(duration_hours: f64, soc_lost_pct: Option<f64>) -> i64 {
    if soc_lost_pct.unwrap_or(0.0) >= LARGE_SOC_LOSS_THRESHOLD_PCT
        || duration_hours >= VERY_LONG_SESSION_HOURS
    {
        5
    } else if duration_hours >= LONG_SESSION_HOURS {
        3
    } else {
        2
    }
}

fn requires_interior_soc_support(duration_hours: f64, soc_lost_pct: Option<f64>) -> bool {
    duration_hours >= LONG_SESSION_HOURS
        || soc_lost_pct.unwrap_or(0.0) >= LARGE_SOC_LOSS_THRESHOLD_PCT
}

fn boundary_gaps_are_valid(
    duration_hours: f64,
    start_gap: Option<f64>,
    end_gap: Option<f64>,
) -> bool {
    let max_gap_minutes = max_boundary_gap_minutes(duration_hours);
    match (finite_optional(start_gap), finite_optional(end_gap)) {
        (Some(start), Some(end)) => start <= max_gap_minutes && end <= max_gap_minutes,
        _ => false,
    }
}

fn max_boundary_gap_minutes(duration_hours: f64) -> f64 {
    if duration_hours >= VERY_LONG_SESSION_HOURS {
        VERY_LONG_SOC_BOUNDARY_GAP_MINUTES
    } else if duration_hours >= LONG_SESSION_HOURS {
        LONG_SOC_BOUNDARY_GAP_MINUTES
    } else {
        MAX_SOC_BOUNDARY_GAP_MINUTES
    }
}

fn compute_range_loss(
    row: &PhantomCandidateRow,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    soc_lost_pct: Option<f64>,
) -> Option<f64> {
    if !boundary_gaps_are_valid(
        row.duration_hours,
        row.start_range_gap_minutes,
        row.end_range_gap_minutes,
    ) {
        return None;
    }

    if row.sample_count < 3 {
        return None;
    }

    let start_range = finite_optional(row.range_start_mi)?;
    let end_range = finite_optional(row.range_end_mi)?;
    let start_soc = soc_start.filter(|value| *value > 0.0)?;
    let end_soc = soc_end.filter(|value| *value > 0.0)?;
    let soc_loss = soc_lost_pct?;

    let range_loss = (start_range - end_range).max(0.0);
    let start_eff = (start_range / start_soc) * 100.0;
    let end_eff = (end_range / end_soc) * 100.0;
    let avg_eff = (start_eff + end_eff) / 2.0;
    if avg_eff <= 0.0 {
        return None;
    }

    let efficiency_drift = ((start_eff - end_eff).abs()) / avg_eff;
    if efficiency_drift > RANGE_EFFICIENCY_DRIFT_TOLERANCE {
        return None;
    }

    let expected_loss = avg_eff * (soc_loss / 100.0);
    if range_loss > expected_loss.max(5.0) * 3.0 {
        return None;
    }

    Some(range_loss)
}

fn finite_optional(value: Option<f64>) -> Option<f64> {
    value.filter(|candidate| candidate.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::{Request, StatusCode};
    use tower::ServiceExt;

    fn candidate_row() -> PhantomCandidateRow {
        PhantomCandidateRow {
            period_start: DateTime::parse_from_rfc3339("2026-05-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            period_end: DateTime::parse_from_rfc3339("2026-05-01T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            duration_hours: 12.0,
            soc_start: Some(70.0),
            soc_end: Some(66.0),
            start_sample_at: Some(
                DateTime::parse_from_rfc3339("2026-05-01T00:05:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            end_sample_at: Some(
                DateTime::parse_from_rfc3339("2026-05-01T11:56:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            start_sample_gap_minutes: Some(5.0),
            end_sample_gap_minutes: Some(4.0),
            range_start_mi: Some(250.0),
            range_end_mi: Some(236.0),
            start_range_gap_minutes: Some(5.0),
            end_range_gap_minutes: Some(4.0),
            capacity_wh: Some(131000.0),
            sample_count: 8,
            battery_sample_count: 6,
            interior_battery_sample_count: 4,
            min_soc_in_window: Some(66.0),
            max_soc_in_window: Some(70.0),
            odometer_delta_mi: Some(0.0),
            drive_sample_count: 0,
            trip_overlap_count: 0,
            charge_overlap_count: 0,
            sleep_seconds: Some(41000.0),
            covered_state_seconds: Some(43200.0),
            has_reduced_range: Some(false),
        }
    }

    async fn make_app() -> axum::Router {
        use crate::middleware::auth::{AppState, JwtKeys};
        use std::sync::Arc;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());

        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let redis = redis::Client::open(redis_url).expect("redis client");

        let keys = crate::keys::generate_keys().expect("generate test keys");
        let jwt_keys =
            Arc::new(JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).expect("jwt keys"));

        let config = crate::config::Config {
            database_url: database_url.clone(),
            redis_url: "redis://127.0.0.1/".into(),
            jwt_secret: None,
            jwt_public_key: None,
            age_encryption_key: None,
            port: 3001,
            allowed_origins: vec!["http://localhost:3000".into()],
            s3_endpoint: None,
            s3_access_key: None,
            s3_secret_key: None,
            backup_artifact_dir: std::env::temp_dir()
                .join("panda-route-test-backups")
                .to_string_lossy()
                .into_owned(),
            vehicle_image_cache_dir: std::env::temp_dir()
                .join("panda-route-test-vehicle-images")
                .to_string_lossy()
                .into_owned(),
            backup_driver: "pg_dump".into(),
            backup_poll_interval_seconds: 60,
            restore_agent_url: "http://127.0.0.1:3002".into(),
            restore_agent_key_file: "/backups/.restore-agent-key".into(),
            recovery: crate::config::RecoveryConfig::default(),
            origin_bind: crate::config::OriginBindConfig::default(),
            security: Default::default(),
            rivian_ws_reconnect_initial_seconds: 10,
            rivian_ws_reconnect_max_seconds: 900,
            rivian_raw_event_retention_days: 7,
            rivian_persist_raw_events: true,
            rivian_suppress_duplicate_telemetry: true,
            panda_env: None,
            cookie_insecure: None,
            allow_insecure_lan_http_auth: false,
            rate_limit: crate::config::RateLimitConfig::default(),
        };

        let state = AppState {
            pool,
            redis,
            jwt_keys,
            age_key: "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
                .to_string(),
            config,
            nominatim_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            resources: Default::default(),
        };

        crate::routes::build_router(state)
    }

    async fn get_status(app: axum::Router, uri: &str) -> http::StatusCode {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn idle_drain_requires_auth() {
        let app = make_app().await;
        assert_eq!(
            get_status(
                app,
                "/v1/vehicles/00000000-0000-0000-0000-000000000000/idle-drain",
            )
            .await,
            StatusCode::UNAUTHORIZED,
        );
    }

    #[test]
    fn min_duration_default_is_six_hours() {
        assert_eq!(super::default_min_duration_hours(), 6.0);
    }

    #[test]
    fn validated_period_exposes_sleep_share_and_energy() {
        let period = derive_phantom_period(candidate_row());
        assert_eq!(period.validation_status, ValidationStatus::Validated);
        assert!(period.sleep_share_pct.unwrap() > 0.9);
        assert_eq!(period.validation_reason, None);
        assert!(period.energy_drained_kwh.unwrap() > 5.0);
    }

    #[test]
    fn charge_overlap_is_excluded() {
        let mut row = candidate_row();
        row.charge_overlap_count = 1;
        let period = derive_phantom_period(row);
        assert_eq!(period.validation_status, ValidationStatus::Excluded);
        assert_eq!(
            period.validation_reason,
            Some(ValidationReason::OverlapsCharge)
        );
    }

    #[test]
    fn noisy_drive_samples_do_not_auto_exclude_period() {
        let mut row = candidate_row();
        row.drive_sample_count = 4;
        let period = derive_phantom_period(row);
        assert_eq!(period.validation_status, ValidationStatus::Validated);
        assert_eq!(period.validation_reason, None);
    }

    #[test]
    fn net_gain_is_excluded() {
        let mut row = candidate_row();
        row.soc_end = Some(72.0);
        let period = derive_phantom_period(row);
        assert_eq!(period.validation_status, ValidationStatus::Excluded);
        assert_eq!(
            period.validation_reason,
            Some(ValidationReason::NetGainDetected)
        );
    }

    #[test]
    fn sparse_large_drop_is_excluded() {
        let mut row = candidate_row();
        row.duration_hours = 36.0;
        row.soc_end = Some(54.0);
        row.sample_count = 2;
        row.battery_sample_count = 2;
        row.interior_battery_sample_count = 0;
        row.min_soc_in_window = Some(69.5);
        row.max_soc_in_window = Some(70.0);
        let period = derive_phantom_period(row);
        assert_eq!(period.validation_status, ValidationStatus::Excluded);
        assert_eq!(
            period.validation_reason,
            Some(ValidationReason::InsufficientTelemetry)
        );
    }

    #[test]
    fn low_state_coverage_hides_sleep_share_without_excluding_period() {
        let mut row = candidate_row();
        row.covered_state_seconds = Some(6000.0);
        row.sleep_seconds = Some(6000.0);
        let period = derive_phantom_period(row);
        assert_eq!(period.validation_status, ValidationStatus::Validated);
        assert_eq!(period.sleep_share_pct, None);
        assert!(period.state_coverage_pct.unwrap() < 0.5);
    }
}

#[cfg(test)]
#[path = "__tests__/energy_idle_boundaries.rs"]
mod energy_idle_boundaries;

#[cfg(test)]
#[path = "__tests__/idle_projection_contract.rs"]
mod idle_projection_contract;

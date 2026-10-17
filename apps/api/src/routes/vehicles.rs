use crate::private_deployment::enrollment::{
    lock_active_enrollment_user, lock_credential_membership, lock_enrollment,
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderName, HeaderValue},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use std::{
    collections::BTreeMap,
    path::{Path as StdPath, PathBuf},
    time::Duration,
};

use crate::{
    db::users::require_admin_or_super_user,
    db::vehicles::{
        require_vehicle_read_access, require_vehicle_role, require_vehicle_session_access,
    },
    errors::AppError,
    ingestion::{
        rivian_auth::{RivianAuthError, RivianVehicleSummary},
        supervisor::SupervisorCommand,
    },
    middleware::auth::{require_vehicle_access, AppState, AuthUser},
    routes::range_normalization::normalize_remaining_range_miles,
    services::{
        demo_seed::{lock_demo_creator, seed_demo_vehicle, seed_new_demo_vehicle},
        external_connections::{
            rivian_renewal_state_for_auth, RivianRenewalState, RIVIAN_RENEWAL_INTERVAL,
        },
        ingestion_capture,
    },
};

#[path = "vehicles/diagnostics/lanes.rs"]
mod lane_diagnostics;
#[path = "vehicles/diagnostics/raw.rs"]
mod raw_diagnostics;

#[path = "vehicles/artwork.rs"]
mod artwork;
#[path = "vehicles/connection.rs"]
mod connection;
#[path = "vehicles/enrollment.rs"]
mod enrollment;
#[path = "vehicles/membership.rs"]
mod membership;
#[path = "vehicles/status.rs"]
mod status;
use status::StatusFieldAvailability;
#[cfg(test)]
use status::{
    classify_status_field_availability, derive_vehicle_status_freshness, max_seen_at,
    StatusFieldAvailabilityReasonCode, StatusFieldAvailabilityState,
};

#[cfg(test)]
#[path = "vehicles/__tests__/diagnostics.rs"]
mod diagnostic_contracts;

#[cfg(test)]
#[path = "vehicles/__tests__/contracts.rs"]
mod workflow_contracts;

#[cfg(test)]
#[path = "vehicles_authorization_tests.rs"]
mod authorization_tests;

#[cfg(test)]
#[path = "vehicles_demo_history_tests.rs"]
mod demo_history_tests;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vehicles/connect", post(connect))
        .route("/vehicles/connect/otp", post(connect_otp))
        .route("/vehicles/demo", post(create_demo_vehicle))
        .route("/vehicles/{id}/demo/refresh", post(refresh_demo_vehicle))
        .route(
            "/vehicle-image-cache/{id}/{image_key}",
            get(vehicle_image_cache_asset),
        )
        .route(
            "/admin/vehicles/{id}/images/remirror",
            post(admin_remirror_vehicle_images),
        )
        .route(
            "/admin/vehicles/{id}/images/cache/purge",
            post(admin_purge_vehicle_artwork_cache),
        )
        .route("/vehicles", post(add_vehicle).get(list_vehicles))
        .route("/vehicles/{id}", delete(delete_vehicle))
        .route("/vehicles/{id}/default", post(set_default_vehicle))
        .route(
            "/vehicles/{id}/members",
            get(list_vehicle_members).post(add_vehicle_member),
        )
        .route(
            "/vehicles/{id}/members/{user_id}",
            put(update_vehicle_member).delete(remove_vehicle_member),
        )
        .route(
            "/vehicles/{id}/invites",
            get(list_vehicle_invites).post(create_vehicle_invite),
        )
        .route(
            "/vehicles/{id}/invites/{invite_id}",
            delete(revoke_vehicle_invite),
        )
        .route("/invites/{token}", get(preview_invite))
        .route("/invites/{token}/accept", post(accept_invite))
        .route(
            "/vehicles/{id}/credentials",
            put(refresh_vehicle_credentials),
        )
        .route("/vehicles/{id}/status", get(vehicle_status))
        .route("/vehicles/{id}/images", get(vehicle_images))
        .route("/vehicles/{id}/raw-data", get(raw_vehicle_data))
        .route("/vehicles/{id}/telemetry/lanes", get(telemetry_lanes))
        .route("/vehicles/{id}/raw-events", get(raw_vehicle_events))
        .route(
            "/vehicles/{id}/ingestion-diagnostics",
            get(get_ingestion_diagnostics),
        )
        .route(
            "/vehicles/{id}/ingestion-diagnostics/start",
            post(start_ingestion_capture),
        )
        .route(
            "/vehicles/{id}/ingestion-diagnostics/stop",
            post(stop_ingestion_capture),
        )
        .route(
            "/vehicles/{id}/ingestion-diagnostics/export",
            get(export_ingestion_capture),
        )
        .route(
            "/vehicles/{id}/raw-events/{event_id}",
            get(raw_vehicle_event),
        )
        .route("/vehicles/{id}/settings", put(update_vehicle_settings))
        .route("/vehicles/{id}/battery-config", put(update_battery_config))
        .route("/vehicles/{id}/name", put(update_vehicle_name))
}

#[derive(Deserialize)]
struct RawDataParams {
    limit: Option<i64>,
    per_page: Option<i64>,
    page: Option<i64>,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
    search: Option<String>,
    fields: Option<String>,
    populated_only: Option<bool>,
}

#[derive(Deserialize)]
struct TelemetryLaneParams {
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
    lanes: Option<String>,
    resolution: Option<String>,
    max_points: Option<i64>,
}

#[derive(Deserialize)]
struct RawEventParams {
    per_page: Option<i64>,
    page: Option<i64>,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
    event_type: Option<String>,
    message_type: Option<String>,
}

async fn get_ingestion_diagnostics(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<ingestion_capture::CaptureStatus>, AppError> {
    CaptureAction::Inspect
        .run(&state, &auth, vehicle_id)
        .await
        .map(Json)
}

async fn start_ingestion_capture(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<ingestion_capture::CaptureStatus>, AppError> {
    CaptureAction::Start
        .run(&state, &auth, vehicle_id)
        .await
        .map(Json)
}

async fn stop_ingestion_capture(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<ingestion_capture::CaptureStatus>, AppError> {
    CaptureAction::Stop
        .run(&state, &auth, vehicle_id)
        .await
        .map(Json)
}

const CAPTURE_EXPORT_PAGE: i64 = 1000;

/// Stream the vehicle's latest capture as NDJSON: one header line, then one
/// line per event. Neither carries the vehicle id, VIN, or name.
async fn export_ingestion_capture(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Response<Body>, AppError> {
    require_vehicle_access(&auth, vehicle_id)?;
    require_raw_event_access(&state, &auth, vehicle_id).await?;
    let export = ingestion_capture::export(&state.pool, vehicle_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let pool = state.pool.clone();
    let capture_id = export.capture_id;
    let records = futures::stream::try_unfold(0_i64, move |cursor| {
        let pool = pool.clone();
        async move {
            let records =
                ingestion_capture::export_rows(&pool, capture_id, cursor, CAPTURE_EXPORT_PAGE)
                    .await
                    .map_err(std::io::Error::other)?;
            let Some((last_id, _)) = records.last() else {
                return Ok::<_, std::io::Error>(None);
            };
            let next_cursor = *last_id;
            let chunk: String = records
                .into_iter()
                .map(|(_, value)| format!("{value}\n"))
                .collect();
            Ok(Some((axum::body::Bytes::from(chunk), next_cursor)))
        }
    });
    let heading = futures::stream::once(async move {
        Ok::<_, std::io::Error>(axum::body::Bytes::from(format!("{}\n", export.header)))
    });
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        export.filename.replace('"', "_")
    ))
    .map_err(|error| AppError::Internal(anyhow::anyhow!("invalid download filename: {error}")))?;
    let mut response = Response::new(Body::from_stream(futures::StreamExt::chain(
        heading, records,
    )));
    response.headers_mut().extend([
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-ndjson"),
        ),
        (header::CONTENT_DISPOSITION, disposition),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
    ]);
    Ok(response)
}

#[derive(Deserialize)]
struct ConnectBody {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct OtpBody {
    challenge_id: String,
    #[serde(alias = "otp")]
    otp_code: String,
}

#[derive(Deserialize)]
struct AddVehicleBody {
    rivian_vehicle_id: String,
    name: Option<String>,
    home_lat: Option<f64>,
    home_lng: Option<f64>,
    trim: Option<String>,
}

#[derive(Deserialize)]
struct CreateDemoVehicleBody {
    model: String,
}

fn is_demo_vehicle_key(value: &str) -> bool {
    value.starts_with("demo-")
}

fn canonical_vehicle_model(model: &str) -> &str {
    let model = model.trim();
    if model.eq_ignore_ascii_case("R2")
        || model.eq_ignore_ascii_case("R2S")
        || model.eq_ignore_ascii_case("R2-S")
    {
        "R2"
    } else {
        model
    }
}

fn canonical_battery_config(config: &str) -> &str {
    let config = config.trim();
    if config.eq_ignore_ascii_case("r2s") {
        "r2"
    } else {
        config
    }
}

fn canonical_vehicle_name(name: &str) -> &str {
    let name = name.trim();
    if name.eq_ignore_ascii_case("R2S") || name.eq_ignore_ascii_case("R2-S") {
        "R2"
    } else {
        name
    }
}

async fn require_remote_backed_vehicle(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
) -> Result<String, AppError> {
    let key =
        sqlx::query_scalar::<_, String>("SELECT rivian_vehicle_id FROM panda.vehicles WHERE id=$1")
            .bind(vehicle_id)
            .fetch_optional(pool)
            .await?
            .ok_or(AppError::NotFound)?;
    if is_demo_vehicle_key(&key) {
        return Err(AppError::Validation(
            "demo vehicles do not use Rivian credentials or remote artwork".into(),
        ));
    }
    Ok(key)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOtpChallenge {
    email: String,
    otp_token: String,
    csrf_token: String,
    app_session_token: String,
    user_id: Uuid,
}

#[derive(Debug, Default, sqlx::FromRow)]
struct LatestVehicleTelemetry {
    ts: Option<chrono::DateTime<chrono::Utc>>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude_m: Option<f64>,
    speed_mph: Option<f64>,
    location_ts: Option<chrono::DateTime<chrono::Utc>>,
    speed_mph_ts: Option<chrono::DateTime<chrono::Utc>>,
    battery_level: Option<f64>,
    battery_capacity_wh: Option<f64>,
    distance_to_empty_mi: Option<f64>,
    battery_limit: Option<f64>,
    battery_level_ts: Option<chrono::DateTime<chrono::Utc>>,
    distance_to_empty_mi_ts: Option<chrono::DateTime<chrono::Utc>>,
    battery_limit_ts: Option<chrono::DateTime<chrono::Utc>>,
    power_state: Option<String>,
    power_state_ts: Option<chrono::DateTime<chrono::Utc>>,
    charger_state: Option<String>,
    charger_state_ts: Option<chrono::DateTime<chrono::Utc>>,
    charger_status: Option<String>,
    charger_status_ts: Option<chrono::DateTime<chrono::Utc>>,
    time_to_end_of_charge_min: Option<i32>,
    time_to_end_of_charge_min_ts: Option<chrono::DateTime<chrono::Utc>>,
    drive_mode: Option<String>,
    gear_status: Option<String>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    heading_deg: Option<f64>,
    odometer_miles: Option<f64>,
    odometer_miles_ts: Option<chrono::DateTime<chrono::Utc>>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
    tire_fl_status: Option<String>,
    tire_fr_status: Option<String>,
    tire_rl_status: Option<String>,
    tire_rr_status: Option<String>,
    tire_fl_valid: Option<bool>,
    tire_fr_valid: Option<bool>,
    tire_rl_valid: Option<bool>,
    tire_rr_valid: Option<bool>,
    door_front_left_locked: Option<bool>,
    door_front_right_locked: Option<bool>,
    door_rear_left_locked: Option<bool>,
    door_rear_right_locked: Option<bool>,
    door_front_left_closed: Option<bool>,
    door_front_right_closed: Option<bool>,
    door_rear_left_closed: Option<bool>,
    door_rear_right_closed: Option<bool>,
    closure_frunk_locked: Option<bool>,
    closure_frunk_closed: Option<bool>,
    closure_liftgate_locked: Option<bool>,
    closure_liftgate_closed: Option<bool>,
    closure_tailgate_locked: Option<bool>,
    closure_tailgate_closed: Option<bool>,
    ota_current_version: Option<String>,
    ota_available_version: Option<String>,
    ota_status: Option<String>,
    ota_current_status: Option<String>,
    hv_thermal_event: Option<String>,
    twelve_volt_health: Option<String>,
    // Extended vehicleStatus fields (migration 0022)
    charge_port_open: Option<bool>,
    charger_derate_active: Option<bool>,
    cabin_precon_status: Option<String>,
    cabin_precon_type: Option<String>,
    pet_mode_active: Option<bool>,
    pet_mode_temp_ok: Option<bool>,
    defrost_active: Option<bool>,
    steering_wheel_heat: Option<i16>,
    seat_fl_heat: Option<i16>,
    seat_fr_heat: Option<i16>,
    seat_rl_heat: Option<i16>,
    seat_rr_heat: Option<i16>,
    seat_fl_vent: Option<i16>,
    seat_fr_vent: Option<i16>,
    tonneau_locked: Option<bool>,
    tonneau_closed: Option<bool>,
    side_bin_left_locked: Option<bool>,
    side_bin_right_locked: Option<bool>,
    side_bin_left_closed: Option<bool>,
    side_bin_right_closed: Option<bool>,
    window_fl_closed: Option<bool>,
    window_fr_closed: Option<bool>,
    window_rl_closed: Option<bool>,
    window_rr_closed: Option<bool>,
    gear_guard_locked: Option<bool>,
    gear_guard_video_status: Option<String>,
    wiper_fluid_low: Option<bool>,
    brake_fluid_low: Option<bool>,
    alarm_active: Option<bool>,
    service_mode: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct VehicleListRow {
    id: Uuid,
    rivian_vehicle_id: String,
    model: String,
    trim: Option<String>,
    vin: Option<String>,
    color: Option<String>,
    name: Option<String>,
    battery_capacity_wh: Option<f64>,
    target_tire_pressure_psi: Option<f64>,
    battery_config: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    // Enrichment fields (migration 0023)
    interior_color: Option<String>,
    wheel_option: Option<String>,
    max_vehicle_power_kw: Option<f64>,
    charge_port_type: Option<String>,
    battery_cell_type: Option<String>,
    supported_features: Option<serde_json::Value>,
    // Backfill tracking (migration 0025)
    history_backfill_status: Option<String>,
    history_backfilled_at: Option<chrono::DateTime<chrono::Utc>>,
    history_session_count: Option<i32>,
    worker_health: Option<String>,
    worker_health_msg: Option<String>,
    auth_state: Option<String>,
    auth_reason_code: Option<String>,
    token_created_at: Option<chrono::DateTime<chrono::Utc>>,
    membership_role: String,
}

#[derive(Deserialize)]
struct RefreshCredentialsBody {
    rivian_vehicle_id: Option<String>,
}

#[derive(Debug, Default, sqlx::FromRow)]
struct VehicleRuntimeStateRow {
    is_online: Option<bool>,
    last_event_at: Option<chrono::DateTime<chrono::Utc>>,
    last_payload_at: Option<chrono::DateTime<chrono::Utc>>,
    last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    last_ws_received_at: Option<chrono::DateTime<chrono::Utc>>,
    last_ws_payload_received_at: Option<chrono::DateTime<chrono::Utc>>,
    last_ws_heartbeat_received_at: Option<chrono::DateTime<chrono::Utc>>,
    last_charge_history_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    last_charge_history_success_at: Option<chrono::DateTime<chrono::Utc>>,
    worker_health: Option<String>,
    worker_health_msg: Option<String>,
    auth_state: Option<String>,
    auth_reason_code: Option<String>,
}

#[derive(Debug, Default, sqlx::FromRow)]
struct VehicleStatusFieldSeenRow {
    tire_fl_psi_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_fr_psi_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_rl_psi_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_rr_psi_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_fl_status_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_fr_status_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_rl_status_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_rr_status_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_fl_valid_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_fr_valid_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_rl_valid_at: Option<chrono::DateTime<chrono::Utc>>,
    tire_rr_valid_at: Option<chrono::DateTime<chrono::Utc>>,
    hv_thermal_event_at: Option<chrono::DateTime<chrono::Utc>>,
    twelve_volt_health_at: Option<chrono::DateTime<chrono::Utc>>,
    ota_current_version_at: Option<chrono::DateTime<chrono::Utc>>,
    ota_available_version_at: Option<chrono::DateTime<chrono::Utc>>,
    ota_status_at: Option<chrono::DateTime<chrono::Utc>>,
    ota_current_status_at: Option<chrono::DateTime<chrono::Utc>>,
    charge_port_open_at: Option<chrono::DateTime<chrono::Utc>>,
    charger_derate_active_at: Option<chrono::DateTime<chrono::Utc>>,
    cabin_precon_status_at: Option<chrono::DateTime<chrono::Utc>>,
    cabin_precon_type_at: Option<chrono::DateTime<chrono::Utc>>,
    pet_mode_active_at: Option<chrono::DateTime<chrono::Utc>>,
    pet_mode_temp_ok_at: Option<chrono::DateTime<chrono::Utc>>,
    defrost_active_at: Option<chrono::DateTime<chrono::Utc>>,
    steering_wheel_heat_at: Option<chrono::DateTime<chrono::Utc>>,
    seat_fl_heat_at: Option<chrono::DateTime<chrono::Utc>>,
    seat_fr_heat_at: Option<chrono::DateTime<chrono::Utc>>,
    seat_rl_heat_at: Option<chrono::DateTime<chrono::Utc>>,
    seat_rr_heat_at: Option<chrono::DateTime<chrono::Utc>>,
    seat_fl_vent_at: Option<chrono::DateTime<chrono::Utc>>,
    seat_fr_vent_at: Option<chrono::DateTime<chrono::Utc>>,
    tonneau_locked_at: Option<chrono::DateTime<chrono::Utc>>,
    tonneau_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    side_bin_left_locked_at: Option<chrono::DateTime<chrono::Utc>>,
    side_bin_right_locked_at: Option<chrono::DateTime<chrono::Utc>>,
    side_bin_left_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    side_bin_right_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    window_fl_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    window_fr_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    window_rl_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    window_rr_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    gear_guard_locked_at: Option<chrono::DateTime<chrono::Utc>>,
    gear_guard_video_status_at: Option<chrono::DateTime<chrono::Utc>>,
    wiper_fluid_low_at: Option<chrono::DateTime<chrono::Utc>>,
    brake_fluid_low_at: Option<chrono::DateTime<chrono::Utc>>,
    alarm_active_at: Option<chrono::DateTime<chrono::Utc>>,
    service_mode_at: Option<chrono::DateTime<chrono::Utc>>,
    closure_frunk_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    closure_liftgate_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    closure_tailgate_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    door_front_left_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    door_front_right_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    door_rear_left_closed_at: Option<chrono::DateTime<chrono::Utc>>,
    door_rear_right_closed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Serializable response shape for `GET /v1/vehicles/:id/status`.
/// Using a typed struct instead of `serde_json::json!` avoids the recursive
/// macro expansion that exhausts the compiler's recursion limit as fields grow.
#[derive(serde::Serialize)]
struct VehicleStatusResponse {
    vehicle_id: Uuid,
    is_online: bool,
    last_event_at: Option<chrono::DateTime<chrono::Utc>>,
    last_updated: Option<chrono::DateTime<chrono::Utc>>,
    last_payload_at: Option<chrono::DateTime<chrono::Utc>>,
    last_heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    last_ws_received_at: Option<chrono::DateTime<chrono::Utc>>,
    last_ws_payload_received_at: Option<chrono::DateTime<chrono::Utc>>,
    last_ws_heartbeat_received_at: Option<chrono::DateTime<chrono::Utc>>,
    last_charge_history_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    last_charge_history_success_at: Option<chrono::DateTime<chrono::Utc>>,
    worker_health: Option<String>,
    worker_health_msg: Option<String>,
    auth_state: Option<String>,
    auth_reason_code: Option<String>,
    credential_issued_at: Option<chrono::DateTime<chrono::Utc>>,
    expected_renewal_at: Option<chrono::DateTime<chrono::Utc>>,
    renewal_state: Option<RivianRenewalState>,
    battery_level: Option<f64>,
    battery_level_ts: Option<chrono::DateTime<chrono::Utc>>,
    range_miles: Option<f64>,
    range_miles_ts: Option<chrono::DateTime<chrono::Utc>>,
    battery_capacity_kwh: Option<f64>,
    battery_limit: Option<f64>,
    battery_limit_ts: Option<chrono::DateTime<chrono::Utc>>,
    power_state: Option<String>,
    power_state_ts: Option<chrono::DateTime<chrono::Utc>>,
    charger_state: Option<String>,
    charger_state_ts: Option<chrono::DateTime<chrono::Utc>>,
    charger_status: Option<String>,
    charger_status_ts: Option<chrono::DateTime<chrono::Utc>>,
    time_to_end_of_charge_min: Option<i32>,
    time_to_end_of_charge_min_ts: Option<chrono::DateTime<chrono::Utc>>,
    speed_mph: Option<f64>,
    speed_mph_ts: Option<chrono::DateTime<chrono::Utc>>,
    altitude_m: Option<f64>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    location_ts: Option<chrono::DateTime<chrono::Utc>>,
    drive_mode: Option<String>,
    gear_status: Option<String>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    heading_deg: Option<f64>,
    odometer_miles: Option<f64>,
    odometer_miles_ts: Option<chrono::DateTime<chrono::Utc>>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
    tire_min_psi: Option<f64>,
    tire_fl_status: Option<String>,
    tire_fr_status: Option<String>,
    tire_rl_status: Option<String>,
    tire_rr_status: Option<String>,
    tire_fl_valid: Option<bool>,
    tire_fr_valid: Option<bool>,
    tire_rl_valid: Option<bool>,
    tire_rr_valid: Option<bool>,
    door_front_left_locked: Option<bool>,
    door_front_right_locked: Option<bool>,
    door_rear_left_locked: Option<bool>,
    door_rear_right_locked: Option<bool>,
    door_front_left_closed: Option<bool>,
    door_front_right_closed: Option<bool>,
    door_rear_left_closed: Option<bool>,
    door_rear_right_closed: Option<bool>,
    closure_frunk_locked: Option<bool>,
    closure_frunk_closed: Option<bool>,
    closure_liftgate_locked: Option<bool>,
    closure_liftgate_closed: Option<bool>,
    closure_tailgate_locked: Option<bool>,
    closure_tailgate_closed: Option<bool>,
    ota_current_version: Option<String>,
    ota_available_version: Option<String>,
    ota_status: Option<String>,
    ota_current_status: Option<String>,
    hv_thermal_event: Option<String>,
    twelve_volt_health: Option<String>,
    doors_locked: Option<bool>,
    open_closures: serde_json::Value,
    tire_pressure_status: Option<String>,
    software_update_status: Option<String>,
    // Extended vehicleStatus fields (migration 0022)
    charge_port_open: Option<bool>,
    charger_derate_active: Option<bool>,
    cabin_precon_status: Option<String>,
    cabin_precon_type: Option<String>,
    pet_mode_active: Option<bool>,
    pet_mode_temp_ok: Option<bool>,
    defrost_active: Option<bool>,
    steering_wheel_heat: Option<i16>,
    seat_fl_heat: Option<i16>,
    seat_fr_heat: Option<i16>,
    seat_rl_heat: Option<i16>,
    seat_rr_heat: Option<i16>,
    seat_fl_vent: Option<i16>,
    seat_fr_vent: Option<i16>,
    tonneau_locked: Option<bool>,
    tonneau_closed: Option<bool>,
    side_bin_left_locked: Option<bool>,
    side_bin_right_locked: Option<bool>,
    side_bin_left_closed: Option<bool>,
    side_bin_right_closed: Option<bool>,
    window_fl_closed: Option<bool>,
    window_fr_closed: Option<bool>,
    window_rl_closed: Option<bool>,
    window_rr_closed: Option<bool>,
    gear_guard_locked: Option<bool>,
    gear_guard_video_status: Option<String>,
    wiper_fluid_low: Option<bool>,
    brake_fluid_low: Option<bool>,
    alarm_active: Option<bool>,
    service_mode: Option<bool>,
    telemetry_stale: bool,
    telemetry_stale_reason: Option<String>,
    field_availability: BTreeMap<String, StatusFieldAvailability>,
}

#[derive(Debug, sqlx::FromRow)]
struct VehicleImageRow {
    placement: String,
    design: Option<String>,
    size: Option<String>,
    resolution: Option<String>,
    url: String,
    overlays: serde_json::Value,
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct VehicleArtworkCacheStateRow {
    status: String,
    asset_count: i32,
    ready_asset_count: i32,
    attempts: i32,
    last_repair_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    last_repair_success_at: Option<chrono::DateTime<chrono::Utc>>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VehicleImageOverlay {
    url: String,
    overlay: Option<String>,
    #[serde(rename = "zIndex")]
    z_index: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MirroredOverlayAsset {
    source_url: String,
    mirror_key: String,
    mirror_relpath: String,
    mime_type: String,
    sha256: String,
    overlay: Option<String>,
    #[serde(rename = "zIndex")]
    z_index: Option<i32>,
}

#[derive(Debug, Clone)]
struct MirroredFileAsset {
    mirror_key: String,
    mirror_relpath: String,
    mime_type: String,
    sha256: String,
}

#[derive(Debug, sqlx::FromRow)]
struct RawVehicleSampleRow {
    ts: chrono::DateTime<chrono::Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude_m: Option<f64>,
    speed_mph: Option<f64>,
    battery_level: Option<f64>,
    battery_capacity_wh: Option<f64>,
    distance_to_empty_mi: Option<f64>,
    battery_limit: Option<f64>,
    power_state: Option<String>,
    charger_state: Option<String>,
    charger_status: Option<String>,
    time_to_end_of_charge_min: Option<i32>,
    drive_mode: Option<String>,
    gear_status: Option<String>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    hvac_active: Option<bool>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
    heading_deg: Option<f64>,
    odometer_miles: Option<f64>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
    tire_fl_status: Option<String>,
    tire_fr_status: Option<String>,
    tire_rl_status: Option<String>,
    tire_rr_status: Option<String>,
    tire_fl_valid: Option<bool>,
    tire_fr_valid: Option<bool>,
    tire_rl_valid: Option<bool>,
    tire_rr_valid: Option<bool>,
    door_front_left_locked: Option<bool>,
    door_front_right_locked: Option<bool>,
    door_rear_left_locked: Option<bool>,
    door_rear_right_locked: Option<bool>,
    door_front_left_closed: Option<bool>,
    door_front_right_closed: Option<bool>,
    door_rear_left_closed: Option<bool>,
    door_rear_right_closed: Option<bool>,
    closure_frunk_closed: Option<bool>,
    closure_liftgate_closed: Option<bool>,
    closure_tailgate_closed: Option<bool>,
    ota_current_version: Option<String>,
    ota_available_version: Option<String>,
    ota_status: Option<String>,
    ota_current_status: Option<String>,
    hv_thermal_event: Option<String>,
    twelve_volt_health: Option<String>,
    is_online: Option<bool>,
}

#[derive(Debug, sqlx::FromRow)]
struct RawVehicleCoverageRow {
    first_event_at: Option<chrono::DateTime<chrono::Utc>>,
    last_event_at: Option<chrono::DateTime<chrono::Utc>>,
    sample_count: i64,
    odometer_samples: i64,
    battery_samples: i64,
    range_samples: i64,
    outside_temp_samples: i64,
    power_samples: i64,
    regen_samples: i64,
    tire_pressure_samples: i64,
    lock_samples: i64,
    software_samples: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct RawTelemetryFieldCoverageRow {
    field: String,
    sample_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct TelemetryLaneRow {
    bucket: chrono::DateTime<chrono::Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude_m: Option<f64>,
    speed_mph: Option<f64>,
    battery_level: Option<f64>,
    battery_capacity_wh: Option<f64>,
    distance_to_empty_mi: Option<f64>,
    battery_limit: Option<f64>,
    time_to_end_of_charge_min: Option<f64>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
    heading_deg: Option<f64>,
    odometer_miles: Option<f64>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
}

#[derive(Debug, Serialize)]
struct TelemetryLaneWindow {
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    resolution_seconds: i64,
    approximate: bool,
}

#[derive(Debug, Serialize)]
struct TelemetryLane {
    numeric: BTreeMap<String, Vec<Option<f64>>>,
    coverage: BTreeMap<String, i64>,
    source: &'static str,
}

#[derive(Debug, Serialize)]
struct TelemetryLaneFrame {
    vehicle_id: Uuid,
    window: TelemetryLaneWindow,
    spine: Vec<chrono::DateTime<chrono::Utc>>,
    lanes: BTreeMap<String, TelemetryLane>,
    truncated: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct RawEventSummaryRow {
    id: Uuid,
    received_at: chrono::DateTime<chrono::Utc>,
    event_type: String,
    message_type: Option<String>,
    has_json: bool,
    has_payload: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct RawEventDetailRow {
    id: Uuid,
    received_at: chrono::DateTime<chrono::Utc>,
    event_type: String,
    message_type: Option<String>,
    payload_json: Option<serde_json::Value>,
    payload_text: Option<String>,
}

const RAW_TELEMETRY_FIELDS: &[&str] = &[
    "latitude",
    "longitude",
    "altitude_m",
    "speed_mph",
    "battery_level",
    "battery_capacity_wh",
    "distance_to_empty_mi",
    "battery_limit",
    "power_state",
    "charger_state",
    "charger_status",
    "time_to_end_of_charge_min",
    "drive_mode",
    "gear_status",
    "cabin_temp_c",
    "driver_temp_c",
    "outside_temp_c",
    "hvac_active",
    "power_kw",
    "regen_power_kw",
    "heading_deg",
    "odometer_miles",
    "tire_fl_psi",
    "tire_fr_psi",
    "tire_rl_psi",
    "tire_rr_psi",
    "tire_fl_status",
    "tire_fr_status",
    "tire_rl_status",
    "tire_rr_status",
    "tire_fl_valid",
    "tire_fr_valid",
    "tire_rl_valid",
    "tire_rr_valid",
    "door_front_left_locked",
    "door_front_right_locked",
    "door_rear_left_locked",
    "door_rear_right_locked",
    "door_front_left_closed",
    "door_front_right_closed",
    "door_rear_left_closed",
    "door_rear_right_closed",
    "closure_frunk_closed",
    "closure_liftgate_closed",
    "closure_tailgate_closed",
    "ota_current_version",
    "ota_available_version",
    "ota_status",
    "ota_current_status",
    "hv_thermal_event",
    "twelve_volt_health",
    "is_online",
];

#[derive(Serialize)]
struct ConnectResponse {
    status: &'static str,
    requires_otp: bool,
    challenge_id: Option<String>,
    vehicle_id: Option<Uuid>,
    vehicles: Vec<RivianVehicleSummary>,
}

fn age_identity(state: &AppState) -> Result<age::x25519::Identity, AppError> {
    state
        .age_key
        .parse::<age::x25519::Identity>()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("bad age key: {e}")))
}

async fn store_encrypted_redis<T: Serialize>(
    state: &AppState,
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
    value: &T,
    ttl_secs: u64,
) -> Result<(), AppError> {
    let identity = age_identity(state)?;
    let encrypted = crate::ingestion::session_store::encrypt_json(value, &identity)
        .map_err(AppError::Internal)?;
    let result = redis::cmd("SET")
        .arg(key)
        .arg(encrypted)
        .arg("EX")
        .arg(ttl_secs)
        .query_async::<()>(conn)
        .await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            warn!(operation = "connect_session.write", error = %error, "secure_session_store.unavailable");
            Err(AppError::Redis(error))
        }
    }
}

async fn load_encrypted_redis<T: DeserializeOwned>(
    state: &AppState,
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> Result<Option<T>, AppError> {
    let stored = redis::cmd("GET")
        .arg(key)
        .query_async::<Option<Vec<u8>>>(conn)
        .await;
    match stored {
        Err(error) => {
            warn!(operation = "connect_session.read", error = %error, "secure_session_store.unavailable");
            Err(AppError::Redis(error))
        }
        Ok(None) => Ok(None),
        Ok(Some(encrypted)) => {
            let identity = age_identity(state)?;
            match crate::ingestion::session_store::decrypt_json::<T>(&encrypted, &identity) {
                Ok(value) => Ok(Some(value)),
                Err(error) => {
                    warn!(key, error = %error, "vehicle.connect_session.decrypt_failed");
                    Err(AppError::RivianConnectSessionExpired)
                }
            }
        }
    }
}

fn map_rivian_login_error(error: RivianAuthError) -> AppError {
    RivianLoginPhase::Password.translate(error)
}

fn map_rivian_otp_error(error: RivianAuthError) -> AppError {
    RivianLoginPhase::Verification.translate(error)
}

async fn connect(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ConnectBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    info!(user_id = %auth.user_id, email_present = !body.email.trim().is_empty(), "vehicle.connect.start");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let login = crate::ingestion::rivian_auth::rivian_login(&client, &body.email, &body.password)
        .await
        .map_err(|error| {
            warn!(user_id = %auth.user_id, error = %error, "vehicle.connect.login_failed");
            map_rivian_login_error(error)
        })?;
    connection::ConnectionExchange::from_login(login, auth.user_id)
        .deliver(&state, &client, auth.user_id)
        .await
}

async fn connect_otp(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<OtpBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    info!(
        user_id = %auth.user_id,
        challenge_id = %body.challenge_id,
        "vehicle.connect_otp.start"
    );

    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    let key = format!("rivian:otp:{}", body.challenge_id);
    let pending = load_encrypted_redis::<PendingOtpChallenge>(&state, &mut conn, &key).await?;
    let pending = pending.ok_or_else(|| {
        warn!(
            user_id = %auth.user_id,
            challenge_id = %body.challenge_id,
            "vehicle.connect_otp.challenge_missing"
        );
        AppError::RivianConnectSessionExpired
    })?;

    if pending.user_id != auth.user_id {
        warn!(
            user_id = %auth.user_id,
            challenge_id = %body.challenge_id,
            stored_user_id = %pending.user_id,
            "vehicle.connect_otp.challenge_user_mismatch"
        );
        return Err(AppError::RivianConnectSessionExpired);
    }

    let challenge = crate::ingestion::rivian_auth::RivianOtpChallenge {
        email: pending.email,
        otp_token: pending.otp_token,
        csrf_token: pending.csrf_token,
        app_session_token: pending.app_session_token,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let tokens =
        crate::ingestion::rivian_auth::rivian_login_otp(&client, &challenge, &body.otp_code)
            .await
            .map_err(|e| {
                warn!(
                    user_id = %auth.user_id,
                    challenge_id = %body.challenge_id,
                    error = %e,
                    "vehicle.connect_otp.login_failed"
                );
                map_rivian_otp_error(e)
            })?;
    connection::DiscoveredSession::load(&client, tokens)
        .await?
        .publish(&state, &mut conn, auth.user_id, Some(&key))
        .await
}

async fn add_vehicle(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<AddVehicleBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    add_vehicle_with_lookup(state, auth, body, lookup_rivian_vehicles).await
}

async fn lookup_rivian_vehicles(
    tokens: &crate::ingestion::session_store::RivianTokenBundle,
) -> Result<Vec<RivianVehicleSummary>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| AppError::Internal(error.into()))?;
    crate::ingestion::rivian_auth::rivian_user_vehicles(&client, tokens)
        .await
        .map_err(|_| AppError::RivianApi("Unable to verify Rivian vehicle access".into()))
}

fn require_account_vehicle(
    account_vehicles: &[RivianVehicleSummary],
    rivian_vehicle_id: &str,
) -> Result<RivianVehicleSummary, AppError> {
    account_vehicles
        .iter()
        .find(|vehicle| vehicle.id == rivian_vehicle_id)
        .cloned()
        .ok_or_else(|| AppError::Validation("Rivian account does not include this vehicle".into()))
}

async fn add_vehicle_with_lookup(
    state: AppState,
    auth: AuthUser,
    body: AddVehicleBody,
    lookup: impl AsyncFnOnce(
        &crate::ingestion::session_store::RivianTokenBundle,
    ) -> Result<Vec<RivianVehicleSummary>, AppError>,
) -> Result<Json<serde_json::Value>, AppError> {
    let remote_id = body.rivian_vehicle_id.trim().to_owned();
    info!(user_id = %auth.user_id, rivian_vehicle_id = %body.rivian_vehicle_id, "vehicle.add.start");
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    let key = format!("rivian:connect:{}", auth.user_id);
    let tokens = load_encrypted_redis::<crate::ingestion::session_store::RivianTokenBundle>(
        &state, &mut conn, &key,
    )
    .await?
    .ok_or(AppError::RivianConnectSessionExpired)?;
    let vehicles = lookup(&tokens).await?;
    let account_vehicle = require_account_vehicle(&vehicles, &remote_id)?;
    let saved = enrollment::VerifiedEnrollment {
        remote_id,
        account_vehicle,
        requested: body,
    }
    .persist(&state, auth.user_id, &tokens)
    .await?;
    if saved.created {
        queue_vehicle_artwork_repair(&state, saved.vehicle_id).await;
    }
    if let Err(error) = redis::AsyncCommands::del::<_, ()>(&mut conn, &key).await {
        warn!(user_id = %auth.user_id, vehicle_id = %saved.vehicle_id, err = %error, "vehicle.add.connect_session_cleanup_failed");
    }
    info!(user_id = %auth.user_id, vehicle_id = %saved.vehicle_id, "vehicle.add.persisted");
    Ok(Json(saved.notify(&state).await))
}

async fn refresh_vehicle_credentials(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Json(body): Json<RefreshCredentialsBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner", "manager"]).await?;
    let remote = require_remote_backed_vehicle(&state.pool, vid).await?;
    if body
        .rivian_vehicle_id
        .as_deref()
        .is_some_and(|requested| requested != remote)
    {
        return Err(AppError::Validation(
            "selected Rivian vehicle does not match this local vehicle".into(),
        ));
    }
    let mut redis = state.redis.get_multiplexed_async_connection().await?;
    let staging_key = format!("rivian:connect:{}", auth.user_id);
    let tokens = load_encrypted_redis::<crate::ingestion::session_store::RivianTokenBundle>(
        &state,
        &mut redis,
        &staging_key,
    )
    .await?
    .ok_or(AppError::RivianConnectSessionExpired)?;
    let verified = connection::VerifiedRefresh::verify(&state, vid, &remote, tokens).await?;
    let saved = verified.persist(&state.pool, auth.user_id).await?;
    if let Err(error) = redis::AsyncCommands::del::<_, ()>(&mut redis, &staging_key).await {
        warn!(user_id = %auth.user_id, vehicle_id = %vid, err = %error, "vehicle.refresh_credentials.connect_session_cleanup_failed");
    }
    Ok(Json(saved.notify(&state).await))
}

async fn wait_for_vehicle_data(pool: &sqlx::PgPool, vehicle_id: Uuid) -> bool {
    for _ in 0..10 {
        let runtime = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT worker_health, auth_state
             FROM panda.vehicle_runtime_state WHERE vehicle_id = $1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        if runtime
            .as_ref()
            .is_some_and(|(health, auth)| vehicle_data_ready(health.as_deref(), auth.as_deref()))
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

fn vehicle_data_ready(worker_health: Option<&str>, auth_state: Option<&str>) -> bool {
    worker_health == Some("connected") && auth_state == Some("authorized")
}

async fn delete_vehicle(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    crate::private_deployment::history::require_vehicle_deletion_allowed(&state.config)?;
    sqlx::query("DELETE FROM panda.vehicles WHERE id=$1")
        .bind(vid)
        .execute(&state.pool)
        .await?;
    let next_default = sqlx::query_scalar::<_, Option<Uuid>>(
        "UPDATE panda.users u SET default_vehicle_id=COALESCE((SELECT vehicle_id FROM panda.vehicle_memberships WHERE user_id=$1 AND is_default=TRUE LIMIT 1),u.default_vehicle_id) WHERE u.id=$1 RETURNING default_vehicle_id")
        .bind(auth.user_id).fetch_optional(&state.pool).await?.flatten();
    state
        .supervisor
        .send(SupervisorCommand::StopWorker { vehicle_id: vid })
        .await;
    Ok(Json(
        serde_json::json!({"ok":true,"default_vehicle_id":next_default}),
    ))
}

#[derive(Deserialize)]
struct UpdateBatteryConfigBody {
    battery_capacity_kwh: Option<f64>,
    battery_config: Option<String>,
    target_tire_pressure_psi: Option<f64>,
}

async fn update_battery_config(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Json(body): Json<UpdateBatteryConfigBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    update_vehicle_settings_impl(&state, auth.user_id, vid, body).await
}

async fn update_vehicle_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Json(body): Json<UpdateBatteryConfigBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    update_vehicle_settings_impl(&state, auth.user_id, vid, body).await
}

async fn update_vehicle_settings_impl(
    state: &AppState,
    user_id: Uuid,
    vid: Uuid,
    body: UpdateBatteryConfigBody,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, user_id, vid, &["owner", "manager"]).await?;
    let update = VehicleSettingsUpdate::validate(body)?;
    update.persist(&state.pool, vid).await?;
    Ok(Json(serde_json::json!({"ok":true})))
}

#[derive(Deserialize)]
struct UpdateVehicleNameBody {
    name: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct VehicleMemberRow {
    user_id: Uuid,
    email: String,
    role: String,
    is_default: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
struct AddVehicleMemberBody {
    email: String,
    role: String,
}

#[derive(Deserialize)]
struct CreateVehicleInviteBody {
    email: String,
    role: String,
    expires_in_days: Option<i32>,
}

#[derive(sqlx::FromRow, Serialize)]
struct VehicleInviteRow {
    id: Uuid,
    vehicle_id: Uuid,
    invited_by: Uuid,
    invitee_email: String,
    role: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
struct UpdateVehicleMemberBody {
    role: String,
}

async fn update_vehicle_name(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Json(body): Json<UpdateVehicleNameBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner", "manager"]).await?;
    let name = canonical_vehicle_name(&body.name);
    match name {
        "" => Err(AppError::Validation("name must not be blank".into())),
        _ => {
            sqlx::query("INSERT INTO panda.vehicle_user_settings (vehicle_id,user_id,display_name) VALUES ($1,$2,$3) ON CONFLICT (vehicle_id,user_id) DO UPDATE SET display_name=EXCLUDED.display_name,updated_at=now()")
                .bind(vid).bind(auth.user_id).bind(name).execute(&state.pool).await?;
            Ok(Json(serde_json::json!({"ok":true})))
        }
    }
}

async fn set_default_vehicle(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_session_access(&state.pool, &auth, vid).await?;
    let mut tx = state.pool.begin().await?;
    enrollment::DefaultSelection::Replace {
        user_id: auth.user_id,
        vehicle_id: vid,
    }
    .apply(&mut tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        serde_json::json!({ "ok": true, "default_vehicle_id": vid }),
    ))
}

async fn list_vehicle_members(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_session_access(&state.pool, &auth, vid).await?;

    let rows = sqlx::query_as::<_, VehicleMemberRow>(
        "SELECT vm.user_id, u.email, vm.role, vm.is_default, vm.created_at
         FROM panda.vehicle_memberships vm
         JOIN panda.users u ON u.id = vm.user_id
         WHERE vm.vehicle_id = $1
         ORDER BY CASE vm.role WHEN 'owner' THEN 0 WHEN 'manager' THEN 1 ELSE 2 END, u.email",
    )
    .bind(vid)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({
        "members": rows,
    })))
}

async fn add_vehicle_member(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Json(body): Json<AddVehicleMemberBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    let email = membership::member_email(&body.email)?;
    membership::member_role(&body.role)?;
    match membership::MemberDestination::resolve(&state.pool, email).await? {
        membership::MemberDestination::Registered(user_id) => {
            let grant = membership::MembershipGrant {
                vehicle_id: vid,
                user_id,
                role: &body.role,
            };
            if grant.write(&state.pool).await? == 0 {
                return Err(AppError::Conflict("member already exists".into()));
            }
            grant.settings(&state.pool).await?;
            Ok(Json(serde_json::json!({"ok":true,"invite_created":false})))
        }
        membership::MemberDestination::Invitation(email) => {
            let invite = membership::IssuedInvitation::create(
                &state.pool,
                vid,
                auth.user_id,
                &email,
                &body.role,
                14,
            )
            .await?;
            Ok(Json(
                serde_json::json!({"ok":true,"invite_created":true,"invite_token":invite.token}),
            ))
        }
    }
}

async fn update_vehicle_member(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path((vid, member_user_id)): axum::extract::Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateVehicleMemberBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    membership::member_role(&body.role)?;
    let changed = sqlx::query_scalar::<_, Uuid>(
        "UPDATE panda.vehicle_memberships SET role=$3,updated_at=now() WHERE vehicle_id=$1 AND user_id=$2 RETURNING user_id")
        .bind(vid).bind(member_user_id).bind(&body.role).fetch_optional(&state.pool).await?;
    changed.ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({"ok":true})))
}

async fn remove_vehicle_member(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path((vid, member_user_id)): axum::extract::Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    let (role, owners) = sqlx::query_as::<_, (String, i64)>(
        "SELECT m.role,
                CASE WHEN m.role='owner' THEN
                    (SELECT COUNT(*) FROM panda.vehicle_memberships WHERE vehicle_id=$1 AND role='owner')
                ELSE 0 END AS owners
         FROM panda.vehicle_memberships m WHERE m.vehicle_id=$1 AND m.user_id=$2",
    )
    .bind(vid)
    .bind(member_user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if role == "owner" && owners <= 1 {
        return Err(AppError::Validation(
            "vehicle must keep at least one owner".into(),
        ));
    }
    sqlx::query("DELETE FROM panda.vehicle_memberships WHERE vehicle_id=$1 AND user_id=$2")
        .bind(vid)
        .bind(member_user_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(serde_json::json!({"ok":true})))
}

async fn create_vehicle_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vid): Path<Uuid>,
    Json(body): Json<CreateVehicleInviteBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    membership::member_role(&body.role)?;
    let email = membership::member_email(&body.email)?;
    let days = body.expires_in_days.unwrap_or(14).clamp(1, 30);
    let invite = membership::IssuedInvitation::create(
        &state.pool,
        vid,
        auth.user_id,
        &email,
        &body.role,
        days,
    )
    .await?;
    Ok(Json(
        serde_json::json!({"ok":true,"invite_token":invite.token,"expires_at":invite.expires_at}),
    ))
}

async fn list_vehicle_invites(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vid): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    let rows = sqlx::query_as::<_, VehicleInviteRow>(
        "SELECT id, vehicle_id, invited_by, invitee_email, role, expires_at, accepted_at, revoked_at, created_at
         FROM panda.vehicle_invites
         WHERE vehicle_id = $1
         ORDER BY created_at DESC",
    )
    .bind(vid)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({ "invites": rows })))
}

async fn revoke_vehicle_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vid, invite_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_role(&state.pool, auth.user_id, vid, &["owner"]).await?;
    sqlx::query(
        "UPDATE panda.vehicle_invites
         SET revoked_at = now(), updated_at = now()
         WHERE id = $1 AND vehicle_id = $2 AND accepted_at IS NULL",
    )
    .bind(invite_id)
    .bind(vid)
    .execute(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn preview_invite(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let preview = sqlx::query_as::<_, InvitationPreview>(
        "SELECT i.id,i.vehicle_id,COALESCE(v.name,v.model) AS vehicle_name,i.invitee_email,i.role,i.expires_at,i.accepted_at,i.revoked_at FROM panda.vehicle_invites i JOIN panda.vehicles v ON v.id=i.vehicle_id WHERE i.token_hash=$1")
        .bind(hash_invite_token(&token)).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    serde_json::to_value(preview)
        .map(Json)
        .map_err(|error| AppError::Internal(error.into()))
}

async fn accept_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(token): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut tx = state.pool.begin().await?;
    let invite = membership::LockedInvitation::load(&mut tx, auth.user_id, &token).await?;
    let vehicle_id = invite.accept(&mut tx, auth.user_id).await?;
    tx.commit().await?;
    Ok(Json(serde_json::json!({"ok":true,"vehicle_id":vehicle_id})))
}

fn hash_invite_token(token: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().to_vec()
}

async fn list_vehicles(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query_as::<_, VehicleListRow>(
        "SELECT v.id, v.rivian_vehicle_id, v.model, v.trim, v.vin, v.color, \
                COALESCE(vus.display_name, v.name) AS name, v.battery_capacity_wh, v.target_tire_pressure_psi, \
                v.battery_config, v.created_at, v.interior_color, v.wheel_option, v.max_vehicle_power_kw, \
                v.charge_port_type, v.battery_cell_type, v.supported_features, \
                v.history_backfill_status, v.history_backfilled_at, v.history_session_count, \
                vrs.worker_health, vrs.worker_health_msg, vrs.auth_state, vrs.auth_reason_code, \
                vc.token_created_at, \
                vm.role AS membership_role \
         FROM panda.vehicles v \
         JOIN panda.vehicle_memberships vm ON vm.vehicle_id = v.id \
         LEFT JOIN panda.vehicle_user_settings vus ON vus.vehicle_id = v.id AND vus.user_id = vm.user_id \
         LEFT JOIN panda.vehicle_runtime_state vrs ON vrs.vehicle_id = v.id \
         LEFT JOIN panda.vehicle_credentials vc ON vc.vehicle_id = v.id \
         WHERE vm.user_id = $1
           AND ($2::uuid IS NULL OR v.id = $2)
         ORDER BY v.created_at",
    )
    .bind(auth.user_id)
    .bind(auth.api_vehicle_id)
    .fetch_all(&state.pool)
    .await?;

    let mut vehicles = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row.id;
        let is_demo = is_demo_vehicle_key(&row.rivian_vehicle_id);
        let mut entry = row.into_listing(auth.user_id, Utc::now());
        if !is_demo {
            queue_vehicle_artwork_repair(&state, id).await;
        }
        entry["images"] = fetch_vehicle_images_json(&state.pool, &state.config, id)
            .await
            .unwrap_or_else(|_| serde_json::json!({"all": []}));
        vehicles.push(entry);
    }
    Ok(Json(serde_json::json!({"vehicles": vehicles})))
}

async fn create_demo_vehicle(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Option<Json<CreateDemoVehicleBody>>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let mut tx = state.pool.begin().await?;
    lock_demo_creator(&mut tx, auth.user_id).await?;
    let requested = body
        .as_ref()
        .map(|body| body.model.as_str())
        .unwrap_or("R1T");
    let specification = enrollment::DemoSpecification::parse(requested)?;
    let result = specification.ensure(&mut tx, auth.user_id).await?;
    tx.commit().await?;
    Ok(Json(result.response()))
}

async fn refresh_demo_vehicle(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    require_vehicle_access(&auth, vehicle_id)?;
    let (demo_key, model) = sqlx::query_as::<_, (String, String)>(
        "SELECT rivian_vehicle_id, model FROM panda.vehicles WHERE id=$1",
    )
    .bind(vehicle_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    if !is_demo_vehicle_key(&demo_key) {
        return Err(AppError::Validation(
            "only demo vehicles can refresh demo data".into(),
        ));
    }
    if state.config.is_production() {
        return Err(AppError::Conflict(
            "Demo history is protected in production. Refresh is unavailable; existing data was preserved."
                .into(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    let summary = seed_demo_vehicle(&mut tx, vehicle_id, &model, Utc::now()).await?;
    tx.commit().await?;

    Ok(Json(
        enrollment::DemoCreation::refreshed(vehicle_id, summary).response(),
    ))
}

async fn vehicle_status(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
) -> Result<Json<VehicleStatusResponse>, AppError> {
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    queue_vehicle_artwork_repair(&state, vid).await;
    let snapshot = status::Snapshot::read(&state.pool, vid).await?;
    Ok(Json(snapshot.render(vid, Utc::now())))
}

#[cfg(test)]
mod range_tests {
    use crate::routes::range_normalization::normalize_remaining_range_miles;

    #[test]
    fn leaves_plausible_miles_untouched() {
        let value = normalize_remaining_range_miles(Some(227.0), Some(71.0), Some(135_000.0), None);
        assert_eq!(value, Some(227.0));
    }

    #[test]
    fn converts_implausible_km_value_to_miles() {
        let value = normalize_remaining_range_miles(Some(380.0), Some(71.0), Some(135_000.0), None);
        assert_eq!(
            value.map(|miles| (miles * 10.0).round() / 10.0),
            Some(236.1)
        );
    }

    #[test]
    fn leaves_high_but_plausible_miles_untouched() {
        let value = normalize_remaining_range_miles(Some(227.0), Some(71.0), Some(105_000.0), None);
        assert_eq!(value, Some(227.0));
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::{derive_vehicle_status_freshness, LatestVehicleTelemetry, VehicleRuntimeStateRow};
    use chrono::{TimeZone, Utc};

    #[test]
    fn flags_ws_silence_as_stale() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 12, 0, 0).unwrap();
        let runtime = VehicleRuntimeStateRow {
            last_ws_received_at: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 40, 0).unwrap()),
            worker_health: Some("connected".to_string()),
            ..Default::default()
        };

        let (worker_health, telemetry_stale, reason) = derive_vehicle_status_freshness(
            now,
            Some(&runtime),
            &LatestVehicleTelemetry::default(),
        );

        assert_eq!(worker_health.as_deref(), Some("stale"));
        assert!(telemetry_stale);
        assert_eq!(reason.as_deref(), Some("ws_silent"));
    }

    #[test]
    fn flags_old_battery_timestamp_even_when_worker_is_connected() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 12, 0, 0).unwrap();
        let runtime = VehicleRuntimeStateRow {
            last_ws_received_at: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            worker_health: Some("connected".to_string()),
            ..Default::default()
        };
        let latest = LatestVehicleTelemetry {
            ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            battery_level: Some(71.0),
            battery_level_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 10, 20, 0).unwrap()),
            ..Default::default()
        };

        let (worker_health, telemetry_stale, reason) =
            derive_vehicle_status_freshness(now, Some(&runtime), &latest);

        assert_eq!(worker_health.as_deref(), Some("connected"));
        assert!(telemetry_stale);
        assert_eq!(reason.as_deref(), Some("battery_stale"));
    }

    #[test]
    fn keeps_connected_feed_healthy_when_range_is_old() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 12, 0, 0).unwrap();
        let runtime = VehicleRuntimeStateRow {
            last_ws_received_at: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            worker_health: Some("connected".to_string()),
            ..Default::default()
        };
        let latest = LatestVehicleTelemetry {
            ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            distance_to_empty_mi: Some(210.0),
            distance_to_empty_mi_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 10, 20, 0).unwrap()),
            ..Default::default()
        };

        let (worker_health, telemetry_stale, reason) =
            derive_vehicle_status_freshness(now, Some(&runtime), &latest);

        assert_eq!(worker_health.as_deref(), Some("connected"));
        assert!(telemetry_stale);
        assert_eq!(reason.as_deref(), Some("range_stale"));
    }

    #[test]
    fn keeps_connected_feed_healthy_when_active_charging_data_is_old() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 12, 0, 0).unwrap();
        let runtime = VehicleRuntimeStateRow {
            last_ws_received_at: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            worker_health: Some("connected".to_string()),
            ..Default::default()
        };
        let latest = LatestVehicleTelemetry {
            ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            charger_state: Some("charging".to_string()),
            charger_state_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 10, 20, 0).unwrap()),
            ..Default::default()
        };

        let (worker_health, telemetry_stale, reason) =
            derive_vehicle_status_freshness(now, Some(&runtime), &latest);

        assert_eq!(worker_health.as_deref(), Some("connected"));
        assert!(telemetry_stale);
        assert_eq!(reason.as_deref(), Some("charging_stale"));
    }

    #[test]
    fn keeps_recent_runtime_as_healthy() {
        let now = Utc.with_ymd_and_hms(2026, 6, 19, 12, 0, 0).unwrap();
        let runtime = VehicleRuntimeStateRow {
            last_ws_received_at: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            worker_health: Some("connected".to_string()),
            ..Default::default()
        };
        let latest = LatestVehicleTelemetry {
            ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 59, 0).unwrap()),
            battery_level: Some(71.0),
            battery_level_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 11, 58, 30).unwrap()),
            ..Default::default()
        };

        let (worker_health, telemetry_stale, reason) =
            derive_vehicle_status_freshness(now, Some(&runtime), &latest);

        assert_eq!(worker_health.as_deref(), Some("connected"));
        assert!(!telemetry_stale);
        assert_eq!(reason, None);
    }
}

#[cfg(test)]
mod availability_tests {
    use super::{
        classify_status_field_availability, max_seen_at, StatusFieldAvailabilityReasonCode,
        StatusFieldAvailabilityState,
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn marks_missing_field_as_never_seen() {
        let latest_event_at = Some(Utc.with_ymd_and_hms(2026, 7, 7, 12, 0, 0).unwrap());
        let availability = classify_status_field_availability(latest_event_at, None, None);

        assert!(!availability.ever_seen);
        assert_eq!(
            availability.availability,
            StatusFieldAvailabilityState::NeverSeen
        );
        assert_eq!(
            availability.reason_code,
            Some(StatusFieldAvailabilityReasonCode::NeverSeen)
        );
    }

    #[test]
    fn marks_field_as_historical_when_latest_event_is_newer() {
        let latest_event_at = Some(Utc.with_ymd_and_hms(2026, 7, 7, 12, 0, 0).unwrap());
        let last_seen_at = Some(Utc.with_ymd_and_hms(2026, 7, 7, 10, 30, 0).unwrap());
        let availability = classify_status_field_availability(latest_event_at, last_seen_at, None);

        assert!(availability.ever_seen);
        assert_eq!(availability.last_seen_at, last_seen_at);
        assert_eq!(
            availability.availability,
            StatusFieldAvailabilityState::Historical
        );
        assert_eq!(
            availability.reason_code,
            Some(StatusFieldAvailabilityReasonCode::MissingRecentPayload)
        );
    }

    #[test]
    fn keeps_latest_field_as_current() {
        let latest_event_at = Some(Utc.with_ymd_and_hms(2026, 7, 7, 12, 0, 0).unwrap());
        let availability =
            classify_status_field_availability(latest_event_at, latest_event_at, None);

        assert_eq!(
            availability.availability,
            StatusFieldAvailabilityState::Current
        );
        assert_eq!(availability.reason_code, None);
    }

    #[test]
    fn preserves_invalid_sensor_reason_for_current_and_historical_fields() {
        let latest_event_at = Some(Utc.with_ymd_and_hms(2026, 7, 7, 12, 0, 0).unwrap());
        let current = classify_status_field_availability(
            latest_event_at,
            latest_event_at,
            Some(StatusFieldAvailabilityReasonCode::InvalidSensor),
        );
        let historical = classify_status_field_availability(
            latest_event_at,
            Some(Utc.with_ymd_and_hms(2026, 7, 7, 11, 0, 0).unwrap()),
            Some(StatusFieldAvailabilityReasonCode::InvalidSensor),
        );

        assert_eq!(
            current.reason_code,
            Some(StatusFieldAvailabilityReasonCode::InvalidSensor)
        );
        assert_eq!(
            historical.reason_code,
            Some(StatusFieldAvailabilityReasonCode::InvalidSensor)
        );
    }

    #[test]
    fn returns_latest_seen_timestamp() {
        let a = Utc.with_ymd_and_hms(2026, 7, 7, 8, 0, 0).unwrap();
        let b = Utc.with_ymd_and_hms(2026, 7, 7, 10, 0, 0).unwrap();
        let c = Utc.with_ymd_and_hms(2026, 7, 7, 9, 0, 0).unwrap();

        assert_eq!(max_seen_at([Some(a), Some(b), Some(c)]), Some(b));
    }
}

async fn vehicle_images(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    Ok(Json(
        fetch_vehicle_images_json(&state.pool, &state.config, vid).await?,
    ))
}

async fn cache_vehicle_images(
    pool: &sqlx::PgPool,
    config: &crate::config::Config,
    vehicle_id: Uuid,
    tokens: &crate::ingestion::session_store::RivianTokenBundle,
) {
    if !crate::services::outbound_policy::optional_traffic_allowed() {
        return;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    mark_vehicle_artwork_repair_attempt(pool, vehicle_id).await;
    crate::services::external_connections::record_attempt(
        pool,
        crate::services::external_connections::RIVIAN_ACCOUNT,
    )
    .await;
    let images = crate::ingestion::rivian_auth::rivian_vehicle_images(&client, tokens).await;
    let (image_count, failure) = match images {
        Ok(images) => {
            let count = images.len();
            for image in images {
                let metadata = build_cached_vehicle_image_metadata(
                    &image.url,
                    serde_json::json!({
                        "source":image.source,"vehicle_version":image.vehicle_version,"rivian_vehicle_id":image.vehicle_id,
                        "rivian_order_id":image.order_id,"extension":image.extension
                    }),
                );
                let _ = sqlx::query(
                    r#"
                    INSERT INTO panda.vehicle_images
                      (vehicle_id, placement, design, size, resolution, url, overlays, metadata)
                    VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                    ON CONFLICT (vehicle_id, url) DO UPDATE
                    SET placement = EXCLUDED.placement,
                        design = EXCLUDED.design,
                        size = EXCLUDED.size,
                        resolution = EXCLUDED.resolution,
                        overlays = EXCLUDED.overlays,
                        metadata = EXCLUDED.metadata,
                        updated_at = now()
                    "#,
                )
                .bind(vehicle_id)
                .bind(image.placement.as_deref().unwrap_or("unknown"))
                .bind(image.design)
                .bind(image.size)
                .bind(image.resolution)
                .bind(image.url)
                .bind(image.overlays)
                .bind(metadata)
                .execute(pool)
                .await;
            }
            mirror_vehicle_images(pool, config, vehicle_id).await;
            (count, None)
        }
        Err(error) => (0, Some(error.to_string())),
    };
    refresh_vehicle_artwork_cache_state(pool, config, vehicle_id, failure.as_deref()).await;
    match failure {
        None => {
            crate::services::external_connections::record_success(
                pool,
                crate::services::external_connections::RIVIAN_ACCOUNT,
            )
            .await;
            info!(vehicle_id = %vehicle_id, image_count, "vehicle.images.cached");
        }
        Some(error) => {
            crate::services::external_connections::record_failure(
                pool,
                crate::services::external_connections::RIVIAN_ACCOUNT,
                &error,
            )
            .await;
            warn!(vehicle_id = %vehicle_id, error = %error, "vehicle.add.image_cache_failed");
        }
    }
}

async fn ensure_vehicle_images_cached(
    pool: &sqlx::PgPool,
    config: &crate::config::Config,
    vehicle_id: Uuid,
    age_key: &str,
    force_manifest_refresh: bool,
) {
    if !crate::services::outbound_policy::optional_traffic_allowed() {
        return;
    }
    let rows = sqlx::query_as::<_, VehicleImageRow>(
        "SELECT placement, design, size, resolution, url, overlays, metadata
         FROM panda.vehicle_images
         WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let work = if rows.is_empty() || force_manifest_refresh {
        ArtworkWork::Manifest
    } else if rows
        .iter()
        .any(|row| vehicle_image_needs_local_mirror(config, row))
    {
        ArtworkWork::LocalRepair
    } else {
        ArtworkWork::Complete
    };
    match work {
        ArtworkWork::Complete => return,
        ArtworkWork::LocalRepair => {
            mark_vehicle_artwork_repair_attempt(pool, vehicle_id).await;
            mirror_vehicle_images(pool, config, vehicle_id).await;
            refresh_vehicle_artwork_cache_state(pool, config, vehicle_id, None).await;
            return;
        }
        ArtworkWork::Manifest => {}
    }
    let identity = match age_key.parse::<age::x25519::Identity>() {
        Ok(identity) => identity,
        Err(error) => {
            warn!(vehicle_id = %vehicle_id, error = %error, "vehicle.images.backfill_bad_age_key");
            return;
        }
    };

    let encrypted_tokens = match sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(tokens)) => tokens,
        Ok(None) => return,
        Err(error) => {
            warn!(vehicle_id = %vehicle_id, error = %error, "vehicle.images.backfill_credentials_failed");
            return;
        }
    };

    match crate::ingestion::session_store::decrypt_tokens(&encrypted_tokens, &identity) {
        Ok(tokens) => cache_vehicle_images(pool, config, vehicle_id, &tokens).await,
        Err(error) => {
            refresh_vehicle_artwork_cache_state(
                pool,
                config,
                vehicle_id,
                Some("Stored Rivian session could not be used for artwork repair."),
            )
            .await;
            warn!(vehicle_id = %vehicle_id, error = %error, "vehicle.images.backfill_decrypt_failed");
        }
    }
}

fn build_cached_vehicle_image_metadata(
    source_url: &str,
    base: serde_json::Value,
) -> serde_json::Value {
    // A stable first-party key lets the UI render a local restoring placeholder
    // even before an initial mirror has completed. Successful mirrors replace
    // it with a checksum-revisioned immutable key.
    let pending_key = format!(
        "{}.webp",
        hex::encode(Sha256::digest(source_url.as_bytes()))
    );
    merge_metadata(
        base,
        serde_json::json!({
            "source_url": source_url,
            "mirror_key": pending_key,
            "mirror_status": "pending"
        }),
    )
}

fn merge_metadata(mut left: serde_json::Value, right: serde_json::Value) -> serde_json::Value {
    match (&mut left, right) {
        (serde_json::Value::Object(target), serde_json::Value::Object(source)) => {
            target.extend(source);
            left
        }
        (_, replacement) => replacement,
    }
}

fn overlay_entries_from_value(value: &serde_json::Value) -> Vec<VehicleImageOverlay> {
    serde_json::from_value::<Vec<VehicleImageOverlay>>(value.clone()).unwrap_or_default()
}

fn image_cache_root(config: &crate::config::Config) -> PathBuf {
    PathBuf::from(&config.vehicle_image_cache_dir)
}

fn mirror_file_path(config: &crate::config::Config, relpath: &str) -> PathBuf {
    image_cache_root(config).join(relpath.replace('/', std::path::MAIN_SEPARATOR_STR))
}

fn sanitize_image_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
}

fn infer_extension(source_url: &str, content_type: Option<&str>) -> &'static str {
    let lowered = source_url.to_ascii_lowercase();
    for ext in ["webp", "png", "jpg", "jpeg", "gif"] {
        if lowered.contains(&format!(".{ext}")) {
            return ext;
        }
    }
    match content_type.unwrap_or_default() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "webp",
    }
}

fn guess_mime_type(path: &StdPath) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "image/webp",
    }
}

async fn download_and_store_asset(
    client: &reqwest::Client,
    config: &crate::config::Config,
    vehicle_id: Uuid,
    source_url: &str,
) -> Result<MirroredFileAsset, anyhow::Error> {
    crate::services::outbound_policy::require_optional_traffic()?;
    let upstream = client.get(source_url).send().await?.error_for_status()?;
    let mime = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .map(str::to_owned);
    let bytes = crate::services::outbound_policy::read_limited(upstream, 5 * 1024 * 1024).await?;
    let asset = MirroredFileAsset::identify(vehicle_id, source_url, &bytes, mime);
    let path = mirror_file_path(config, &asset.mirror_relpath);
    if let Some(directory) = path.parent() {
        tokio::fs::create_dir_all(directory).await?;
    }
    tokio::fs::write(path, bytes).await?;
    Ok(asset)
}

async fn mirror_vehicle_image_assets(
    client: &reqwest::Client,
    config: &crate::config::Config,
    vehicle_id: Uuid,
    source_url: &str,
    overlays: &[VehicleImageOverlay],
) -> Result<serde_json::Value, anyhow::Error> {
    let primary = download_and_store_asset(client, config, vehicle_id, source_url).await?;
    let mut manifest = serde_json::json!({
        "source_url":source_url,"mirror_status":"ready","mirror_key":primary.mirror_key,
        "mirror_relpath":primary.mirror_relpath,"mime_type":primary.mime_type,"sha256":primary.sha256,
        "overlay_mirrors":[]
    });
    let assets = manifest["overlay_mirrors"]
        .as_array_mut()
        .expect("overlay list");
    for overlay in overlays {
        let asset = download_and_store_asset(client, config, vehicle_id, &overlay.url).await?;
        assets.push(serde_json::to_value(MirroredOverlayAsset {
            source_url: overlay.url.clone(),
            mirror_key: asset.mirror_key,
            mirror_relpath: asset.mirror_relpath,
            mime_type: asset.mime_type,
            sha256: asset.sha256,
            overlay: overlay.overlay.clone(),
            z_index: overlay.z_index,
        })?);
    }
    Ok(manifest)
}

fn mirrored_asset_is_valid(config: &crate::config::Config, metadata: &serde_json::Value) -> bool {
    let expected = metadata.get("sha256").and_then(serde_json::Value::as_str);
    let actual = metadata
        .get("mirror_relpath")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| std::fs::read(mirror_file_path(config, path)).ok())
        .map(|bytes| hex::encode(Sha256::digest(bytes)));
    matches!((actual.as_deref(), expected), (Some(actual), Some(expected)) if actual == expected)
}

fn vehicle_image_needs_local_mirror(config: &crate::config::Config, row: &VehicleImageRow) -> bool {
    if !["http://", "https://"]
        .iter()
        .any(|scheme| row.url.starts_with(scheme))
    {
        return false;
    }
    let main_ready = metadata_has_local_mirror(config, &row.metadata);
    if !main_ready {
        return true;
    }
    match row
        .metadata
        .get("overlay_mirrors")
        .and_then(serde_json::Value::as_array)
    {
        Some(overlays) => !overlays
            .iter()
            .all(|overlay| mirrored_asset_is_valid(config, overlay)),
        None => !overlay_entries_from_value(&row.overlays).is_empty(),
    }
}

async fn mirror_vehicle_images(
    pool: &sqlx::PgPool,
    config: &crate::config::Config,
    vehicle_id: Uuid,
) {
    let rows = match sqlx::query_as::<_, VehicleImageRow>(
        "SELECT placement, design, size, resolution, url, overlays, metadata
         FROM panda.vehicle_images
         WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            warn!(vehicle_id = %vehicle_id, error = %error, "vehicle.images.mirror_rows_failed");
            return;
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    for row in rows
        .into_iter()
        .filter(|row| vehicle_image_needs_local_mirror(config, row))
    {
        let overlays = overlay_entries_from_value(&row.overlays);
        let revision = match mirror_vehicle_image_assets(
            &client, config, vehicle_id, &row.url, &overlays,
        )
        .await
        {
            Ok(metadata) => metadata,
            Err(error) => {
                warn!(vehicle_id = %vehicle_id, error = %error, "vehicle.images.mirror_failed");
                serde_json::json!({"source_url":row.url,"mirror_status":"failed","mirror_error":error.to_string()})
            }
        };
        let metadata = merge_metadata(row.metadata, revision);
        let _ = sqlx::query("UPDATE panda.vehicle_images SET metadata=$3,updated_at=now() WHERE vehicle_id=$1 AND url=$2")
            .bind(vehicle_id).bind(row.url).bind(metadata).execute(pool).await;
    }
}

fn metadata_has_local_mirror(config: &crate::config::Config, metadata: &serde_json::Value) -> bool {
    metadata
        .get("mirror_status")
        .and_then(|value| value.as_str())
        == Some("ready")
        && mirrored_asset_is_valid(config, metadata)
}

fn artwork_asset_counts(config: &crate::config::Config, rows: &[VehicleImageRow]) -> (i32, i32) {
    let counts = artwork::CacheInventory::inspect(config, rows);
    (counts.total, counts.ready)
}

async fn mark_vehicle_artwork_repair_attempt(pool: &sqlx::PgPool, vehicle_id: Uuid) {
    let _ = sqlx::query(
        r#"INSERT INTO panda.vehicle_artwork_cache_state
              (vehicle_id, status, attempts, last_repair_attempt_at, updated_at)
           VALUES ($1, 'repairing', 1, now(), now())
           ON CONFLICT (vehicle_id) DO UPDATE SET
             status = 'repairing', attempts = panda.vehicle_artwork_cache_state.attempts + 1,
             last_repair_attempt_at = now(), last_error = NULL, updated_at = now()"#,
    )
    .bind(vehicle_id)
    .execute(pool)
    .await;
}

async fn refresh_vehicle_artwork_cache_state(
    pool: &sqlx::PgPool,
    config: &crate::config::Config,
    vehicle_id: Uuid,
    error: Option<&str>,
) {
    let rows = sqlx::query_as::<_, VehicleImageRow>(
        "SELECT placement, design, size, resolution, url, overlays, metadata
         FROM panda.vehicle_images WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let (asset_count, ready_asset_count) = artwork_asset_counts(config, &rows);
    let status = artwork::CacheInventory {
        total: asset_count,
        ready: ready_asset_count,
    }
    .status(error.is_some());
    let sanitized_error = error.map(|_| "Rivian artwork repair failed; retry is scheduled.");
    let _ = sqlx::query(
        r#"INSERT INTO panda.vehicle_artwork_cache_state
              (vehicle_id, status, asset_count, ready_asset_count, last_repair_success_at, last_error, updated_at)
           VALUES ($1, $2, $3, $4, CASE WHEN $2 = 'ready' THEN now() ELSE NULL END, $5, now())
           ON CONFLICT (vehicle_id) DO UPDATE SET
             status = EXCLUDED.status, asset_count = EXCLUDED.asset_count,
             ready_asset_count = EXCLUDED.ready_asset_count,
             last_repair_success_at = CASE WHEN EXCLUDED.status = 'ready' THEN now() ELSE panda.vehicle_artwork_cache_state.last_repair_success_at END,
             last_error = EXCLUDED.last_error, updated_at = now()"#,
    )
    .bind(vehicle_id)
    .bind(status)
    .bind(asset_count)
    .bind(ready_asset_count)
    .bind(sanitized_error)
    .execute(pool)
    .await;
}

async fn vehicle_artwork_cache_state_json(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
) -> serde_json::Value {
    let result = sqlx::query_as::<_, VehicleArtworkCacheStateRow>(
        "SELECT status,asset_count,ready_asset_count,attempts,last_repair_attempt_at,last_repair_success_at,last_error FROM panda.vehicle_artwork_cache_state WHERE vehicle_id=$1")
        .bind(vehicle_id).fetch_optional(pool).await;
    if let Ok(Some(state)) = result {
        return serde_json::to_value(state).expect("artwork state serializes");
    }
    serde_json::json!({"status":"pending","asset_count":0,"ready_asset_count":0})
}

fn resolved_image_url(
    _config: &crate::config::Config,
    vehicle_id: Uuid,
    _original_url: &str,
    metadata: &serde_json::Value,
) -> Option<String> {
    metadata
        .get("mirror_key")?
        .as_str()
        .map(|key| format!("/v1/vehicle-image-cache/{vehicle_id}/{key}"))
}

fn resolved_overlays(
    config: &crate::config::Config,
    vehicle_id: Uuid,
    row_overlays: &serde_json::Value,
    metadata: &serde_json::Value,
) -> serde_json::Value {
    artwork::overlay_projection(config, vehicle_id, row_overlays, metadata)
}

async fn find_mirrored_asset(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    image_key: &str,
) -> Result<Option<serde_json::Value>, AppError> {
    let rows = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT metadata FROM panda.vehicle_images WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;
    let selected = rows.into_iter().find_map(|metadata| {
        if metadata
            .get("mirror_key")
            .and_then(serde_json::Value::as_str)
            == Some(image_key)
        {
            return Some(metadata);
        }
        metadata
            .get("overlay_mirrors")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|overlay| {
                overlay
                    .get("mirror_key")
                    .and_then(serde_json::Value::as_str)
                    == Some(image_key)
            })
            .cloned()
    });
    Ok(selected)
}

async fn fetch_vehicle_images_json(
    pool: &sqlx::PgPool,
    config: &crate::config::Config,
    vehicle_id: Uuid,
) -> Result<serde_json::Value, AppError> {
    let rows = sqlx::query_as::<_, VehicleImageRow>(
        r#"
        SELECT placement, design, size, resolution, url, overlays, metadata
        FROM panda.vehicle_images
        WHERE vehicle_id = $1
        ORDER BY
          CASE WHEN size = 'large' THEN 0 ELSE 1 END,
          placement,
          design NULLS LAST,
                    created_at
                "#,
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;

    let selection = artwork::ArtworkSelection::new(&rows);
    let cache = vehicle_artwork_cache_state_json(pool, vehicle_id).await;
    Ok(selection.render(config, vehicle_id, cache))
}

async fn vehicle_image_cache_asset(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, image_key)): Path<(Uuid, String)>,
) -> Result<Response, AppError> {
    if !sanitize_image_key(&image_key) {
        return Err(AppError::NotFound);
    }
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let metadata = find_mirrored_asset(&state.pool, vehicle_id, &image_key).await?;
    let asset = CachedArtwork::read(&state.config, metadata.as_ref()).await;
    if matches!(asset, CachedArtwork::Restoring) {
        queue_vehicle_artwork_repair(&state, vehicle_id).await;
    }
    Ok(asset.respond())
}

fn vehicle_artwork_placeholder_response() -> Response {
    // A compact, first-party neutral fallback for stale browser URLs while the
    // background worker repairs the persistent artwork cache.
    const PLACEHOLDER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180" role="img" aria-label="Vehicle artwork unavailable"><rect width="320" height="180" fill="#1d2430"/><path d="M54 121h212l-16-39c-4-10-13-16-24-16H94c-11 0-20 6-24 16l-16 39Z" fill="#64748b"/><circle cx="94" cy="122" r="18" fill="#0f172a"/><circle cx="226" cy="122" r="18" fill="#0f172a"/></svg>"##;
    let mut response = Response::new(Body::from(PLACEHOLDER));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

fn vehicle_artwork_restoring_response() -> Response {
    match crate::services::outbound_policy::optional_traffic_allowed() {
        false => axum::http::StatusCode::NOT_FOUND.into_response(),
        true => {
            let mut response = vehicle_artwork_placeholder_response();
            *response.status_mut() = axum::http::StatusCode::ACCEPTED;
            response.headers_mut().insert(
                HeaderName::from_static("x-panda-artwork-state"),
                HeaderValue::from_static("restoring"),
            );
            response
        }
    }
}

async fn admin_remirror_vehicle_images(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    ArtworkMaintenance::Remirror
        .run(&state, &auth, vehicle_id)
        .await
        .map(Json)
}

async fn admin_purge_vehicle_artwork_cache(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    ArtworkMaintenance::Purge
        .run(&state, &auth, vehicle_id)
        .await
        .map(Json)
}

async fn vehicle_artwork_cache_complete(
    pool: &sqlx::PgPool,
    config: &crate::config::Config,
    vehicle_id: Uuid,
) -> bool {
    let rows = sqlx::query_as::<_, VehicleImageRow>(
        "SELECT placement, design, size, resolution, url, overlays, metadata
         FROM panda.vehicle_images WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let (total, ready) = artwork_asset_counts(config, &rows);
    total > 0 && total == ready
}

async fn claim_vehicle_artwork_repair(pool: &sqlx::PgPool, vehicle_id: Uuid) -> bool {
    sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO panda.vehicle_artwork_cache_state
              (vehicle_id, status, next_attempt_at, updated_at)
           VALUES ($1, 'pending', now(), now())
           ON CONFLICT (vehicle_id) DO UPDATE SET
             status = 'pending', next_attempt_at = now(), updated_at = now()
           WHERE panda.vehicle_artwork_cache_state.status <> 'repairing'
              OR panda.vehicle_artwork_cache_state.last_repair_attempt_at < now() - interval '5 minutes'
           RETURNING vehicle_id"#,
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .is_some()
}

async fn queue_vehicle_artwork_repair(state: &AppState, vehicle_id: Uuid) {
    queue_vehicle_artwork_repair_with_mode(state, vehicle_id, false).await;
}

async fn queue_vehicle_artwork_repair_with_mode(
    state: &AppState,
    vehicle_id: Uuid,
    force_manifest_refresh: bool,
) {
    if !crate::services::outbound_policy::optional_traffic_allowed() {
        return;
    }
    let is_demo =
        sqlx::query_scalar::<_, String>("SELECT rivian_vehicle_id FROM panda.vehicles WHERE id=$1")
            .bind(vehicle_id)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .is_some_and(|key| is_demo_vehicle_key(&key));
    if is_demo {
        return;
    }
    if let Some(job) = ArtworkJob::claim(
        &state.pool,
        &state.config,
        &state.age_key,
        vehicle_id,
        force_manifest_refresh,
    )
    .await
    {
        tokio::spawn(job.run());
    }
}

async fn repair_vehicle_artwork(
    pool: sqlx::PgPool,
    config: crate::config::Config,
    age_key: String,
    vehicle_id: Uuid,
    force_manifest_refresh: bool,
) {
    let mut complete = false;
    for delay in [0, 5, 10] {
        if delay != 0 {
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
        ensure_vehicle_images_cached(&pool, &config, vehicle_id, &age_key, force_manifest_refresh)
            .await;
        complete = vehicle_artwork_cache_complete(&pool, &config, vehicle_id).await;
        if complete {
            break;
        }
    }
    refresh_vehicle_artwork_cache_state(
        &pool,
        &config,
        vehicle_id,
        (!complete).then_some("Artwork cache is incomplete."),
    )
    .await;
}

/// Starts a bounded repair pass after the API has loaded its encrypted vehicle
/// credentials. Browser requests only read local files; they never trigger this.
pub fn start_vehicle_artwork_repair_worker(
    pool: sqlx::PgPool,
    config: crate::config::Config,
    age_key: String,
) {
    if !crate::services::outbound_policy::optional_traffic_allowed() {
        return;
    }
    tokio::spawn(async move {
        let vehicles = sqlx::query_scalar::<_, Uuid>("SELECT v.id FROM panda.vehicles v JOIN panda.vehicle_credentials c ON c.vehicle_id=v.id WHERE v.rivian_vehicle_id NOT LIKE 'demo-%'")
            .fetch_all(&pool).await.unwrap_or_default();
        for vehicle in vehicles {
            if let Some(job) = ArtworkJob::claim(&pool, &config, &age_key, vehicle, false).await {
                job.run().await;
            }
        }
    });
}

fn normalize_image_placement(value: &str) -> &'static str {
    let normalized = value.to_lowercase();
    if normalized.contains("side") && !is_charging_side_value(value) {
        "side"
    } else if normalized.contains("overhead")
        || normalized.contains("top")
        || normalized.contains("bird")
    {
        "overhead"
    } else if normalized.contains("front") {
        "front"
    } else if normalized.contains("rear") || normalized.contains("back") {
        "rear"
    } else {
        "unknown"
    }
}

fn is_charging_side_value(value: &str) -> bool {
    let normalized = value.to_lowercase();
    normalized.contains("side")
        && (normalized.contains("charging") || normalized.contains("charge"))
}

fn normalize_image_design(value: Option<&str>) -> &'static str {
    let Some(value) = value else {
        return "unknown";
    };
    let normalized = value.to_lowercase();
    if normalized.contains("dark") {
        "dark"
    } else if normalized.contains("light") {
        "light"
    } else {
        "unknown"
    }
}

fn open_closures_json(closures: &[(&str, Option<bool>)]) -> serde_json::Value {
    let mut open = None;
    for (label, closed) in closures {
        if let Some(closed) = closed {
            let values = open.get_or_insert_with(Vec::new);
            if !closed {
                values.push(serde_json::Value::String((*label).to_owned()));
            }
        }
    }
    open.map(serde_json::Value::Array).unwrap_or_default()
}

#[cfg(test)]
static TELEMETRY_LANES_QUERY: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(lane_diagnostics::full_query);

async fn telemetry_lanes(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Query(params): Query<TelemetryLaneParams>,
) -> Result<Json<TelemetryLaneFrame>, AppError> {
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    let plan = lane_diagnostics::LanePlan::compile(params)?;
    Ok(Json(plan.read(&state.pool, vid).await?))
}

#[cfg(test)]
fn parse_telemetry_lanes(value: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    lane_diagnostics::names(value)
}

#[cfg(test)]
fn resolve_telemetry_resolution(
    value: Option<&str>,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    max_points: i64,
) -> Result<i64, AppError> {
    lane_diagnostics::resolution(value, from, to, max_points)
}

async fn raw_vehicle_data(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Query(params): Query<RawDataParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    let plan = raw_diagnostics::RawPlan::compile(params)?;
    Ok(Json(plan.read(&state.pool, vid).await?))
}

async fn raw_vehicle_events(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(vid): axum::extract::Path<Uuid>,
    Query(params): Query<RawEventParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_raw_event_access(&state, &auth, vid).await?;
    let page = RawEventPage::compile(params)?;
    let (items, total) = page.read(&state.pool, vid).await?;
    Ok(Json(serde_json::json!({
        "vehicle_id":vid,"retention_days":state.config.rivian_raw_event_retention_days.max(1),
        "items":items.into_iter().map(raw_event_summary_json).collect::<Vec<_>>(),
        "total":total,"page":page.page,"per_page":page.per_page
    })))
}

async fn raw_vehicle_event(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path((vid, event_id)): axum::extract::Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_raw_event_access(&state, &auth, vid).await?;
    let row = sqlx::query_as::<_, RawEventDetailRow>(
        r#"
        SELECT id, received_at, event_type, message_type, payload_json, payload_text
        FROM panda.rivian_ws_raw_events
        WHERE id = $1 AND vehicle_id = $2
        "#,
    )
    .bind(event_id)
    .bind(vid)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let has_json = row.payload_json.is_some();
    let has_text = row.payload_text.is_some();
    let mut value = serde_json::to_value(row).expect("event detail serializes");
    let fields = value.as_object_mut().expect("event detail is an object");
    let json = fields.remove("payload_json").unwrap_or_default();
    let text = fields.remove("payload_text").unwrap_or_default();
    let (payload, format) = if has_json {
        (json, "json")
    } else if has_text {
        (text, "text")
    } else {
        (serde_json::Value::Null, "empty")
    };
    fields.extend([
        ("payload".into(), payload),
        ("payload_format".into(), serde_json::json!(format)),
        ("has_json".into(), serde_json::json!(format == "json")),
        ("has_payload".into(), serde_json::json!(format != "empty")),
    ]);
    Ok(Json(value))
}

async fn require_raw_event_access(
    state: &AppState,
    auth: &AuthUser,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    if auth.api_access_level.is_some() {
        return Err(AppError::Forbidden);
    }
    require_vehicle_role(&state.pool, auth.user_id, vehicle_id, &["owner", "manager"]).await
}

fn validate_raw_time_bounds(
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), AppError> {
    if from.zip(to).is_some_and(|(from, to)| from > to) {
        return Err(AppError::Validation("from must be before to".into()));
    }
    Ok(())
}

fn parse_raw_fields(fields: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    raw_diagnostics::select_fields(fields)
}

fn raw_telemetry_where_clause(selected_fields: &[&str], populated_only: bool) -> String {
    raw_diagnostics::predicate(selected_fields, populated_only)
}

fn raw_field_coverage_query(where_clause: &str) -> String {
    raw_diagnostics::field_coverage_query(where_clause)
}

fn raw_sample_json(r: RawVehicleSampleRow) -> serde_json::Value {
    raw_diagnostics::project_sample(r)
}

fn raw_event_summary_json(row: RawEventSummaryRow) -> serde_json::Value {
    serde_json::to_value(row).expect("raw event summary serializes")
}

#[cfg(test)]
mod telemetry_tests;
#[cfg(test)]
mod tests;

impl VehicleListRow {
    fn into_listing(self, user_id: Uuid, now: chrono::DateTime<Utc>) -> serde_json::Value {
        let is_demo = is_demo_vehicle_key(&self.rivian_vehicle_id);
        let issued = if is_demo { None } else { self.token_created_at };
        let renewal =
            issued.map(|at| rivian_renewal_state_for_auth(at, now, self.auth_state.as_deref()));
        let display_name = match self.name.as_deref() {
            Some("Demo R2S") if is_demo => "Demo R2",
            Some(name) => name,
            None => canonical_vehicle_model(&self.model),
        }
        .to_owned();
        let model = canonical_vehicle_model(&self.model).to_owned();
        let capacity = self.battery_capacity_wh.map(|wh| wh / 1000.0);
        let mut value = serde_json::to_value(self).expect("vehicle listing serializes");
        let object = value.as_object_mut().expect("vehicle listing is an object");
        for field in ["name", "battery_capacity_wh", "token_created_at"] {
            object.remove(field);
        }
        object.extend([
            ("user_id".into(), serde_json::json!(user_id)),
            ("is_demo".into(), serde_json::json!(is_demo)),
            ("model".into(), serde_json::json!(model)),
            ("year".into(), serde_json::Value::Null),
            ("battery_capacity_kwh".into(), serde_json::json!(capacity)),
            ("display_name".into(), serde_json::json!(display_name)),
            ("credential_issued_at".into(), serde_json::json!(issued)),
            (
                "expected_renewal_at".into(),
                serde_json::json!(issued.map(|at| at + RIVIAN_RENEWAL_INTERVAL)),
            ),
            ("renewal_state".into(), serde_json::json!(renewal)),
        ]);
        value
    }
}

enum CaptureAction {
    Inspect,
    Start,
    Stop,
}
impl CaptureAction {
    async fn run(
        self,
        state: &AppState,
        auth: &AuthUser,
        vehicle_id: Uuid,
    ) -> Result<ingestion_capture::CaptureStatus, AppError> {
        require_vehicle_access(auth, vehicle_id)?;
        require_raw_event_access(state, auth, vehicle_id).await?;
        match self {
            Self::Inspect => {}
            Self::Start => {
                require_remote_backed_vehicle(&state.pool, vehicle_id).await?;
                ingestion_capture::start(&state.pool, vehicle_id, auth.user_id)
                    .await
                    .map_err(AppError::Internal)?;
            }
            Self::Stop => {
                ingestion_capture::stop(
                    &state.pool,
                    vehicle_id,
                    ingestion_capture::StopReason::User,
                )
                .await
                .map_err(AppError::Internal)?;
            }
        }
        ingestion_capture::status(&state.pool, vehicle_id)
            .await
            .map_err(AppError::from)
    }
}

struct VehicleSettingsUpdate {
    capacity_wh: Option<f64>,
    battery_config: Option<String>,
    pressure: Option<f64>,
}
impl VehicleSettingsUpdate {
    fn validate(body: UpdateBatteryConfigBody) -> Result<Self, AppError> {
        if let Some(pressure) = body.target_tire_pressure_psi {
            for (rejected, message) in [
                (
                    !pressure.is_finite(),
                    "target_tire_pressure_psi must be a finite number",
                ),
                (
                    !(20.0..=80.0).contains(&pressure),
                    "target_tire_pressure_psi must be between 20 and 80",
                ),
            ] {
                if rejected {
                    return Err(AppError::Validation(message.into()));
                }
            }
        }
        Ok(Self {
            capacity_wh: body.battery_capacity_kwh.map(|kwh| kwh * 1000.0),
            battery_config: body
                .battery_config
                .map(|value| canonical_battery_config(&value).to_owned()),
            pressure: body.target_tire_pressure_psi,
        })
    }
    async fn persist(self, pool: &sqlx::PgPool, vehicle: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE panda.vehicles SET battery_capacity_wh=COALESCE($2,battery_capacity_wh), battery_config=COALESCE($3,battery_config), target_tire_pressure_psi=COALESCE($4,target_tire_pressure_psi) WHERE id=$1")
            .bind(vehicle).bind(self.capacity_wh).bind(self.battery_config).bind(self.pressure).execute(pool).await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow, Serialize)]
struct InvitationPreview {
    id: Uuid,
    vehicle_id: Uuid,
    vehicle_name: String,
    invitee_email: String,
    role: String,
    expires_at: chrono::DateTime<Utc>,
    accepted_at: Option<chrono::DateTime<Utc>>,
    revoked_at: Option<chrono::DateTime<Utc>>,
}

impl MirroredFileAsset {
    fn identify(vehicle: Uuid, source: &str, bytes: &[u8], mime: Option<String>) -> Self {
        let sha256 = hex::encode(Sha256::digest(bytes));
        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        let extension = infer_extension(source, mime.as_deref());
        let mirror_key = format!("{source_hash}-{}.{extension}", &sha256[..16]);
        let mirror_relpath = format!("{vehicle}/{mirror_key}");
        let mime_type =
            mime.unwrap_or_else(|| guess_mime_type(StdPath::new(&mirror_relpath)).to_owned());
        Self {
            mirror_key,
            mirror_relpath,
            mime_type,
            sha256,
        }
    }
}

enum CachedArtwork {
    Missing,
    Restoring,
    Ready { bytes: Vec<u8>, mime: String },
}
impl CachedArtwork {
    async fn read(config: &crate::config::Config, metadata: Option<&serde_json::Value>) -> Self {
        let Some(metadata) = metadata else {
            return Self::Missing;
        };
        let Some(path) = metadata
            .get("mirror_relpath")
            .and_then(serde_json::Value::as_str)
        else {
            return Self::Restoring;
        };
        let bytes = match tokio::fs::read(mirror_file_path(config, path)).await {
            Ok(bytes) => bytes,
            Err(_) => return Self::Restoring,
        };
        let Some(expected) = metadata.get("sha256").and_then(serde_json::Value::as_str) else {
            return Self::Missing;
        };
        if expected != hex::encode(Sha256::digest(&bytes)) {
            return Self::Restoring;
        }
        Self::Ready {
            bytes,
            mime: metadata
                .get("mime_type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_owned(),
        }
    }
    fn respond(self) -> Response {
        match self {
            Self::Missing => vehicle_artwork_placeholder_response(),
            Self::Restoring => vehicle_artwork_restoring_response(),
            Self::Ready { bytes, mime } => {
                let mut response = Response::new(Body::from(bytes));
                response.headers_mut().extend([
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_str(&mime).unwrap_or_else(|_| {
                            HeaderValue::from_static("application/octet-stream")
                        }),
                    ),
                    (
                        header::CACHE_CONTROL,
                        HeaderValue::from_static("private, max-age=31536000, immutable"),
                    ),
                ]);
                response
            }
        }
    }
}

enum ArtworkWork {
    Complete,
    LocalRepair,
    Manifest,
}

#[derive(Clone, Copy)]
enum RivianLoginPhase {
    Password,
    Verification,
}
impl RivianLoginPhase {
    fn translate(self, error: RivianAuthError) -> AppError {
        match (self, error) {
            (Self::Password, RivianAuthError::InvalidCredentials) => {
                AppError::RivianCredentialsRejected
            }
            (Self::Verification, RivianAuthError::InvalidCredentials) => {
                AppError::RivianConnectSessionExpired
            }
            (Self::Verification, RivianAuthError::InvalidOtp) => AppError::RivianOtpRejected,
            (phase, RivianAuthError::Network(error)) => {
                match phase {
                    Self::Password => warn!(error = %error, "vehicle.connect.rivian_network_error"),
                    Self::Verification => {
                        warn!(error = %error, "vehicle.connect_otp.rivian_network_error")
                    }
                }
                AppError::RivianApi("Unable to reach Rivian. Please try again shortly.".into())
            }
            (phase, error) => {
                match error {
                    RivianAuthError::UnexpectedResponse(error) => match phase {
                        Self::Password => {
                            warn!(error = %error, "vehicle.connect.rivian_unexpected_response")
                        }
                        Self::Verification => {
                            warn!(error = %error, "vehicle.connect_otp.rivian_unexpected_response")
                        }
                    },
                    _ => warn!("vehicle.connect.login_returned_otp_error"),
                }
                let message = match phase {
                    Self::Password => "Rivian returned an unexpected sign-in response. Please try again shortly.",
                    Self::Verification => "Rivian returned an unexpected verification response. Please try again shortly.",
                };
                AppError::RivianApi(message.into())
            }
        }
    }
}

struct RawEventPage {
    params: RawEventParams,
    page: i64,
    per_page: i64,
}
impl RawEventPage {
    fn compile(mut params: RawEventParams) -> Result<Self, AppError> {
        validate_raw_time_bounds(params.from, params.to)?;
        for filter in [&mut params.event_type, &mut params.message_type] {
            *filter = filter
                .take()
                .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_owned()));
        }
        Ok(Self {
            page: params.page.unwrap_or(1).max(1),
            per_page: params.per_page.unwrap_or(25).clamp(1, 100),
            params,
        })
    }
    async fn read(
        &self,
        pool: &sqlx::PgPool,
        vehicle: Uuid,
    ) -> Result<(Vec<RawEventSummaryRow>, i64), AppError> {
        const PREDICATE: &str = "FROM panda.rivian_ws_raw_events WHERE vehicle_id=$1 AND ($2::timestamptz IS NULL OR received_at >= $2) AND ($3::timestamptz IS NULL OR received_at <= $3) AND ($4::text IS NULL OR event_type=$4) AND ($5::text IS NULL OR message_type=$5)";
        let items_sql = format!("SELECT id,received_at,event_type,message_type,payload_json IS NOT NULL AS has_json,(payload_json IS NOT NULL OR payload_text IS NOT NULL) AS has_payload {PREDICATE} ORDER BY received_at DESC LIMIT $6 OFFSET $7");
        let count_sql = format!("SELECT COUNT(*)::BIGINT {PREDICATE}");
        let offset = (self.page - 1).saturating_mul(self.per_page);
        let items = sqlx::query_as::<_, RawEventSummaryRow>(sqlx::AssertSqlSafe(items_sql))
            .bind(vehicle)
            .bind(self.params.from)
            .bind(self.params.to)
            .bind(self.params.event_type.as_deref())
            .bind(self.params.message_type.as_deref())
            .bind(self.per_page)
            .bind(offset)
            .fetch_all(pool)
            .await?;
        let total = sqlx::query_scalar(sqlx::AssertSqlSafe(count_sql))
            .bind(vehicle)
            .bind(self.params.from)
            .bind(self.params.to)
            .bind(self.params.event_type.as_deref())
            .bind(self.params.message_type.as_deref())
            .fetch_one(pool)
            .await?;
        Ok((items, total))
    }
}

enum ArtworkMaintenance {
    Remirror,
    Purge,
}
impl ArtworkMaintenance {
    async fn run(
        self,
        state: &AppState,
        auth: &AuthUser,
        vehicle: Uuid,
    ) -> Result<serde_json::Value, AppError> {
        require_admin_or_super_user(&state.pool, auth.user_id).await?;
        require_remote_backed_vehicle(&state.pool, vehicle).await?;
        let mut result = serde_json::json!({"ok":true,"vehicle_id":vehicle});
        match self {
            Self::Remirror => {
                crate::services::outbound_policy::require_optional_traffic()
                    .map_err(|_| AppError::ExternalConnectionDisabled("vehicle artwork".into()))?;
                queue_vehicle_artwork_repair_with_mode(state, vehicle, true).await;
                result["queued"] = serde_json::Value::Bool(true);
            }
            Self::Purge => {
                let _ = tokio::fs::remove_dir_all(
                    image_cache_root(&state.config).join(vehicle.to_string()),
                )
                .await;
                sqlx::query("UPDATE panda.vehicle_images SET metadata=jsonb_set(metadata,'{mirror_status}','\"pending\"'::jsonb,true),updated_at=now() WHERE vehicle_id=$1")
                    .bind(vehicle).execute(&state.pool).await?;
                refresh_vehicle_artwork_cache_state(&state.pool, &state.config, vehicle, None)
                    .await;
            }
        }
        Ok(result)
    }
}

struct ArtworkJob {
    pool: sqlx::PgPool,
    config: crate::config::Config,
    age_key: String,
    vehicle: Uuid,
    refresh: bool,
}
impl ArtworkJob {
    async fn claim(
        pool: &sqlx::PgPool,
        config: &crate::config::Config,
        age_key: &str,
        vehicle: Uuid,
        refresh: bool,
    ) -> Option<Self> {
        if !refresh && vehicle_artwork_cache_complete(pool, config, vehicle).await {
            return None;
        }
        if !claim_vehicle_artwork_repair(pool, vehicle).await {
            return None;
        }
        Some(Self {
            pool: pool.clone(),
            config: config.clone(),
            age_key: age_key.to_owned(),
            vehicle,
            refresh,
        })
    }
    async fn run(self) {
        repair_vehicle_artwork(
            self.pool,
            self.config,
            self.age_key,
            self.vehicle,
            self.refresh,
        )
        .await;
    }
}

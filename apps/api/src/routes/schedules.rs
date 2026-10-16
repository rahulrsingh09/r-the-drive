//! Read-only schedule and enrichment routes. Vehicle writes return 403.
//!
//! Routes:
//!   GET  /v1/vehicles/{id}/charging-schedule
//!   PUT  /v1/vehicles/{id}/charging-schedule
//!   GET  /v1/vehicles/{id}/departure-schedules
//!   POST /v1/vehicles/{id}/departure-schedules
//!   PATCH /v1/vehicles/{id}/departure-schedules/{schedule_id}
//!   DELETE /v1/vehicles/{id}/departure-schedules/{schedule_id}
//!   GET  /v1/vehicles/{id}/wallboxes
//!   GET  /v1/vehicles/{id}/ota-details

use axum::{
    extract::{Path, State},
    routing::{get, patch},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/vehicles/{id}/charging-schedule",
            get(get_charging_schedule).put(reject_vehicle_write),
        )
        .route(
            "/vehicles/{id}/departure-schedules",
            get(list_departure_schedules).post(reject_vehicle_write),
        )
        .route(
            "/vehicles/{id}/departure-schedules/{schedule_id}",
            patch(reject_vehicle_write).delete(reject_vehicle_write),
        )
        .route("/vehicles/{id}/wallboxes", get(list_wallboxes))
        .route("/vehicles/{id}/ota-details", get(get_ota_details))
}

async fn reject_vehicle_write(_auth: AuthUser) -> Result<(), AppError> {
    Err(AppError::Forbidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn outbound_policy_returns_forbidden_for_authenticated_vehicle_writes() {
        let auth = AuthUser {
            user_id: Uuid::new_v4(),
            default_vehicle_id: Some(Uuid::new_v4()),
            api_access_level: None,
            api_vehicle_id: None,
        };
        let response = reject_vehicle_write(auth)
            .await
            .unwrap_err()
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }
}

// ── GET /v1/vehicles/{id}/charging-schedule ───────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
struct ChargingScheduleRow {
    id: Uuid,
    enabled: bool,
    start_time_minutes: Option<i32>,
    duration_minutes: Option<i32>,
    amperage: Option<f64>,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
    week_days: Option<Vec<String>>,
    rivian_updated_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

async fn get_charging_schedule(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<Option<ChargingScheduleRow>>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let row = sqlx::query_as::<_, ChargingScheduleRow>(
        "SELECT id, enabled, start_time_minutes, duration_minutes, amperage,
                location_lat, location_lng, week_days, rivian_updated_at, updated_at
         FROM panda.charging_schedules
         WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(Json(row))
}

// ── GET /v1/vehicles/{id}/departure-schedules ─────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
struct DepartureScheduleRow {
    id: Uuid,
    rivian_schedule_id: String,
    name: Option<String>,
    enabled: bool,
    occurrence: Option<serde_json::Value>,
    comfort_settings: Option<serde_json::Value>,
    updated_at: DateTime<Utc>,
}

async fn list_departure_schedules(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<Vec<DepartureScheduleRow>>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let rows = sqlx::query_as::<_, DepartureScheduleRow>(
        "SELECT id, rivian_schedule_id, name, enabled, occurrence, comfort_settings, updated_at
         FROM panda.departure_schedules
         WHERE vehicle_id = $1
         ORDER BY created_at",
    )
    .bind(vehicle_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows))
}

// ── GET /v1/vehicles/{id}/wallboxes ───────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
struct WallboxRow {
    id: Uuid,
    rivian_wallbox_id: String,
    name: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    max_power_kw: Option<f64>,
    model: Option<String>,
    serial_number: Option<String>,
    firmware_version: Option<String>,
    linked: Option<bool>,
    updated_at: DateTime<Utc>,
}

async fn list_wallboxes(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<Vec<WallboxRow>>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let rows = sqlx::query_as::<_, WallboxRow>(
        "SELECT w.id, w.rivian_wallbox_id, w.name, w.latitude, w.longitude,
                w.max_power_kw, w.model, w.serial_number, w.firmware_version,
                w.linked, w.updated_at
         FROM panda.wallboxes w
         JOIN panda.vehicles v ON v.user_id = w.user_id
         WHERE v.id = $1
         ORDER BY w.created_at",
    )
    .bind(vehicle_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows))
}

// ── GET /v1/vehicles/{id}/ota-details ─────────────────────────────────────────

#[derive(Debug, Serialize, sqlx::FromRow)]
struct OtaDetailsRow {
    ota_current_version: Option<String>,
    ota_available_version: Option<String>,
    ota_release_notes_url: Option<String>,
}

async fn get_ota_details(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<OtaDetailsRow>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    // Latest versions from telemetry + notes URL from vehicles table.
    let row = sqlx::query_as::<_, OtaDetailsRow>(
        "SELECT
             t.ota_current_version,
             t.ota_available_version,
             v.ota_release_notes_url
         FROM panda.vehicles v
         LEFT JOIN LATERAL (
             SELECT ota_current_version, ota_available_version
             FROM timeseries.telemetry
             WHERE vehicle_id = $1
               AND (ota_current_version IS NOT NULL OR ota_available_version IS NOT NULL)
             ORDER BY ts DESC
             LIMIT 1
         ) t ON true
         WHERE v.id = $1",
    )
    .bind(vehicle_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(row))
}

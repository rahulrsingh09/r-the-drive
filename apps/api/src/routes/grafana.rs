//! Grafana SimpleJSON datasource for Panda telemetry.
//!
//! Implements the Grafana SimpleJSON protocol:
//! <https://grafana.com/grafana/plugins/grafana-simple-json-datasource/>
//!
//! Endpoints:
//!   GET  /v1/grafana          — health check
//!   POST /v1/grafana/search   — list available metrics
//!   POST /v1/grafana/query    — time series data
//!   POST /v1/grafana/annotations — stub (empty)
//!   POST /v1/grafana/tag-keys — stub (empty)
//!   POST /v1/grafana/tag-values — stub (empty)

mod plan;

use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
};

/// All telemetry columns that Grafana can request.
/// IMPORTANT: This is a security allowlist. Never interpolate column names
/// into SQL without checking against this list first.
const ALLOWED_METRICS: &[&str] = &[
    "battery_level",
    "battery_capacity_wh",
    "power_kw",
    "regen_power_kw",
    "speed_mph",
    "outside_temp_c",
    "cabin_temp_c",
    "odometer_miles",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/grafana", get(health_check))
        .route("/grafana/search", post(search))
        .route("/grafana/query", post(query))
        .route("/grafana/annotations", post(annotations))
        .route("/grafana/tag-keys", post(tag_keys))
        .route("/grafana/tag-values", post(tag_values))
        .layer(DefaultBodyLimit::max(64 * 1024))
}

/// Grafana health check — returns 200 OK with empty body.
async fn health_check(_auth: AuthUser) -> impl IntoResponse {
    (StatusCode::OK, "")
}

// ── Search ────────────────────────────────────────────────────────────────────

/// Returns the list of available metric names.
async fn search(_auth: AuthUser) -> Json<Vec<&'static str>> {
    Json(ALLOWED_METRICS.to_vec())
}

// ── Query ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GrafanaQueryBody {
    range: GrafanaRange,
    targets: Vec<GrafanaTarget>,
    #[serde(rename = "maxDataPoints")]
    max_data_points: Option<i64>,
}

#[derive(Deserialize)]
struct GrafanaRange {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

#[derive(Deserialize)]
struct GrafanaTarget {
    /// Metric name (must be in ALLOWED_METRICS).
    target: String,
    /// Vehicle UUID — required to scope the query.
    #[serde(rename = "vehicleId")]
    vehicle_id: Option<Uuid>,
}

#[derive(Serialize)]
struct TimeSeriesResult {
    target: String,
    datapoints: Vec<[f64; 2]>,
}

#[derive(Deserialize)]
struct VehicleIdQuery {
    vehicle_id: Option<Uuid>,
}

async fn query(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(qp): Query<VehicleIdQuery>,
    Json(body): Json<GrafanaQueryBody>,
) -> Result<Json<Vec<TimeSeriesResult>>, AppError> {
    let plan = plan::ReadPlan::admit(&state, &auth, qp.vehicle_id, body).await?;
    let permit = state
        .resources
        .heavy(auth.user_id, &state.config.security)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(state.config.security.metrics_timeout_seconds),
        plan.execute(&state, permit),
    )
    .await
    .map_err(|_| AppError::DependencyUnavailable("Grafana query deadline exceeded".into()))?
}

// ── Stubs ─────────────────────────────────────────────────────────────────────

async fn annotations(_auth: AuthUser) -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

async fn tag_keys(_auth: AuthUser) -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

async fn tag_values(_auth: AuthUser) -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

#[cfg(test)]
#[path = "__tests__/operational_read_models.rs"]
mod operational_contracts;

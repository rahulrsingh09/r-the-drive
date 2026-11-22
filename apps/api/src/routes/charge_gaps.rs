//! Probable missed charges: battery rises while parked with no charge session.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::{
        app_settings,
        charge_gaps::{self, ChargeGapReport},
    },
};

/// Listing scans 30 days of telemetry, so the router mounts under the
/// heavy-read limit.
pub fn router() -> Router<AppState> {
    Router::new().route("/vehicles/{vehicle_id}/charge-gaps", get(list_charge_gaps))
}

async fn list_charge_gaps(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<ChargeGapReport>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let timezone = app_settings::load_app_timezone_name(&state.pool).await?;
    Ok(Json(
        charge_gaps::find_charge_gaps(&state.pool, vehicle_id, &timezone).await?,
    ))
}

#[cfg(test)]
#[path = "charge_gaps_authorization_tests.rs"]
mod authorization_tests;

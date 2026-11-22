//! Missed-trip detection and owner/manager recovery.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    db::vehicles::{require_vehicle_manager_access, require_vehicle_read_access},
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::trip_gaps::{self, ParallaxQuiet, RecoverError, TripGap},
};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/vehicles/{vehicle_id}/trip-gaps/{gap_id}/recover",
        post(recover_gap),
    )
}

/// Listing replays a week of telemetry, so it uses the heavy-read limit.
pub fn read_router() -> Router<AppState> {
    Router::new().route("/vehicles/{vehicle_id}/trip-gaps", get(list_gaps))
}

#[derive(Serialize)]
struct TripGapsResponse {
    vehicle_id: Uuid,
    lookback_days: i64,
    gaps: Vec<TripGap>,
    parallax_quiet: Option<ParallaxQuiet>,
}

#[derive(Serialize)]
struct RecoveredTrip {
    trip_id: Uuid,
}

async fn list_gaps(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<TripGapsResponse>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let (gaps, parallax_quiet) = tokio::try_join!(
        trip_gaps::find_trip_gaps(&state.pool, vehicle_id),
        trip_gaps::parallax_quiet_while_moving(&state.pool, vehicle_id),
    )?;
    Ok(Json(TripGapsResponse {
        vehicle_id,
        lookback_days: trip_gaps::LOOKBACK_DAYS,
        gaps,
        parallax_quiet,
    }))
}

async fn recover_gap(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((vehicle_id, gap_id)): Path<(Uuid, String)>,
) -> Result<Json<RecoveredTrip>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    if !trip_gaps::is_gap_id(&gap_id) {
        return Err(AppError::Validation("invalid gap id".into()));
    }
    match trip_gaps::recover_trip_gap(&state.pool, vehicle_id, &gap_id).await {
        Ok(trip_id) => {
            tracing::info!(%vehicle_id, %trip_id, "missed trip recovered");
            Ok(Json(RecoveredTrip { trip_id }))
        }
        Err(RecoverError::NotFound) => Err(AppError::NotFound),
        Err(RecoverError::Busy) => Err(AppError::Conflict(
            "A live trip for this vehicle is still being saved; try again in a minute".into(),
        )),
        Err(RecoverError::Failed(error)) => Err(AppError::Internal(error)),
    }
}

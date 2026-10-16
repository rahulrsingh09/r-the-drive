//! Dashboard overview endpoint — a single aggregated tile payload for the
//! main dashboard screen.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{require_vehicle_access, AppState, AuthUser},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/dashboard/overview/{vehicle_id}", get(overview))
}

#[derive(Serialize)]
struct OverviewResponse {
    vehicle_id: Uuid,
    generated_at: DateTime<Utc>,
    live: Option<LiveTile>,
    last_trip: Option<TripTile>,
    last_charge: Option<ChargeTile>,
    today: TodayTile,
}

#[derive(Deserialize, Serialize, sqlx::FromRow)]
struct LiveTile {
    battery_level: Option<f64>,
    distance_to_empty_mi: Option<f64>,
    power_state: Option<String>,
    odometer_miles: Option<f64>,
    outside_temp_c: Option<f64>,
    ts: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Serialize, sqlx::FromRow)]
struct TripTile {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    distance_miles: Option<f64>,
    efficiency_wh_per_mile: Option<f64>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
}

#[derive(Deserialize, Serialize, sqlx::FromRow)]
struct ChargeTile {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    energy_added_wh: Option<f64>,
    charger_type: Option<String>,
}

#[derive(Serialize)]
struct TodayTile {
    date: NaiveDate,
    miles_driven: Option<f64>,
}

async fn overview(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<OverviewResponse>, AppError> {
    require_vehicle_access(&auth, vehicle_id)?;
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let evidence = OverviewEvidence::read(&state.pool, vehicle_id).await?;

    Ok(Json(OverviewResponse {
        vehicle_id,
        generated_at: Utc::now(),
        live: evidence.live.map(|value| value.0),
        last_trip: evidence.last_trip.map(|value| value.0),
        last_charge: evidence.last_charge.map(|value| value.0),
        today: TodayTile {
            date: Utc::now().date_naive(),
            miles_driven: evidence.today_miles,
        },
    }))
}

#[derive(sqlx::FromRow)]
struct OverviewEvidence {
    live: Option<sqlx::types::Json<LiveTile>>,
    last_trip: Option<sqlx::types::Json<TripTile>>,
    last_charge: Option<sqlx::types::Json<ChargeTile>>,
    today_miles: Option<f64>,
}

impl OverviewEvidence {
    async fn read(pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Self, AppError> {
        Ok(sqlx::query_as::<_, Self>(
            "SELECT
             (SELECT to_jsonb(live) FROM (
                 SELECT battery_level,distance_to_empty_mi,power_state,odometer_miles,outside_temp_c,ts
                 FROM timeseries.telemetry WHERE vehicle_id=$1 ORDER BY ts DESC LIMIT 1
             ) live) AS live,
             (SELECT to_jsonb(trip) FROM (
                 SELECT id,started_at,ended_at,distance_miles,efficiency_wh_per_mile,soc_start,soc_end
                 FROM panda.trips WHERE vehicle_id=$1 ORDER BY ended_at DESC LIMIT 1
             ) trip) AS last_trip,
             (SELECT to_jsonb(charge) FROM (
                 SELECT id,started_at,ended_at,soc_start,soc_end,energy_added_wh,charger_type
                 FROM panda.charge_sessions WHERE vehicle_id=$1 ORDER BY started_at DESC LIMIT 1
             ) charge) AS last_charge,
             (SELECT miles_driven FROM timeseries.odometer_daily
                 WHERE vehicle_id=$1 AND day=date_trunc('day',now())::timestamptz LIMIT 1
             ) AS today_miles"
        ).bind(vehicle).fetch_one(pool).await?)
    }
}

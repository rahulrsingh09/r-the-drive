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
    middleware::auth::{AppState, AuthUser},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/vehicles/{vehicle_id}/parked-energy", get(parked_energy))
}

#[derive(Deserialize)]
struct ParkedEnergyParams {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct ParkedEnergyResponse {
    vehicle_id: Uuid,
    generated_at: DateTime<Utc>,
    source: &'static str,
    samples: Vec<ParkedEnergySample>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ParkedEnergySample {
    window: String,
    source_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
    parked_started_at: Option<DateTime<Utc>>,
    duration_minutes: Option<i32>,
    total_kwh: Option<f64>,
    vehicle_systems_kwh: Option<f64>,
    outlets_kwh: Option<f64>,
    climate_kwh: Option<f64>,
    gear_guard_kwh: Option<f64>,
    total_range_impact_km: Option<f64>,
    vehicle_systems_range_impact_km: Option<f64>,
    outlets_range_impact_km: Option<f64>,
    climate_range_impact_km: Option<f64>,
    gear_guard_range_impact_km: Option<f64>,
}

async fn parked_energy(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
    Query(params): Query<ParkedEnergyParams>,
) -> Result<Json<ParkedEnergyResponse>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let plan = ParkedEvidencePlan::resolve(params);
    let samples = plan.read(&state.pool, vehicle_id).await?;

    Ok(Json(ParkedEnergyResponse {
        vehicle_id,
        generated_at: Utc::now(),
        source: "rivian_reported",
        samples,
    }))
}

struct ParkedEvidencePlan {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

impl ParkedEvidencePlan {
    fn resolve(params: ParkedEnergyParams) -> Self {
        let to = params.to.unwrap_or_else(Utc::now);
        Self {
            from: params
                .from
                .unwrap_or_else(|| to - chrono::Duration::days(30)),
            to,
        }
    }

    async fn read(
        self,
        pool: &sqlx::PgPool,
        vehicle: Uuid,
    ) -> Result<Vec<ParkedEnergySample>, AppError> {
        Ok(sqlx::query_as::<_, ParkedEnergySample>(
            "WITH evidence AS (
                SELECT *, row_number() OVER (PARTITION BY period_window ORDER BY source_at DESC) AS rank
                FROM timeseries.parallax_parked_energy_samples
                WHERE vehicle_id=$1 AND source_at >= $2 AND source_at <= $3
             )
             SELECT period_window AS window,source_at,received_at,parked_started_at,duration_minutes,
                    total_kwh,vehicle_systems_kwh,outlets_kwh,climate_kwh,gear_guard_kwh,
                    total_range_impact_km,vehicle_systems_range_impact_km,outlets_range_impact_km,
                    climate_range_impact_km,gear_guard_range_impact_km
             FROM evidence WHERE rank=1 ORDER BY period_window"
        ).bind(vehicle).bind(self.from).bind(self.to).fetch_all(pool).await?)
    }
}

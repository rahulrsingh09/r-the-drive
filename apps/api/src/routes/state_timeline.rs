//! Vehicle state timeline endpoint — returns `vehicle_state_periods` rows for
//! a time range, suitable for rendering a Gantt-style state chart.

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

pub fn router() -> Router<AppState> {
    Router::new().route("/vehicles/{vehicle_id}/state-timeline", get(state_timeline))
}
#[derive(Deserialize)]
struct TimelineParams {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    500
}

#[derive(Serialize, sqlx::FromRow)]
struct StatePeriod {
    id: i64,
    state: String,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    duration_seconds: Option<i32>,
}

#[derive(Serialize)]
struct TimelineResponse {
    vehicle_id: Uuid,
    periods: Vec<StatePeriod>,
}

async fn state_timeline(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
    Query(params): Query<TimelineParams>,
) -> Result<Json<TimelineResponse>, AppError> {
    require_vehicle_access(&auth, vehicle_id)?;
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let plan = TimelinePlan::resolve(params);
    let periods = plan.read(&state.pool, vehicle_id).await?;

    Ok(Json(TimelineResponse {
        vehicle_id,
        periods,
    }))
}

struct TimelinePlan {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
}

impl TimelinePlan {
    fn resolve(params: TimelineParams) -> Self {
        Self {
            from: params
                .from
                .unwrap_or_else(|| Utc::now() - chrono::Duration::days(7)),
            to: params.to.unwrap_or_else(Utc::now),
            limit: params.limit.min(2000),
        }
    }

    async fn read(self, pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Vec<StatePeriod>, AppError> {
        Ok(sqlx::query_as::<_, StatePeriod>(
            "WITH selected AS MATERIALIZED (
                SELECT id, started_at FROM panda.vehicle_state_periods
                WHERE vehicle_id=$1 AND started_at >= $2 AND started_at <= $3
                ORDER BY started_at DESC LIMIT $4
             )
             SELECT p.id, p.state, p.started_at, p.ended_at, p.duration_seconds
             FROM selected s JOIN panda.vehicle_state_periods p ON p.id=s.id
             ORDER BY s.started_at DESC",
        )
        .bind(vehicle)
        .bind(self.from)
        .bind(self.to)
        .bind(self.limit)
        .fetch_all(pool)
        .await?)
    }
}

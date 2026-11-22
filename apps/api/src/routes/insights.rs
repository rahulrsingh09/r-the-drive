//! `GET /vehicles/{vehicle_id}/insights`: deterministic insight rules.
//!
//! The rules live in `services::insights`. This route only authorizes the
//! vehicle, parses the scope and returns the results.

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::insights::{self, Insight, InsightScope},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/vehicles/{vehicle_id}/insights", get(get_insights))
}

#[derive(Deserialize)]
struct InsightsParams {
    /// `now` (default), `trip:{id}` or `charge:{id}`.
    scope: Option<String>,
}

#[derive(Serialize)]
struct InsightsResponse {
    insights: Vec<Insight>,
}

async fn get_insights(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
    Query(params): Query<InsightsParams>,
) -> Result<Json<InsightsResponse>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let scope = InsightScope::parse(params.scope.as_deref())?;
    let insights =
        insights::load_insights(&state.pool, vehicle_id, auth.user_id, scope, Utc::now()).await?;
    Ok(Json(InsightsResponse { insights }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization_test_support::Fixture;

    async fn call(
        f: &Fixture,
        user: Uuid,
        vehicle: Uuid,
        scope: &str,
    ) -> Result<Json<InsightsResponse>, AppError> {
        get_insights(
            f.auth(user),
            State(f.state.clone()),
            Path(vehicle),
            Query(InsightsParams {
                scope: Some(scope.to_string()),
            }),
        )
        .await
    }

    async fn trip(f: &Fixture, vehicle: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO panda.trips (vehicle_id, started_at, ended_at, start_lat, start_lng,
             end_lat, end_lng, distance_miles, energy_wh, efficiency_wh_per_mile)
             VALUES ($1, now() - interval '2 hours', now() - interval '1 hour',
                     30.267, -97.743, 30.300, -97.700, 12.5, 5000, 400) RETURNING id",
        )
        .bind(vehicle)
        .fetch_one(&f.state.pool)
        .await
        .unwrap()
    }

    /// A closed DC charge, so the charge scope runs its curve queries.
    async fn charge(f: &Fixture, vehicle: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO panda.charge_sessions
             (vehicle_id, started_at, ended_at, source, charger_type, soc_start, soc_end)
             VALUES ($1, now() - interval '3 hours', now() - interval '150 minutes',
                     'telemetry', 'dc', 20, 60) RETURNING id",
        )
        .bind(vehicle)
        .fetch_one(&f.state.pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn authorization_insights_scopes_reject_other_vehicles_and_non_members() {
        let f = Fixture::new().await;
        let owner = f.user("user").await;
        let other_owner = f.user("user").await;
        let outsider = f.user("user").await;
        let vehicle = f.vehicle(owner, "synthetic-insights-own").await;
        let other_vehicle = f.vehicle(other_owner, "synthetic-insights-other").await;
        let own_trip = trip(&f, vehicle).await;
        let own_charge = charge(&f, vehicle).await;
        let other_trip = trip(&f, other_vehicle).await;
        let other_charge = charge(&f, other_vehicle).await;

        // The member reads every scope of the own vehicle.
        for scope in [
            "now".to_string(),
            format!("trip:{own_trip}"),
            format!("charge:{own_charge}"),
        ] {
            let result = call(&f, owner, vehicle, &scope).await;
            assert!(result.is_ok(), "{scope}: {:?}", result.err());
        }

        // Another vehicle's trip or charge is not found through the own vehicle.
        for scope in [
            format!("trip:{other_trip}"),
            format!("charge:{other_charge}"),
        ] {
            let result = call(&f, owner, vehicle, &scope).await;
            assert!(matches!(result, Err(AppError::NotFound)), "{scope}");
        }

        // A user who is not a member gets the access error for every scope,
        // including scopes that name the vehicle's own trip and charge.
        for scope in [
            "now".to_string(),
            format!("trip:{own_trip}"),
            format!("charge:{own_charge}"),
        ] {
            let result = call(&f, outsider, vehicle, &scope).await;
            assert!(matches!(result, Err(AppError::Forbidden)), "{scope}");
        }
        let result = call(&f, owner, other_vehicle, &format!("trip:{other_trip}")).await;
        assert!(matches!(result, Err(AppError::Forbidden)));

        f.cleanup().await;
    }
}

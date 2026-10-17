//! Charging history backfill routes.
//!
//! This module owns the public API for charging history backfill. Both the
//! current settings endpoint and the older charging-specific endpoint route
//! through the same service so status, locking, and error handling stay aligned.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use uuid::Uuid;

use crate::{
    db::vehicles::{require_vehicle_manager_access, require_vehicle_read_access},
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::charge_backfill::{self, ChargeBackfillError},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vehicles/{id}/backfill-status", get(get_backfill_status))
        .route("/vehicles/{id}/backfill", post(trigger_backfill))
        .route("/vehicles/{id}/charging-backfill", post(trigger_backfill))
}

async fn get_backfill_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<charge_backfill::ChargeBackfillStatus>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    Ok(Json(
        charge_backfill::get_status(&state.pool, vehicle_id)
            .await
            .map_err(map_backfill_error)?,
    ))
}

async fn trigger_backfill(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<charge_backfill::ChargeBackfillStart>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    Ok(Json(
        charge_backfill::spawn(state.pool.clone(), state.age_key.clone(), vehicle_id)
            .await
            .map_err(map_backfill_error)?,
    ))
}

fn map_backfill_error(error: ChargeBackfillError) -> AppError {
    match error {
        ChargeBackfillError::AlreadyRunning => {
            AppError::Conflict("backfill already running".into())
        }
        ChargeBackfillError::CredentialsNotFound => AppError::NotFound,
        ChargeBackfillError::Database(error) => AppError::Database(error),
        ChargeBackfillError::Other(error) => AppError::Internal(error),
    }
}

#[cfg(test)]
mod recovery_contracts {
    use super::*;
    use crate::authorization_test_support::Fixture;
    use axum::{body::Body, http::Request, Extension};
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn backfill_route_aliases_authorization_and_service_failures() {
        let fixture = Fixture::new().await;
        let owner = fixture.user("user").await;
        let viewer = fixture.user("user").await;
        let outsider = fixture.user("user").await;
        let vehicle = fixture.vehicle(owner, "demo-backfill-contract").await;
        fixture.member(viewer, vehicle, "viewer").await;
        let app = Router::new()
            .nest("/v1", router())
            .layer(Extension(fixture.state.jwt_keys.decoding.clone()))
            .with_state(fixture.state.clone());
        let status_path = format!("/v1/vehicles/{vehicle}/backfill-status");
        let before = fixture.snapshot().await;
        for (method, path, user, expected) in [
            ("GET", status_path.clone(), None, 401),
            ("GET", status_path.clone(), Some(outsider), 403),
            ("GET", status_path.clone(), Some(viewer), 200),
            (
                "POST",
                format!("/v1/vehicles/{vehicle}/backfill"),
                Some(viewer),
                403,
            ),
            (
                "POST",
                format!("/v1/vehicles/{vehicle}/backfill"),
                Some(owner),
                404,
            ),
            (
                "POST",
                format!("/v1/vehicles/{vehicle}/charging-backfill"),
                Some(owner),
                404,
            ),
        ] {
            let mut request = Request::builder().method(method).uri(path);
            if let Some(user) = user {
                request =
                    request.header("authorization", format!("Bearer {}", fixture.token(user)));
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
            if expected == 200 {
                let bytes = axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap();
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(
                    value,
                    json!({
                        "vehicle_id": vehicle,
                        "history_backfilled_at": null,
                        "status": null,
                        "rivian_session_count": null,
                        "local_session_count": 0,
                        "missing_source_count": 1
                    })
                );
            }
        }
        assert_eq!(fixture.snapshot().await, before);
        sqlx::query("UPDATE panda.vehicles SET history_backfill_status='running', updated_at=now() WHERE id=$1")
            .bind(vehicle).execute(&fixture.state.pool).await.unwrap();
        for suffix in ["backfill", "charging-backfill"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/v1/vehicles/{vehicle}/{suffix}"))
                        .header("authorization", format!("Bearer {}", fixture.token(owner)))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 409);
        }
        fixture.cleanup().await;
    }

    #[test]
    fn backfill_error_conversion_preserves_all_variants() {
        assert!(matches!(
            map_backfill_error(ChargeBackfillError::AlreadyRunning),
            AppError::Conflict(message) if message == "backfill already running"
        ));
        assert!(matches!(
            map_backfill_error(ChargeBackfillError::CredentialsNotFound),
            AppError::NotFound
        ));
        assert!(matches!(
            map_backfill_error(ChargeBackfillError::Database(sqlx::Error::RowNotFound)),
            AppError::Database(sqlx::Error::RowNotFound)
        ));
        assert!(matches!(
            map_backfill_error(ChargeBackfillError::Other(anyhow::anyhow!("synthetic failure"))),
            AppError::Internal(error) if error.to_string() == "synthetic failure"
        ));
    }
}

//! CRUD routes for user-defined cost profiles.

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    models::cost_profile::{validate_tou_periods, CostProfile, TouPeriod},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/cost-profiles", get(list_profiles).post(create_profile))
        .route(
            "/cost-profiles/{id}",
            get(get_profile).put(update_profile).delete(delete_profile),
        )
}

// ─── Request bodies ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateProfileBody {
    name: String,
    billing_type: String,
    rate: f64,
    session_fee: Option<f64>,
    currency: Option<String>,
    timezone: Option<String>,
    tou_periods: Option<serde_json::Value>,
    effective_from: Option<chrono::NaiveDate>,
    effective_to: Option<chrono::NaiveDate>,
}

#[derive(Deserialize)]
struct UpdateProfileBody {
    name: Option<String>,
    billing_type: Option<String>,
    rate: Option<f64>,
    session_fee: Option<f64>,
    currency: Option<String>,
    timezone: Option<String>,
    tou_periods: Option<serde_json::Value>,
    effective_from: Option<chrono::NaiveDate>,
    effective_to: Option<chrono::NaiveDate>,
}

#[derive(Serialize)]
struct ProfileListResponse {
    cost_profiles: Vec<CostProfile>,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn list_profiles(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<ProfileListResponse>, AppError> {
    let cost_profiles = workflow::ProfileSelection::All(auth.user_id)
        .load(&state.pool)
        .await?;
    Ok(Json(ProfileListResponse { cost_profiles }))
}

async fn get_profile(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CostProfile>, AppError> {
    workflow::ProfileSelection::One {
        owner: auth.user_id,
        id,
    }
    .require(&state.pool)
    .await
    .map(Json)
}

async fn create_profile(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateProfileBody>,
) -> Result<Json<CostProfile>, AppError> {
    workflow::ProfileMutation::create(auth.user_id, body)?
        .persist(&state.pool)
        .await
        .map(Json)
}

async fn update_profile(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProfileBody>,
) -> Result<Json<CostProfile>, AppError> {
    workflow::ProfileMutation::update(&state.pool, auth.user_id, id, body)
        .await?
        .persist(&state.pool)
        .await
        .map(Json)
}

async fn delete_profile(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    workflow::ProfileSelection::One {
        owner: auth.user_id,
        id,
    }
    .delete(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ─── Validation ───────────────────────────────────────────────────────────────

fn validate_billing_type(bt: &str) -> Result<(), AppError> {
    match bt {
        "per_kwh" | "per_minute" | "free" | "flat" | "tou" => Ok(()),
        _ => Err(AppError::Validation(
            "billing_type must be one of: per_kwh, per_minute, free, flat, tou".to_string(),
        )),
    }
}

fn normalize_tou_periods(value: Option<serde_json::Value>) -> Result<serde_json::Value, AppError> {
    match value {
        Some(value) => Ok(value),
        None => Ok(serde_json::json!([])),
    }
}

fn validate_profile_details(
    billing_type: &str,
    rate: f64,
    timezone: Option<&str>,
    tou_periods: &serde_json::Value,
) -> Result<(), AppError> {
    validate_billing_type(billing_type)?;
    if !rate.is_finite() || rate < 0.0 {
        return Err(AppError::Validation(
            "rate must be a non-negative number".into(),
        ));
    }
    // Sanity check: rates are dollars per kWh.  Reject obviously-mis-unitted
    // values (e.g. someone typed "32.2499" meaning cents per kWh) before they
    // silently inflate every cost calculation by 100×.  Flat-fee profiles
    // ("flat" billing_type) use `rate` as a per-session dollar amount, where
    // larger values are legitimate — so this guard only applies to per_kwh.
    if billing_type == "per_kwh" && rate > 5.0 {
        return Err(AppError::Validation(format!(
            "Rate {rate:.4} $/kWh is too high. Enter the rate in dollars per kWh \
             (e.g. 0.13 for 13¢/kWh), not cents."
        )));
    }

    if billing_type == "tou" {
        if timezone
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(AppError::Validation(
                "timezone is required for TOU profiles".into(),
            ));
        }
        let periods: Vec<TouPeriod> =
            serde_json::from_value(tou_periods.clone()).map_err(|_| {
                AppError::Validation("tou_periods must be a JSON array of schedule periods".into())
            })?;
        validate_tou_periods(&periods).map_err(AppError::Validation)?;
    }

    Ok(())
}

#[path = "cost_profiles/workflow.rs"]
mod workflow;

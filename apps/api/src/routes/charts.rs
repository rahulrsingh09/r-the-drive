#[path = "charts/workflow.rs"]
mod workflow;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use uuid::Uuid;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::{
        chart_registry::{
            merge_entries, ChartManagerEntry, ChartRecord, BUNDLED_CHART_BASELINE_REVISION,
        },
        chart_validation::parse_and_validate_for_slug,
    },
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartInput {
    slug: String,
    name: String,

    description: Option<String>,
    #[serde(alias = "is_enabled")]
    is_enabled: Option<bool>,
    config: Value,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartPatch {
    name: Option<String>,
    description: Option<String>,
    #[serde(alias = "is_enabled")]
    is_enabled: Option<bool>,
    config: Option<Value>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloneInput {
    slug: Option<String>,
    name: Option<String>,
}
#[derive(Debug, Deserialize)]
struct LockInput {
    locked: bool,
}
#[derive(Debug, Deserialize)]
struct EffectiveQuery {
    dashboard_slug: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlacementInput {
    dashboard_slug: String,
}

#[derive(Debug, Deserialize)]
struct PlacementPatch {
    placements: Vec<PlacementInput>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/charts", get(list).post(create))
        .route("/chart-sources", get(source_manifest))
        .route("/charts/effective", get(effective))
        .route(
            "/charts/{id}",
            get(fetch).put(update).patch(update).delete(remove),
        )
        .route("/charts/{id}/clone", post(clone_chart))
        .route("/charts/{id}/reset", post(reset))
        .route("/charts/{id}/placements", patch(set_placements))
        .route("/admin/charts/{id}", put(admin_update).patch(admin_update))
        .route("/admin/charts/{id}/lock", put(admin_lock).patch(admin_lock))
        .route("/admin/charts/{id}/restore", post(admin_restore))
}

async fn source_manifest(_auth: AuthUser) -> Json<Value> {
    Json(
        serde_json::from_str(include_str!("../../charts/sources.json"))
            .expect("bundled chart source manifest must be valid JSON"),
    )
}

async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ChartManagerEntry>>, AppError> {
    let rows = visible_rows(&state, auth.user_id).await?;
    let admin = is_admin(&state, auth.user_id).await?;
    Ok(Json(workflow::CatalogScope::Manager.project(
        rows,
        auth.user_id,
        admin,
    )))
}

async fn effective(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<EffectiveQuery>,
) -> Result<Json<Vec<ChartManagerEntry>>, AppError> {
    let admin = is_admin(&state, auth.user_id).await?;
    let rows = visible_rows(&state, auth.user_id).await?;
    Ok(Json(
        workflow::CatalogScope::Effective(query.dashboard_slug.as_deref()).project(
            rows,
            auth.user_id,
            admin,
        ),
    ))
}

async fn fetch(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ChartRecord>, AppError> {
    let row = get_chart(&state, id).await?;
    check_read(&row, auth.user_id)?;
    Ok(Json(row))
}

async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ChartInput>,
) -> Result<(StatusCode, Json<ChartRecord>), AppError> {
    let write = workflow::NewChart::from_input(auth.user_id, body)?;
    Ok((StatusCode::CREATED, Json(write.insert(&state).await?)))
}

async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ChartPatch>,
) -> Result<Json<ChartRecord>, AppError> {
    let existing = get_chart(&state, id).await?;
    check_write(&existing, auth.user_id, false)?;
    Ok(Json(
        workflow::Change::patch(&existing, body)?
            .persist(&state)
            .await?,
    ))
}

async fn remove(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let row = get_chart(&state, id).await?;
    check_write(&row, auth.user_id, false)?;
    sqlx::query("DELETE FROM panda.charts WHERE id=$1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clone_chart(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CloneInput>,
) -> Result<(StatusCode, Json<ChartRecord>), AppError> {
    let source = get_chart(&state, id).await?;
    check_read(&source, auth.user_id)?;
    let write = workflow::NewChart::from_source(auth.user_id, source, body)?;
    Ok((StatusCode::CREATED, Json(write.insert(&state).await?)))
}

async fn reset(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ChartRecord>, AppError> {
    let existing = get_chart(&state, id).await?;
    if existing.owner_id != Some(auth.user_id) {
        return Err(AppError::Forbidden);
    }
    let removal = workflow::OverrideRemoval::resolve(&state, existing).await?;
    Ok(Json(removal.remove(&state).await?))
}

async fn set_placements(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PlacementPatch>,
) -> Result<Json<ChartRecord>, AppError> {
    let existing = get_chart(&state, id).await?;
    check_write(&existing, auth.user_id, false)?;
    Ok(Json(
        workflow::Change::placements(existing, body)?
            .persist(&state)
            .await?,
    ))
}

async fn admin_update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ChartPatch>,
) -> Result<Json<ChartRecord>, AppError> {
    require_admin(&state, auth.user_id).await?;
    let existing = get_chart(&state, id).await?;
    Ok(Json(
        workflow::Change::patch(&existing, body)?
            .persist(&state)
            .await?,
    ))
}
async fn admin_lock(
    State(state): State<AppState>,
    auth: AuthUser,

    Path(id): Path<Uuid>,
    Json(body): Json<LockInput>,
) -> Result<Json<ChartRecord>, AppError> {
    require_admin(&state, auth.user_id).await?;
    Ok(Json(
        workflow::Change::Lock {
            id,
            locked: body.locked,
        }
        .persist(&state)
        .await?,
    ))
}
async fn admin_restore(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ChartRecord>, AppError> {
    require_admin(&state, auth.user_id).await?;
    let row = get_chart(&state, id).await?;
    if row.owner_id.is_some() {
        let removal = workflow::OverrideRemoval::resolve(&state, row).await?;
        return Ok(Json(removal.remove(&state).await?));
    }
    Ok(Json(
        workflow::Change::baseline(row)?.persist(&state).await?,
    ))
}

fn bundled_default(slug: &str) -> Option<Value> {
    serde_json::from_str::<Vec<Value>>(include_str!("../../charts/defaults.json"))
        .ok()?
        .into_iter()
        .find(|chart| chart["slug"].as_str() == Some(slug))
}

async fn visible_rows(state: &AppState, user_id: Uuid) -> Result<Vec<ChartRecord>, AppError> {
    Ok(sqlx::query_as::<_,ChartRecord>("SELECT id,owner_id,slug,name,description,is_default,is_locked,is_enabled,config,baseline_revision,created_at,updated_at FROM panda.charts WHERE owner_id IS NULL OR owner_id=$1 ORDER BY slug,owner_id NULLS FIRST").bind(user_id).fetch_all(&state.pool).await?)
}
async fn get_chart(state: &AppState, id: Uuid) -> Result<ChartRecord, AppError> {
    sqlx::query_as::<_,ChartRecord>("SELECT id,owner_id,slug,name,description,is_default,is_locked,is_enabled,config,baseline_revision,created_at,updated_at FROM panda.charts WHERE id=$1").bind(id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)
}
fn check_read(row: &ChartRecord, user_id: Uuid) -> Result<(), AppError> {
    if row.owner_id.is_none() || row.owner_id == Some(user_id) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
fn check_write(row: &ChartRecord, user_id: Uuid, admin: bool) -> Result<(), AppError> {
    if admin || row.owner_id == Some(user_id) && !row.is_locked {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
async fn is_admin(state: &AppState, user_id: Uuid) -> Result<bool, AppError> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("SELECT role FROM panda.users WHERE id=$1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten()
            .is_some_and(|r| r == "admin" || r == "super_user"),
    )
}
async fn require_admin(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    if is_admin(state, user_id).await? {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
fn validate_slug(slug: &str) -> Result<(), AppError> {
    if !validate_slug_value(slug) {
        Err(AppError::Validation(
            "slug must use lowercase letters, digits, dots, hyphens, and underscores".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_slug_value(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 80
        && slug.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}
fn validate_name(name: &str) -> Result<(), AppError> {
    if name.trim().is_empty() || name.len() > 160 {
        Err(AppError::Validation(
            "name must be between 1 and 160 characters".into(),
        ))
    } else {
        Ok(())
    }
}
fn validate_config(slug: &str, config: &Value) -> Result<(), AppError> {
    parse_and_validate_for_slug(slug, config)
        .map(|_| ())
        .map_err(|errors| {
            AppError::Validation(
                serde_json::to_string(
                    &serde_json::json!({"code":"CHART_DEFINITION_INVALID","errors":errors}),
                )
                .unwrap_or_else(|_| "invalid chart definition".into()),
            )
        })
}
fn map_conflict(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .and_then(|e| e.constraint())
        .is_some()
    {
        AppError::Conflict("a chart with that slug already exists for this owner".into())
    } else {
        AppError::Database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slug_validation_is_bounded() {
        assert!(validate_slug("soc-history").is_ok());
        assert!(validate_slug("SOC").is_err());
        assert!(validate_slug("https://bad").is_err());
    }
}

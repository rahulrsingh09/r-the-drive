#[path = "dashboards/reconcile.rs"]
mod reconcile;
#[path = "dashboards/store.rs"]
mod store;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
};

// Advance this whenever a bundled system dashboard changes so existing
// installations receive the new baseline on their next startup.
const BUNDLED_DASHBOARD_BASELINE_REVISION: i32 = 9;

const RETIRED_TRIPS_MANAGED_WIDGETS: &[(&str, &str)] = &[(
    "trips.tire-pressure-timeline",
    "d5000005-0000-0000-0000-000000000007",
)];

const UPSERT_SYSTEM_DEFAULT_SQL: &str = r#"
    INSERT INTO dashboards
      (id, owner_id, slug, name, is_default, is_locked, config, baseline_revision)
    VALUES ($1, NULL, $2, $3, TRUE, TRUE, $4, $5)
    ON CONFLICT (id) DO UPDATE
    SET slug = EXCLUDED.slug,
        name = EXCLUDED.name,
        is_default = TRUE,
        is_locked = TRUE,
        config = EXCLUDED.config,
        baseline_revision = EXCLUDED.baseline_revision,
        updated_at = NOW()
    WHERE dashboards.owner_id IS NULL
      AND dashboards.is_default = TRUE
      AND COALESCE(dashboards.baseline_revision, 0) < EXCLUDED.baseline_revision
    "#;

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Dashboard {
    pub id: Uuid,
    pub owner_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub is_locked: bool,
    pub config: Value,
}

#[derive(Debug, Deserialize)]
pub struct CreateDashboard {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub config: Value,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDashboard {
    pub name: Option<String>,
    pub description: Option<String>,
    pub config: Option<Value>,
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dashboards", get(list).post(create))
        .route("/dashboards/{id}", get(fetch).put(update).delete(remove))
        .route("/dashboards/{id}/clone", post(clone_dashboard))
        .route("/admin/dashboards/{id}", put(admin_update))
        .route("/admin/dashboards/{id}/lock", post(admin_set_lock))
        .route(
            "/admin/dashboards/{id}/restore-default",
            post(admin_restore_default),
        )
}

pub fn metadata_router() -> Router<AppState> {
    Router::new().route("/dashboards/by-slug/{slug}", get(by_slug))
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// List the current user's dashboards plus all visible system defaults.
async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<Dashboard>>, AppError> {
    Ok(Json(
        store::ReadPlan::Visible(auth.user_id)
            .rows(&state.pool)
            .await?,
    ))
}

/// Fetch a single dashboard by id.
async fn fetch(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Dashboard>, AppError> {
    let row = get_dashboard(&state, id).await?;
    check_read_access(&row, auth.user_id)?;
    Ok(Json(row))
}

/// Resolve slug → config. User's own dashboard takes priority over system default.
async fn by_slug(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(slug): Path<String>,
) -> Result<Json<Dashboard>, AppError> {
    Ok(Json(
        store::ReadPlan::Preferred {
            owner: auth.user_id,
            slug: &slug,
        }
        .one(&state.pool)
        .await?,
    ))
}

/// Create a user-owned dashboard.
async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateDashboard>,
) -> Result<(StatusCode, Json<Dashboard>), AppError> {
    validate_slug(&body.slug)?;
    let write = store::NewDashboard::create(auth.user_id, body);
    Ok((StatusCode::CREATED, Json(write.save(&state.pool).await?)))
}

/// Update a user-owned dashboard (non-admin cannot touch system defaults).
async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateDashboard>,
) -> Result<Json<Dashboard>, AppError> {
    let existing = get_dashboard(&state, id).await?;
    check_write_access(&existing, auth.user_id, false)?;
    let write = store::PatchWrite::prepare(existing, body);
    Ok(Json(write.save(&state.pool, false).await?))
}

/// Delete a user-owned dashboard.
async fn remove(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let existing = get_dashboard(&state, id).await?;
    check_write_access(&existing, auth.user_id, false)?;

    sqlx::query("DELETE FROM dashboards WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Clone a dashboard (system default → user copy, or copy any accessible dashboard).
async fn clone_dashboard(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Dashboard>), AppError> {
    let source = get_dashboard(&state, id).await?;
    check_read_access(&source, auth.user_id)?;
    let write = store::NewDashboard::clone_from(auth.user_id, source);
    Ok((StatusCode::CREATED, Json(write.save(&state.pool).await?)))
}

// ─── Admin handlers ───────────────────────────────────────────────────────────

async fn admin_update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateDashboard>,
) -> Result<Json<Dashboard>, AppError> {
    require_admin(&state, auth.user_id).await?;
    let existing = get_dashboard(&state, id).await?;
    let write = store::PatchWrite::prepare(existing, body);
    Ok(Json(write.save(&state.pool, true).await?))
}

#[derive(Deserialize)]
struct LockBody {
    locked: bool,
}

async fn admin_set_lock(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<LockBody>,
) -> Result<Json<Dashboard>, AppError> {
    require_admin(&state, auth.user_id).await?;
    Ok(Json(
        store::MetadataWrite::Lock {
            id,
            locked: body.locked,
        }
        .save(&state.pool)
        .await?,
    ))
}

async fn admin_restore_default(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Dashboard>, AppError> {
    require_admin(&state, auth.user_id).await?;
    let existing = get_dashboard(&state, id).await?;
    Ok(Json(
        store::MetadataWrite::restore(existing)?
            .save(&state.pool)
            .await?,
    ))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

async fn get_dashboard(state: &AppState, id: Uuid) -> Result<Dashboard, AppError> {
    store::ReadPlan::Identity(id).one(&state.pool).await
}

fn check_read_access(d: &Dashboard, user_id: Uuid) -> Result<(), AppError> {
    // System defaults are readable by all authenticated users
    if d.owner_id.is_none() {
        return Ok(());
    }
    if d.owner_id == Some(user_id) {
        return Ok(());
    }
    Err(AppError::Forbidden)
}

fn check_write_access(d: &Dashboard, user_id: Uuid, is_admin: bool) -> Result<(), AppError> {
    if d.is_locked && !is_admin {
        return Err(AppError::Forbidden);
    }
    if d.owner_id.is_none() && !is_admin {
        return Err(AppError::Forbidden);
    }
    if d.owner_id != Some(user_id) && !is_admin {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

async fn require_admin(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    let role = sqlx::query_scalar!("SELECT role FROM panda.users WHERE id = $1", user_id)
        .fetch_optional(&state.pool)
        .await?;

    match role.as_deref() {
        Some("admin") | Some("super_user") => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

fn validate_slug(slug: &str) -> Result<(), AppError> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c.is_ascii_digit())
    {
        return Err(AppError::Validation(
            "Slug must be lowercase letters, digits, and hyphens only".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dashboard_config_with_metadata(
    mut config: Value,
    id: Uuid,
    owner_id: Option<Uuid>,
    slug: &str,
    name: &str,
    description: Option<&str>,
    is_default: bool,
    is_locked: bool,
) -> Value {
    if let Value::Object(ref mut object) = config {
        object.insert("id".into(), Value::String(id.to_string()));
        object.insert("slug".into(), Value::String(slug.to_string()));
        object.insert("name".into(), Value::String(name.to_string()));
        match description {
            Some(description) => {
                object.insert("description".into(), Value::String(description.to_string()));
            }
            None => {
                object.remove("description");
            }
        }
        object.insert("isDefault".into(), Value::Bool(is_default));
        object.insert("isLocked".into(), Value::Bool(is_locked));
        object.insert(
            "ownerId".into(),
            owner_id
                .map(|id| Value::String(id.to_string()))
                .unwrap_or(Value::Null),
        );
    }
    config
}

fn bundled_default_config(id: Uuid) -> Option<Value> {
    let id_str = id.to_string();
    let json_str = match id_str.as_str() {
        "00000000-0000-0000-0000-000000000001" => include_str!("../../dashboards/dashboard.json"),
        "00000000-0000-0000-0000-000000000002" => include_str!("../../dashboards/battery.json"),
        "00000000-0000-0000-0000-000000000003" => include_str!("../../dashboards/efficiency.json"),
        "00000000-0000-0000-0000-000000000004" => include_str!("../../dashboards/charging.json"),
        "00000000-0000-0000-0000-000000000005" => include_str!("../../dashboards/trips.json"),
        _ => return None,
    };

    serde_json::from_str(json_str).ok()
}

// ─── Seed ────────────────────────────────────────────────────────────────────

/// Insert missing system defaults and apply a newer bundled baseline once.
///
/// User-owned dashboards keep their saved layout and widget settings. The
/// additive managed-composition patch below only adds or marks explicitly
/// managed bundled widgets so new fixed page content reaches existing copies.
pub async fn seed_defaults(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let defaults: &[(&str, &str)] = &[
        (
            "00000000-0000-0000-0000-000000000001",
            include_str!("../../dashboards/dashboard.json"),
        ),
        (
            "00000000-0000-0000-0000-000000000002",
            include_str!("../../dashboards/battery.json"),
        ),
        (
            "00000000-0000-0000-0000-000000000003",
            include_str!("../../dashboards/efficiency.json"),
        ),
        (
            "00000000-0000-0000-0000-000000000004",
            include_str!("../../dashboards/charging.json"),
        ),
        (
            "00000000-0000-0000-0000-000000000005",
            include_str!("../../dashboards/trips.json"),
        ),
    ];

    reconcile::seed(pool, defaults).await
}

fn retired_managed_widgets(slug: &str) -> &'static [(&'static str, &'static str)] {
    match slug {
        "trips" => RETIRED_TRIPS_MANAGED_WIDGETS,
        _ => &[],
    }
}

fn merge_managed_widgets(
    config: Value,
    bundled_widgets: &[&Value],
    retired_widgets: &[(&str, &str)],
) -> Option<Value> {
    reconcile::merge(config, bundled_widgets, retired_widgets)
}

fn compact_canonical_trips_layout(widgets: &mut [Value]) -> bool {
    let mut changed = false;
    for widget in widgets {
        let (expected_y, expected_height, next_y) = match widget["id"].as_str() {
            Some("d5000005-0000-0000-0000-000000000005") => (9, 10, 2),
            Some("d5000005-0000-0000-0000-000000000006") => (19, 14, 12),
            _ => continue,
        };
        let layout_is_canonical = widget["layout"]["x"].as_i64() == Some(0)
            && widget["layout"]["y"].as_i64() == Some(expected_y)
            && widget["layout"]["w"].as_i64() == Some(12)
            && widget["layout"]["h"].as_i64() == Some(expected_height);
        if layout_is_canonical {
            widget["layout"]["y"] = Value::from(next_y);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_seed_matches_the_canonical_dashboard_source() {
        for (file_name, api_source, frontend_source) in [
            (
                "dashboard.json",
                include_str!("../../dashboards/dashboard.json"),
                include_str!("../../../../packages/dashboards/src/defaults/dashboard.json"),
            ),
            (
                "battery.json",
                include_str!("../../dashboards/battery.json"),
                include_str!("../../../../packages/dashboards/src/defaults/battery.json"),
            ),
            (
                "efficiency.json",
                include_str!("../../dashboards/efficiency.json"),
                include_str!("../../../../packages/dashboards/src/defaults/efficiency.json"),
            ),
            (
                "charging.json",
                include_str!("../../dashboards/charging.json"),
                include_str!("../../../../packages/dashboards/src/defaults/charging.json"),
            ),
            (
                "trips.json",
                include_str!("../../dashboards/trips.json"),
                include_str!("../../../../packages/dashboards/src/defaults/trips.json"),
            ),
        ] {
            let api_config: Value = serde_json::from_str(api_source).unwrap();
            let frontend_config: Value = serde_json::from_str(frontend_source).unwrap();
            assert_eq!(api_config, frontend_config, "seed drift in {file_name}");
        }
    }

    #[test]
    fn efficiency_tag_chart_advances_only_the_system_default_baseline() {
        let efficiency: Value =
            serde_json::from_str(include_str!("../../dashboards/efficiency.json")).unwrap();
        let chart_ids = efficiency["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|widget| {
                widget["componentType"] == "chart" && widget["options"]["showPicker"] == true
            })
            .and_then(|widget| widget["options"]["chartIds"].as_array())
            .expect("efficiency chart IDs");

        assert_eq!(BUNDLED_DASHBOARD_BASELINE_REVISION, 9);
        assert!(chart_ids.iter().any(|id| id == "efficiency-tags"));
        assert!(UPSERT_SYSTEM_DEFAULT_SQL.contains("dashboards.owner_id IS NULL"));
        assert!(UPSERT_SYSTEM_DEFAULT_SQL.contains("dashboards.is_default = TRUE"));
        assert!(UPSERT_SYSTEM_DEFAULT_SQL
            .contains("COALESCE(dashboards.baseline_revision, 0) < EXCLUDED.baseline_revision"));
    }

    #[test]
    fn bundled_default_config_resolves_known_dashboard_ids() {
        let id: Uuid = "00000000-0000-0000-0000-000000000004".parse().unwrap();
        let config = bundled_default_config(id).expect("charging default");

        assert_eq!(config["slug"], "charging");
        assert!(config["widgets"]
            .as_array()
            .is_some_and(|widgets| !widgets.is_empty()));
    }

    #[test]
    fn overview_bundle_uses_projected_range_by_mileage_as_its_default_chart() {
        let id: Uuid = "00000000-0000-0000-0000-000000000001".parse().unwrap();
        let config = bundled_default_config(id).expect("overview default");
        let chart_id = config["widgets"]
            .as_array()
            .and_then(|widgets| {
                widgets
                    .iter()
                    .find(|widget| widget["managedKey"] == "overview.chart-catalog")
            })
            .and_then(|widget| widget["options"]["chartId"].as_str());

        assert_eq!(chart_id, Some("projected-range-mileage"));
    }

    #[test]
    fn dashboard_config_metadata_matches_owning_row() {
        let id: Uuid = "11111111-1111-1111-1111-111111111111".parse().unwrap();
        let owner_id: Uuid = "22222222-2222-2222-2222-222222222222".parse().unwrap();
        let config = serde_json::json!({
            "schemaVersion": 2,
            "id": "00000000-0000-0000-0000-000000000001",
            "slug": "dashboard",
            "name": "Overview",
            "description": "old",
            "isDefault": true,
            "isLocked": true,
            "ownerId": null,
            "controls": { "dateRange": true },
            "widgets": []
        });

        let normalized = dashboard_config_with_metadata(
            config,
            id,
            Some(owner_id),
            "dashboard-copy",
            "Dashboard Copy",
            None,
            false,
            false,
        );

        assert_eq!(normalized["id"], id.to_string());
        assert_eq!(normalized["ownerId"], owner_id.to_string());
        assert_eq!(normalized["slug"], "dashboard-copy");
        assert_eq!(normalized["name"], "Dashboard Copy");
        assert_eq!(normalized["isDefault"], false);
        assert_eq!(normalized["isLocked"], false);
        assert!(normalized.get("description").is_none());
    }

    #[test]
    fn managed_composition_patch_preserves_personal_layout_and_options() {
        let bundled = serde_json::json!({
            "id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "componentType": "chart",
            "definitionId": "catalog",
            "managed": true,
            "managedKey": "example.chart",
            "options": { "page": "example", "chartId": "example-chart", "showPicker": false },
            "layout": { "x": 0, "y": 2, "w": 12, "h": 7 }
        });
        let managed = vec![&bundled];
        let current = serde_json::json!({
            "widgets": [{
                "id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "componentType": "chart",
                "definitionId": "legacy-chart",
                "title": "My timeline",
                "layout": { "x": 2, "y": 41, "w": 8, "h": 9 },
                "visibility": [{ "type": "vehicle-connection", "value": "plugged" }],
                "options": { "chartSettings": { "example-chart": { "timeFilter": "24h" } }, "custom": true }
            }]
        });

        let patched = merge_managed_widgets(current, &managed, &[]).unwrap();
        let widget = &patched["widgets"][0];
        assert_eq!(widget["layout"]["y"], 41);
        assert_eq!(widget["layout"]["w"], 8);
        assert_eq!(widget["title"], "My timeline");
        assert_eq!(widget["visibility"][0]["value"], "plugged");
        assert_eq!(widget["componentType"], "chart");
        assert_eq!(widget["definitionId"], "catalog");
        assert_eq!(widget["managed"], true);
        assert_eq!(widget["managedKey"], "example.chart");
        assert_eq!(widget["options"]["custom"], true);
        assert_eq!(
            widget["options"]["chartSettings"]["example-chart"]["timeFilter"],
            "24h"
        );
        assert_eq!(widget["options"]["chartId"], "example-chart");
        assert_eq!(widget["options"]["showPicker"], false);
    }

    #[test]
    fn managed_composition_patch_appends_missing_widget_without_overlap() {
        let bundled = serde_json::json!({
            "id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "componentType": "chart",
            "definitionId": "catalog",
            "managed": true,
            "managedKey": "example.chart",
            "options": { "page": "example", "chartId": "example-chart", "showPicker": false },
            "layout": { "x": 0, "y": 2, "w": 12, "h": 7 }
        });
        let managed = vec![&bundled];
        let current = serde_json::json!({
            "widgets": [{
                "id": "11111111-1111-1111-1111-111111111111",
                "layout": { "x": 0, "y": 0, "w": 12, "h": 10 }
            }]
        });

        let patched = merge_managed_widgets(current, &managed, &[]).unwrap();
        assert_eq!(patched["widgets"].as_array().unwrap().len(), 2);
        assert_eq!(patched["widgets"][1]["managedKey"], "example.chart");
        assert_eq!(patched["widgets"][1]["layout"]["y"], 10);

        let again = merge_managed_widgets(patched, &managed, &[]);
        assert!(again.is_none(), "the managed patch should be idempotent");
    }

    #[test]
    fn retired_trips_timeline_is_removed_and_only_canonical_layouts_compact() {
        let current = serde_json::json!({
            "widgets": [
                {
                    "id": "d5000005-0000-0000-0000-000000000007",
                    "componentType": "chart",
                    "managed": true,
                    "managedKey": "trips.tire-pressure-timeline",
                    "layout": { "x": 0, "y": 2, "w": 12, "h": 7 }
                },
                {
                    "id": "d5000005-0000-0000-0000-000000000005",
                    "componentType": "custom",
                    "definitionId": "trips.map",
                    "layout": { "x": 0, "y": 9, "w": 12, "h": 10 }
                },
                {
                    "id": "d5000005-0000-0000-0000-000000000006",
                    "componentType": "custom",
                    "definitionId": "trips.table",
                    "layout": { "x": 0, "y": 19, "w": 12, "h": 14 }
                },
                {
                    "id": "cccccccc-cccc-cccc-cccc-cccccccccccc",
                    "componentType": "chart",
                    "options": { "chartId": "tire-pressure-trips" },
                    "layout": { "x": 0, "y": 33, "w": 12, "h": 7 }
                }
            ]
        });

        let patched =
            merge_managed_widgets(current, &[], retired_managed_widgets("trips")).unwrap();
        let widgets = patched["widgets"].as_array().unwrap();
        assert_eq!(widgets.len(), 3);
        assert_eq!(widgets[0]["layout"]["y"], 2);
        assert_eq!(widgets[1]["layout"]["y"], 12);
        assert_eq!(widgets[2]["options"]["chartId"], "tire-pressure-trips");
        assert!(merge_managed_widgets(patched, &[], retired_managed_widgets("trips"),).is_none());
    }

    #[test]
    fn retired_trips_timeline_preserves_customized_layouts() {
        let current = serde_json::json!({
            "widgets": [
                {
                    "managedKey": "trips.tire-pressure-timeline",
                    "layout": { "x": 0, "y": 2, "w": 12, "h": 7 }
                },
                {
                    "id": "d5000005-0000-0000-0000-000000000005",
                    "componentType": "custom",
                    "definitionId": "trips.map",
                    "layout": { "x": 1, "y": 40, "w": 11, "h": 9 }
                }
            ]
        });

        let patched =
            merge_managed_widgets(current, &[], retired_managed_widgets("trips")).unwrap();
        assert_eq!(patched["widgets"][0]["layout"]["x"], 1);
        assert_eq!(patched["widgets"][0]["layout"]["y"], 40);
        assert_eq!(patched["widgets"][0]["layout"]["w"], 11);
        assert_eq!(patched["widgets"][0]["layout"]["h"], 9);
    }

    #[test]
    fn managed_composition_patch_is_selective() {
        let bundled: Value =
            serde_json::from_str(include_str!("../../dashboards/dashboard.json")).unwrap();
        let managed = bundled["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|widget| widget["managed"].as_bool() == Some(true))
            .collect::<Vec<_>>();
        let current = serde_json::json!({
            "widgets": [{
                "id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "componentType": "chart",
                "definitionId": "catalog",
                "layout": { "x": 0, "y": 0, "w": 12, "h": 5 },
                "options": { "page": "overview", "chartId": "soc-history", "chartIds": ["soc-history"] }
            }]
        });

        let patched = merge_managed_widgets(current.clone(), &managed, &[]);
        assert!(patched.is_some());
        assert_eq!(patched.unwrap()["widgets"][0], current["widgets"][0]);
    }

    // ── integration tests (require DATABASE_URL) ─────────────────────────────

    /// Regression test: `require_admin` must use the `panda.` schema prefix.
    /// Without the prefix the query fails with "relation 'users' does not exist"
    /// and returns 500 instead of 403.
    ///
    /// This test registers a non-admin user and verifies that the admin-only
    /// `POST /v1/admin/dashboards` endpoint returns 403, not 500.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn require_admin_returns_403_not_500_for_non_admin() {
        use crate::middleware::auth::{issue_access_token, AppState, JwtKeys};
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use std::sync::Arc;
        use tower::ServiceExt;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());

        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("pool");
        let redis = redis::Client::open(&*redis_url).expect("redis");

        let keys = crate::keys::generate_keys().expect("generate test keys");
        let jwt_keys = Arc::new(JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).unwrap());

        let config = crate::config::Config {
            database_url: database_url.clone(),
            redis_url: redis_url.clone(),
            jwt_secret: None,
            jwt_public_key: None,
            age_encryption_key: None,
            port: 3001,
            allowed_origins: vec![],
            s3_endpoint: None,
            s3_access_key: None,
            s3_secret_key: None,
            backup_artifact_dir: std::env::temp_dir()
                .join("panda-dashboard-test")
                .to_string_lossy()
                .into_owned(),
            vehicle_image_cache_dir: std::env::temp_dir()
                .join("panda-dashboard-test-vehicle-images")
                .to_string_lossy()
                .into_owned(),
            backup_driver: "pg_dump".into(),
            backup_poll_interval_seconds: 60,
            restore_agent_url: "http://127.0.0.1:3002".into(),
            restore_agent_key_file: "/backups/.restore-agent-key".into(),
            recovery: crate::config::RecoveryConfig::default(),
            origin_bind: crate::config::OriginBindConfig::default(),
            security: Default::default(),
            rivian_ws_reconnect_initial_seconds: 10,
            rivian_ws_reconnect_max_seconds: 900,
            rivian_raw_event_retention_days: 7,
            rivian_persist_raw_events: false,
            rivian_suppress_duplicate_telemetry: true,
            panda_env: None,
            cookie_insecure: Some(true),
            allow_insecure_lan_http_auth: false,
            rate_limit: crate::config::RateLimitConfig::default(),
        };

        let state = AppState {
            pool: pool.clone(),
            redis,
            jwt_keys: jwt_keys.clone(),
            age_key: "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
                .into(),
            config,
            nominatim_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            resources: Default::default(),
        };
        let app = crate::routes::build_router(state);

        // Seed a non-admin directly so this test does not consume the one-time
        // first-user registration flow shared by the auth integration tests.
        let email = format!("require-admin-test-{}@example.com", uuid::Uuid::new_v4());
        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO panda.users (email, password_hash, role) VALUES ($1, $2, 'user') RETURNING id",
        )
        .bind(&email)
        .bind(crate::routes::users_support::hash_password("synthetic-dashboard-password").expect("hash fixture password"))
        .fetch_one(&pool)
        .await
        .expect("seed non-admin user");
        let access_token = issue_access_token(user_id, None, &jwt_keys).expect("issue token");

        // Non-admin trying to update a global dashboard must get 403 (not 500).
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/v1/admin/dashboards/{}", uuid::Uuid::nil()))
            .header("authorization", format!("Bearer {access_token}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Test"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "non-admin must get 403; 500 would indicate the panda. prefix is missing"
        );

        // Clean up the test user.
        let _ = sqlx::query("DELETE FROM panda.users WHERE email = $1")
            .bind(&email)
            .execute(&pool)
            .await;
    }
}

#[cfg(test)]
#[path = "__tests__/personalization.rs"]
mod personalization_contract;

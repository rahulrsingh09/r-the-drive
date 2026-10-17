use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_manager_access,
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::security_audit::SecurityAuditEvent,
};

pub fn router() -> Router<AppState> {
    [
        ("/api-keys", get(list_api_keys).post(create_api_key)),
        ("/api-keys/{id}", delete(revoke_api_key)),
        ("/api-keys/{id}/rotate", post(rotate_api_key)),
        ("/api/catalog", get(api_catalog)),
        ("/admin/api/catalog", get(admin_api_catalog)),
    ]
    .into_iter()
    .fold(Router::new(), |router, (path, methods)| {
        router.route(path, methods)
    })
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiAccessLevel {
    Read,
}

impl ApiAccessLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
        }
    }
}

impl TryFrom<&str> for ApiAccessLevel {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "read" => Ok(Self::Read),
            _ => Err(AppError::Validation("access_level must be read".into())),
        }
    }
}

#[derive(Debug, Serialize)]
struct ApiKeyRecord {
    id: Uuid,
    vehicle_id: Uuid,
    name: String,
    access_level: String,
    access_level_state: &'static str,
    created_at: chrono::DateTime<chrono::Utc>,
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    expiry_state: &'static str,
    rotated_from_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct CreateApiKeyBody {
    vehicle_id: Uuid,
    name: String,
    expires_in_days: Option<i32>,
}

#[derive(Debug, Serialize)]
struct CreateApiKeyResponse {
    key: String,
    record: ApiKeyRecord,
}

#[derive(Debug, Serialize)]
struct ApiCatalogResponse {
    version: &'static str,
    authentication: &'static str,
    endpoints: Vec<ApiEndpointDoc>,
}

#[derive(Debug, Clone, Serialize)]
struct ApiEndpointDoc {
    method: &'static str,
    path: &'static str,
    vehicle_scoped: bool,
    purpose: &'static str,
}

const LIST_API_KEYS_QUERY: &str = r#"
        SELECT k.id, k.vehicle_id, COALESCE(k.name, k.label, 'API key') AS name,
               k.access_level, k.created_at, k.last_used_at, k.expires_at, k.revoked_at,
               k.rotated_from_id
        FROM panda.api_keys k
        WHERE k.user_id = $1
        ORDER BY k.created_at DESC
        "#;

const CREATE_API_KEY_QUERY: &str = r#"
        INSERT INTO panda.api_keys
               (user_id, vehicle_id, key_hash, label, name, access_level, expires_at, rotated_from_id)
        VALUES ($1, $2, $3, $4, $4, $5, $6, $7)
        RETURNING id, vehicle_id, COALESCE(name, label, 'API key') AS name,
                  access_level, created_at, last_used_at, expires_at, revoked_at, rotated_from_id
        "#;

#[derive(sqlx::FromRow)]
struct StoredKey {
    id: Uuid,
    vehicle_id: Option<Uuid>,
    name: String,
    access_level: String,
    created_at: chrono::DateTime<chrono::Utc>,
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    rotated_from_id: Option<Uuid>,
}

impl StoredKey {
    fn response(self, issued: bool) -> Result<ApiKeyRecord, AppError> {
        let missing_scope = if issued {
            "created API key missing vehicle_id"
        } else {
            "vehicle-scoped API key missing vehicle_id"
        };
        let vehicle_id = self
            .vehicle_id
            .ok_or_else(|| AppError::Validation(missing_scope.into()))?;
        let expiry_state = api_key_expiry_state(self.expires_at, self.revoked_at);
        let access_level_state = if issued {
            "supported"
        } else {
            api_access_level_state(&self.access_level)
        };
        Ok(ApiKeyRecord {
            id: self.id,
            vehicle_id,
            name: self.name,
            access_level: self.access_level,
            access_level_state,
            created_at: self.created_at,
            last_used_at: self.last_used_at,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
            expiry_state,
            rotated_from_id: self.rotated_from_id,
        })
    }
}

struct KeyIssuance<'a> {
    user: Uuid,
    vehicle: Uuid,
    name: &'a str,
    days: i32,
    previous: Option<Uuid>,
}

impl KeyIssuance<'_> {
    async fn persist(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<CreateApiKeyResponse, AppError> {
        let key = generate_api_key();
        let deadline = chrono::Utc::now() + chrono::Duration::days(i64::from(self.days));
        let stored = sqlx::query_as::<_, StoredKey>(CREATE_API_KEY_QUERY)
            .bind(self.user)
            .bind(self.vehicle)
            .bind(hash_api_key(&key))
            .bind(self.name)
            .bind(ApiAccessLevel::Read.as_str())
            .bind(deadline)
            .bind(self.previous)
            .fetch_one(&mut **tx)
            .await?;
        Ok(CreateApiKeyResponse {
            key,
            record: stored.response(true)?,
        })
    }
}

async fn list_api_keys(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ApiKeyRecord>>, AppError> {
    require_session_auth(&auth)?;
    let stored = sqlx::query_as::<_, StoredKey>(LIST_API_KEYS_QUERY)
        .bind(auth.user_id)
        .fetch_all(&state.pool)
        .await?;
    stored
        .into_iter()
        .map(|key| key.response(false))
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

async fn create_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateApiKeyBody>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), AppError> {
    require_session_auth(&auth)?;
    require_vehicle_manager_access(&state.pool, &auth, body.vehicle_id).await?;
    let issuance = KeyIssuance {
        user: auth.user_id,
        vehicle: body.vehicle_id,
        name: body.name.trim(),
        days: body.expires_in_days.unwrap_or(90),
        previous: None,
    };
    match (issuance.name.is_empty(), (1..=365).contains(&issuance.days)) {
        (true, _) => return Err(AppError::Validation("API key name is required".into())),
        (_, false) => {
            return Err(AppError::Validation(
                "expires_in_days must be between 1 and 365".into(),
            ))
        }
        _ => {}
    }
    let mut tx = state.pool.begin().await?;
    let issued = issuance.persist(&mut tx).await?;
    let audit = SecurityAuditEvent::success("api_key_created", Some(auth.user_id))
        .target(format!("api_key:{}", issued.record.id))
        .metadata(
            serde_json::json!({"vehicle_id": issuance.vehicle, "expires_in_days": issuance.days}),
        )
        .request_id_from_headers(&headers);
    audit.record_tx(&mut tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(issued)))
}

async fn revoke_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    require_session_auth(&auth)?;
    let mut tx = state.pool.begin().await?;
    let revoked: Option<Uuid> = sqlx::query_scalar(
        "UPDATE panda.api_keys SET revoked_at = now(), updated_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL RETURNING id",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let revoked = revoked.ok_or(AppError::NotFound)?;
    SecurityAuditEvent::success("api_key_revoked", Some(auth.user_id))
        .target(format!("api_key:{revoked}"))
        .request_id_from_headers(&headers)
        .record_tx(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rotate_api_key(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), AppError> {
    require_session_auth(&auth)?;
    let mut tx = state.pool.begin().await?;
    let (vehicle, name, access): (Option<Uuid>, String, String) = sqlx::query_as(
        "SELECT vehicle_id, COALESCE(name, label, 'API key'), access_level
         FROM panda.api_keys WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let vehicle = vehicle
        .ok_or_else(|| AppError::Validation("vehicle-scoped API key missing vehicle_id".into()))?;
    ApiAccessLevel::try_from(access.as_str())
        .map_err(|_| AppError::Validation("only supported read API keys may be rotated".into()))?;
    let overlap = chrono::Utc::now() + chrono::Duration::hours(24);
    sqlx::query("UPDATE panda.api_keys SET expires_at=$2, updated_at=now() WHERE id=$1")
        .bind(id)
        .bind(overlap)
        .execute(&mut *tx)
        .await?;
    let issuance = KeyIssuance {
        user: auth.user_id,
        vehicle,
        name: &name,
        days: 90,
        previous: Some(id),
    };
    let issued = issuance.persist(&mut tx).await?;
    SecurityAuditEvent::success("api_key_rotated", Some(auth.user_id))
        .target(format!("api_key:{}", issued.record.id))
        .metadata(serde_json::json!({"rotated_from_id":id,"overlap_expires_at":overlap}))
        .request_id_from_headers(&headers)
        .record_tx(&mut tx)
        .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(issued)))
}

const CATALOG_ENTRIES: &[ApiEndpointDoc] = &[
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles",
        vehicle_scoped: false,
        purpose: "List the vehicle allowed by this integration key.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/status",
        vehicle_scoped: true,
        purpose: "Read current runtime status.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/images",
        vehicle_scoped: true,
        purpose: "Read cached vehicle imagery.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/raw-data",
        vehicle_scoped: true,
        purpose: "Read bounded raw telemetry samples and coverage.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/telemetry/lanes",
        vehicle_scoped: true,
        purpose: "Read bounded, bucketed telemetry lanes for dense visualizations.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/health",
        vehicle_scoped: true,
        purpose: "Read tire, closure, thermal, and software health.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/idle-drain",
        vehicle_scoped: true,
        purpose: "Read validated parked-session drain history.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/parked-energy",
        vehicle_scoped: true,
        purpose: "Read the latest Rivian-reported parked-energy breakdown.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/state-timeline",
        vehicle_scoped: true,
        purpose: "Read derived vehicle state periods.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/locations",
        vehicle_scoped: true,
        purpose: "Read time-bucketed location history.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/live-session",
        vehicle_scoped: true,
        purpose: "Read the latest live vehicle session.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/charging-schedule",
        vehicle_scoped: true,
        purpose: "Read charging schedule configuration.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/departure-schedules",
        vehicle_scoped: true,
        purpose: "Read departure schedules.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/wallboxes",
        vehicle_scoped: true,
        purpose: "Read known wallbox details.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/ota-details",
        vehicle_scoped: true,
        purpose: "Read OTA details.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/battery/{series}",
        vehicle_scoped: true,
        purpose:
            "Read state of charge, range, capacity, health, mileage, degradation, or drain series.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/metrics/catalog",
        vehicle_scoped: false,
        purpose: "List stable metric identifiers and units.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/metrics/value",
        vehicle_scoped: true,
        purpose: "Read one latest metric value.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/metrics/series",
        vehicle_scoped: true,
        purpose: "Read one metric time series.",
    },
    ApiEndpointDoc {
        method: "POST",
        path: "/v1/metrics/batch",
        vehicle_scoped: true,
        purpose: "Read deduplicated values and bounded sparklines.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/trips",
        vehicle_scoped: true,
        purpose: "Read trip summaries.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{vehicle_id}/trip-tags",
        vehicle_scoped: true,
        purpose: "Read the shared trip tag catalog for a vehicle.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/trips/map",
        vehicle_scoped: true,
        purpose: "Read compact trip-route previews.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/trips/{id}",
        vehicle_scoped: true,
        purpose: "Read a trip and detail, track, speed, elevation, power, or series data.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/drives/{trip_id}/power",
        vehicle_scoped: true,
        purpose: "Read a trip power profile with the vehicle ID in the path.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/charging",
        vehicle_scoped: true,
        purpose: "Read sessions, summaries, chart series, curves, and analysis.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/charging-sessions",
        vehicle_scoped: true,
        purpose: "Read charging sessions with the vehicle ID in the path.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/vehicles/{id}/costs",
        vehicle_scoped: true,
        purpose: "Read charging cost summaries with the vehicle ID in the path.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/efficiency/{view}",
        vehicle_scoped: true,
        purpose: "Read efficiency summaries, trends, and temperature analysis.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/dashboard/overview/{vehicle_id}",
        vehicle_scoped: true,
        purpose: "Read the combined dashboard overview.",
    },
    ApiEndpointDoc {
        method: "GET",
        path: "/v1/grafana",
        vehicle_scoped: false,
        purpose: "Check the Grafana compatibility datasource.",
    },
    ApiEndpointDoc {
        method: "POST",
        path: "/v1/grafana/search",
        vehicle_scoped: false,
        purpose: "List Grafana metric names.",
    },
    ApiEndpointDoc {
        method: "POST",
        path: "/v1/grafana/query",
        vehicle_scoped: true,
        purpose: "Use the legacy Grafana compatibility datasource.",
    },
    ApiEndpointDoc {
        method: "POST",
        path: "/v1/grafana/annotations",
        vehicle_scoped: false,
        purpose: "Read Grafana annotations (currently empty).",
    },
    ApiEndpointDoc {
        method: "POST",
        path: "/v1/grafana/tag-keys",
        vehicle_scoped: false,
        purpose: "Read Grafana tag keys (currently empty).",
    },
    ApiEndpointDoc {
        method: "POST",
        path: "/v1/grafana/tag-values",
        vehicle_scoped: false,
        purpose: "Read Grafana tag values (currently empty).",
    },
];
fn catalog() -> ApiCatalogResponse {
    ApiCatalogResponse {
        version: "v1",
        authentication: "Bearer integration key; read-only and scoped to one vehicle.",
        endpoints: CATALOG_ENTRIES.to_vec(),
    }
}
fn api_access_level_state(value: &str) -> &'static str {
    if ApiAccessLevel::try_from(value).is_ok() {
        "supported"
    } else if ["view", "edit", "admin"].contains(&value) {
        "legacy_unmigrated"
    } else {
        "unknown"
    }
}
async fn require_user_admin(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    let authorized: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.users WHERE id=$1 AND role IN ('admin','super_user'))",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    if authorized {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

async fn api_catalog(_auth: AuthUser) -> Json<ApiCatalogResponse> {
    Json(catalog())
}
async fn admin_api_catalog(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ApiCatalogResponse>, AppError> {
    require_session_auth(&auth)?;
    require_user_admin(&state, auth.user_id).await?;
    Ok(Json(catalog()))
}
fn api_key_expiry_state(
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    _revoked_at: Option<chrono::DateTime<chrono::Utc>>,
) -> &'static str {
    match expires_at {
        None => "legacy_no_expiry",
        Some(expires_at) if expires_at <= chrono::Utc::now() => "expired",
        Some(_) => "expiring",
    }
}
fn generate_api_key() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("panda_{}", URL_SAFE_NO_PAD.encode(bytes))
}
pub fn hash_api_key(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}
fn require_session_auth(auth: &AuthUser) -> Result<(), AppError> {
    if auth.api_access_level.is_some() {
        return Err(AppError::Forbidden);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_catalog_contains_only_read_operations() {
        let catalog = catalog();

        assert!(catalog
            .endpoints
            .iter()
            .all(|endpoint| matches!(endpoint.method, "GET" | "POST")));
        assert!(catalog
            .endpoints
            .iter()
            .any(|endpoint| { endpoint.method == "POST" && endpoint.path == "/v1/metrics/batch" }));
        assert!(catalog.endpoints.iter().any(|endpoint| {
            endpoint.method == "GET" && endpoint.path == "/v1/vehicles/{id}/live-session"
        }));
        assert!(!catalog
            .endpoints
            .iter()
            .any(|endpoint| endpoint.path == "/v1/vehicles/live"));
    }

    #[test]
    fn legacy_api_key_access_levels_are_reported_without_becoming_read_capabilities() {
        assert_eq!(api_access_level_state("view"), "legacy_unmigrated");
        assert_eq!(api_access_level_state("edit"), "legacy_unmigrated");
        assert_eq!(api_access_level_state("admin"), "legacy_unmigrated");
        assert_eq!(api_access_level_state("read"), "supported");
        assert_eq!(api_access_level_state("future"), "unknown");
    }

    #[test]
    fn api_key_runtime_queries_use_plain_row_column_aliases() {
        assert!(LIST_API_KEYS_QUERY.contains("AS name"));
        assert!(CREATE_API_KEY_QUERY.contains("AS name"));
        assert!(!LIST_API_KEYS_QUERY.contains("name!"));
        assert!(!CREATE_API_KEY_QUERY.contains("name!"));
    }

    #[test]
    fn api_key_expiry_state_preserves_legacy_keys_without_expiry() {
        assert_eq!(api_key_expiry_state(None, None), "legacy_no_expiry");
        assert_eq!(
            api_key_expiry_state(
                Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
                None
            ),
            "expired"
        );
        assert_eq!(
            api_key_expiry_state(Some(chrono::Utc::now() + chrono::Duration::days(90)), None),
            "expiring"
        );
    }
}

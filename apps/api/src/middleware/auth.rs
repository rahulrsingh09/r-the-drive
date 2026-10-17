use std::{collections::HashMap, sync::Arc, time::Instant};

use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, HeaderMap, Method},
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::AppError, ingestion::supervisor::SupervisorHandle, routes::api_keys::hash_api_key,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub iss: String,
    pub exp: i64,
    pub iat: i64,
    pub default_vehicle_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub default_vehicle_id: Option<Uuid>,
    pub api_access_level: Option<String>,
    /// Integration keys retain their single-vehicle scope independently of memberships.
    pub api_vehicle_id: Option<Uuid>,
}

pub struct JwtKeys {
    pub encoding: jsonwebtoken::EncodingKey,
    pub decoding: jsonwebtoken::DecodingKey,
}

impl std::fmt::Debug for JwtKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtKeys").finish_non_exhaustive()
    }
}

impl JwtKeys {
    pub fn new(private_pem: &str, public_pem: &str) -> anyhow::Result<Self> {
        let encoding = jsonwebtoken::EncodingKey::from_rsa_pem(private_pem.as_bytes())
            .map_err(|error| anyhow::anyhow!("invalid JWT private key: {error}"))?;
        let decoding = DecodingKey::from_rsa_pem(public_pem.as_bytes())
            .map_err(|error| anyhow::anyhow!("invalid JWT public key: {error}"))?;
        Ok(Self { encoding, decoding })
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub redis: redis::Client,
    pub jwt_keys: Arc<JwtKeys>,
    pub age_key: String,
    pub config: crate::config::Config,
    pub resources: Arc<crate::services::resource_limits::ResourceLimits>,
    /// Address result cache; services::nominatim owns the shared request scheduler.
    pub nominatim_cache: Arc<tokio::sync::RwLock<HashMap<String, (Instant, serde_json::Value)>>>,
    pub supervisor: SupervisorHandle,
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

pub(crate) fn decode_access_claims(token: &str, key: &DecodingKey) -> Option<Claims> {
    if token.is_empty() {
        return None;
    }
    let mut policy = Validation::new(Algorithm::RS256);
    policy.leeway = 0;
    policy.set_issuer(&["panda.app"]);
    decode::<Claims>(token, key, &policy)
        .ok()
        .map(|verified| verified.claims)
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        match token.starts_with("panda_") {
            true => {
                authenticate_api_key(parts.method.clone(), parts.uri.path(), state, token).await
            }
            false => {
                let claims = parts
                    .extensions
                    .get::<DecodingKey>()
                    .and_then(|key| decode_access_claims(token, key))
                    .ok_or(AppError::Unauthorized)?;
                crate::services::sessions::require_enabled_session(&state.pool, &claims).await?;
                Ok(Self {
                    user_id: claims.sub,
                    default_vehicle_id: claims.default_vehicle_id,
                    api_access_level: None,
                    api_vehicle_id: None,
                })
            }
        }
    }
}

async fn authenticate_api_key(
    method: Method,
    path: &str,
    state: &AppState,
    token: &str,
) -> Result<AuthUser, AppError> {
    let route_allowed = is_integration_read_request(&method, path);
    let hash = hash_api_key(token);
    let selected = sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT user_id, vehicle_id, access_level FROM panda.api_keys
         WHERE key_hash=$1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())
           AND user_id IN (SELECT id FROM panda.users WHERE NOT is_disabled)",
    )
    .bind(hash.as_slice())
    .fetch_optional(&state.pool)
    .await?;
    let (user_id, vehicle_id, access_level) = selected.ok_or(AppError::Unauthorized)?;
    if !route_allowed || access_level != "read" {
        tracing::warn!(method = %method, path, access_level = %access_level,
            "API key request denied by integration read scope");
        return Err(AppError::Forbidden);
    }
    sqlx::query(
        "UPDATE panda.api_keys SET last_used_at=now(), updated_at=now()
         WHERE key_hash=$1 AND (last_used_at IS NULL OR last_used_at < now()-INTERVAL '1 minute')",
    )
    .bind(hash.as_slice())
    .execute(&state.pool)
    .await?;
    Ok(AuthUser {
        user_id,
        default_vehicle_id: Some(vehicle_id),
        api_access_level: Some(access_level),
        api_vehicle_id: Some(vehicle_id),
    })
}

pub fn require_vehicle_access(auth: &AuthUser, vehicle_id: Uuid) -> Result<(), AppError> {
    match auth.api_vehicle_id {
        Some(scope) if scope != vehicle_id => Err(AppError::Forbidden),
        _ => Ok(()),
    }
}

fn is_integration_read_request(method: &Method, path: &str) -> bool {
    let path = path.strip_prefix("/v1").unwrap_or(path);
    match *method {
        Method::POST => [
            "/metrics/batch",
            "/grafana/search",
            "/grafana/query",
            "/grafana/annotations",
            "/grafana/tag-keys",
            "/grafana/tag-values",
        ]
        .contains(&path),
        Method::GET => {
            ["/api/catalog", "/vehicles"].contains(&path)
                || [
                    "/battery/",
                    "/metrics/",
                    "/trips",
                    "/charging",
                    "/efficiency/",
                    "/dashboard/overview/",
                    "/grafana",
                ]
                .iter()
                .any(|prefix| path.starts_with(prefix))
                || is_scoped_vehicle_read_path(path)
        }
        _ => false,
    }
}

fn is_scoped_vehicle_read_path(path: &str) -> bool {
    let path = path.strip_prefix("/v1").unwrap_or(path);
    let Some((_, endpoint)) = path
        .strip_prefix("/vehicles/")
        .and_then(|suffix| suffix.split_once('/'))
    else {
        return false;
    };
    const ENDPOINTS: &[&str] = &[
        "status",
        "images",
        "raw-data",
        "telemetry/lanes",
        "health",
        "idle-drain",
        "parked-energy",
        "state-timeline",
        "locations",
        "live-session",
        "charging-sessions",
        "charging-sessions/*",
        "charging-sessions/*/curve",
        "costs",
        "drives/*/power",
        "charging-schedule",
        "departure-schedules",
        "wallboxes",
        "ota-details",
        "backfill-status",
    ];
    ENDPOINTS.iter().any(|pattern| {
        let mut actual = endpoint.split('/');
        pattern.split('/').all(|expected| {
            actual
                .next()
                .is_some_and(|part| expected == "*" || part == expected)
        }) && actual.next().is_none()
    })
}

pub fn issue_access_token(
    user_id: Uuid,
    default_vehicle_id: Option<Uuid>,
    keys: &JwtKeys,
) -> anyhow::Result<String> {
    issue_session_access_token(user_id, default_vehicle_id, None, keys)
}

pub fn issue_session_access_token(
    user_id: Uuid,
    default_vehicle_id: Option<Uuid>,
    sid: Option<Uuid>,
    keys: &JwtKeys,
) -> anyhow::Result<String> {
    let issued = chrono::Utc::now().timestamp();
    let claims = Claims {
        sub: user_id,
        default_vehicle_id,
        sid,
        iss: String::from("panda.app"),
        iat: issued,
        exp: issued + 900,
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(Algorithm::RS256),
        &claims,
        &keys.encoding,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_keys_allow_declared_read_posts_but_no_writes() {
        assert!(is_integration_read_request(
            &Method::POST,
            "/v1/metrics/batch"
        ));
        assert!(is_integration_read_request(
            &Method::POST,
            "/v1/grafana/query"
        ));
        assert!(!is_integration_read_request(
            &Method::POST,
            "/v1/dashboards"
        ));
        assert!(!is_integration_read_request(
            &Method::PUT,
            "/v1/vehicles/id/settings"
        ));
    }

    #[test]
    fn integration_keys_do_not_gain_unlisted_get_routes() {
        assert!(is_integration_read_request(
            &Method::GET,
            "/v1/vehicles/id/status"
        ));
        assert!(is_integration_read_request(
            &Method::GET,
            "/v1/vehicles/id/parked-energy"
        ));
        assert!(is_integration_read_request(
            &Method::GET,
            "/v1/vehicles/id/charging-sessions/session-id/curve"
        ));
        assert!(!is_integration_read_request(&Method::GET, "/v1/api-keys"));
        assert!(!is_integration_read_request(
            &Method::GET,
            "/v1/vehicles/live"
        ));
        assert!(!is_integration_read_request(
            &Method::GET,
            "/v1/admin/users"
        ));
    }

    #[test]
    fn integration_gate_accepts_router_nested_paths() {
        assert!(is_integration_read_request(&Method::GET, "/api/catalog"));
        assert!(is_integration_read_request(&Method::GET, "/vehicles"));
        assert!(is_integration_read_request(
            &Method::GET,
            "/vehicles/id/live-session"
        ));
        assert!(is_integration_read_request(&Method::POST, "/metrics/batch"));
        assert!(!is_integration_read_request(&Method::GET, "/admin/users"));
    }

    #[test]
    fn vehicle_scope_rejects_other_vehicles() {
        let allowed = Uuid::new_v4();
        let auth = AuthUser {
            user_id: Uuid::new_v4(),
            default_vehicle_id: Some(allowed),
            api_access_level: Some("read".into()),
            api_vehicle_id: Some(allowed),
        };

        assert!(require_vehicle_access(&auth, allowed).is_ok());
        assert!(matches!(
            require_vehicle_access(&auth, Uuid::new_v4()),
            Err(AppError::Forbidden)
        ));
    }
}

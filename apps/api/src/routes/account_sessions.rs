//! "Sign out other devices": revoke every refresh-session family of the
//! signed-in user except the one this browser holds.

use axum::{extract::State, routing::post, Json, Router};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::{security_audit::SecurityAuditEvent, sessions::revoke_family},
};

pub fn router() -> Router<AppState> {
    // The refresh cookie has `Path=/v1/auth`, so this route receives it.
    Router::new().route("/auth/sessions/revoke-others", post(revoke_other_sessions))
}

#[derive(Debug, Serialize)]
struct RevokeOthersResponse {
    revoked: usize,
}

fn refresh_cookie_value(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|part| part.trim().strip_prefix("refresh_token="))
        .filter(|token| !token.is_empty())
}

async fn revoke_other_sessions(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
) -> Result<Json<RevokeOthersResponse>, AppError> {
    if auth.api_access_level.is_some() {
        return Err(AppError::Forbidden);
    }
    // Without this browser's own refresh session the request cannot tell which
    // session to keep, so it changes nothing.
    let current_session_missing =
        || AppError::Conflict("This device has no active session. Sign in again.".into());
    let hash = Sha256::digest(
        refresh_cookie_value(&headers)
            .ok_or_else(current_session_missing)?
            .as_bytes(),
    )
    .to_vec();

    let mut tx = state.pool.begin().await?;
    // Same family-first lock order as refresh and password change.
    let families: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM panda.session_families
         WHERE user_id = $1 AND revoked_at IS NULL
         ORDER BY id FOR UPDATE",
    )
    .bind(auth.user_id)
    .fetch_all(&mut *tx)
    .await?;
    let current: Uuid = sqlx::query_scalar(
        "SELECT t.family_id FROM panda.refresh_tokens t
         JOIN panda.session_families f ON f.id = t.family_id
         WHERE t.token_hash = $1 AND f.user_id = $2 AND f.revoked_at IS NULL
           AND t.revoked_at IS NULL AND t.consumed_at IS NULL AND t.expires_at > now()",
    )
    .bind(&hash)
    .bind(auth.user_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(current_session_missing)?;

    let others: Vec<Uuid> = families
        .into_iter()
        .filter(|family| *family != current)
        .collect();
    for family in &others {
        revoke_family(&mut tx, *family).await?;
    }
    SecurityAuditEvent::success("other_sessions_revoked", Some(auth.user_id))
        .target(format!("user:{}", auth.user_id))
        .metadata(serde_json::json!({ "refresh_sessions_revoked": others.len() }))
        .request_id_from_headers(&headers)
        .record_tx(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(Json(RevokeOthersResponse {
        revoked: others.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_a_non_empty_refresh_cookie() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(refresh_cookie_value(&headers), None);
        headers.insert("cookie", "theme=dark; refresh_token=".parse().unwrap());
        assert_eq!(refresh_cookie_value(&headers), None);
        headers.insert(
            "cookie",
            "theme=dark; refresh_token=abc; other=1".parse().unwrap(),
        );
        assert_eq!(refresh_cookie_value(&headers), Some("abc"));
    }
}

#[cfg(test)]
#[path = "account_sessions_authorization_tests.rs"]
mod authorization_tests;

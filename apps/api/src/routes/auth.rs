use axum::{
    extract::State,
    http::{header::SET_COOKIE, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::{Executor, Postgres, Row};
use uuid::Uuid;

use crate::{
    db::vehicles::get_default_vehicle_id,
    errors::AppError,
    middleware::auth::{issue_session_access_token, AppState, AuthUser},
    routes::users_support::hash_password,
    services::security_audit::SecurityAuditEvent,
    services::{authentication_settings, oidc},
};

const MIN_PASSWORD_LEN: usize = 12;

#[derive(Deserialize)]
struct RegisterBody {
    email: String,
    password: String,
    setup_token: Option<String>,
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct InvitationTokenBody {
    token: String,
}

#[derive(Deserialize)]
struct OidcInvitationStartBody {
    token: String,
}

#[derive(Deserialize)]
struct AcceptAccountInvitationBody {
    token: String,
    password: String,
}

#[derive(Deserialize)]
struct ChangePasswordBody {
    current_password: String,
    new_password: String,
}

#[derive(Deserialize)]
struct SetPasswordWithOidcBody {
    new_password: String,
}

#[derive(Serialize)]
struct AccessTokenResponse {
    access_token: String,
    expires_in: u64,
    default_vehicle_id: Option<Uuid>,
}

#[derive(Serialize)]
struct SetupResponse {
    setup_required: bool,
    setup_proof_required: bool,
    setup_proof_available: bool,
}

#[derive(Deserialize)]
struct OidcStartBody {
    return_to: Option<String>,
}

#[derive(Deserialize)]
struct OidcCallback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UnlinkOidcBody {
    current_password: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct UnitPreferencesPayload {
    mode: String,
    distance_unit: String,
    speed_unit: String,
    temperature_unit: String,
    pressure_unit: String,
    altitude_unit: String,
    place_radius_unit: String,
    efficiency_display: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ThemePreferencesPayload {
    mode: String,
    palette: String,
}

#[derive(Serialize)]
struct PreferencesResponse {
    units: UnitPreferencesPayload,
    theme: ThemePreferencesPayload,
    map_style: String,
}

#[derive(Deserialize)]
struct PreferencesUpdateBody {
    units: Option<UnitPreferencesPayload>,
    theme: Option<ThemePreferencesPayload>,
}

#[derive(Deserialize)]
struct MapStyleUpdateBody {
    map_style: String,
}

#[derive(Serialize)]
struct MapStyleResponse {
    map_style: String,
}

#[derive(Serialize)]
struct DashboardChartFavoritesResponse {
    chart_favorites: serde_json::Value,
}

#[derive(Deserialize)]
struct DashboardChartFavoriteUpdateBody {
    key: String,
    chart_id: String,
}

const DUMMY_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$cm9vdHJvb3Ryb290cm9v$6/Ds/Z5DKq/r+z5xFo0O3sDmN5RBUQ2A6yb7z1WB1Wg";

fn oidc_public_start_available(has_super_user: bool) -> bool {
    has_super_user
}

fn password_meets_minimum(password: &str) -> bool {
    password.len() >= MIN_PASSWORD_LEN
}

fn argon2_hash(password: &str) -> Result<String, AppError> {
    use argon2::{
        password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
        Argon2,
    };
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash error: {e}")))?
        .to_string())
}

fn verify_password(password: &str, hash: &str) -> Result<(), AppError> {
    use argon2::{
        password_hash::{PasswordHash, PasswordVerifier},
        Argon2,
    };
    let parsed = PasswordHash::new(hash).map_err(|_| AppError::Unauthorized)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AppError::Unauthorized)
}

fn sha2_hash(token: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(token.as_bytes()).to_vec()
}

fn is_valid_map_style(value: &str) -> bool {
    matches!(
        value,
        "follow-theme" | "positron" | "bright" | "liberty" | "dark" | "fiord" | "3d"
    )
}

fn default_theme_preferences() -> ThemePreferencesPayload {
    ThemePreferencesPayload {
        mode: "dark".to_string(),
        palette: "classic".to_string(),
    }
}

async fn issue_refresh_token<'e, E>(executor: E, user_id: Uuid) -> Result<String, AppError>
where
    E: Executor<'e, Database = Postgres>,
{
    Ok(issue_new_session(executor, user_id).await?.0)
}

pub fn router() -> Router<AppState> {
    [
        ("/auth/setup", get(setup)),
        ("/auth/register", post(register)),
        (
            "/auth/account-invitations/preview",
            post(preview_account_invitation),
        ),
        (
            "/auth/account-invitations/accept",
            post(accept_account_invitation),
        ),
        (
            "/auth/account-invitations/oidc/start",
            post(oidc_invitation_start),
        ),
        ("/auth/login", post(login)),
        ("/auth/bootstrap", post(bootstrap)),
        ("/auth/refresh", post(refresh)),
        ("/auth/logout", post(logout)),
        ("/auth/config", get(auth_config)),
        ("/auth/oidc/start", post(oidc_start)),
        ("/auth/oidc/callback", get(oidc_callback)),
    ]
    .into_iter()
    .fold(Router::new(), |routes, (path, handler)| {
        routes.route(path, handler)
    })
}

pub fn metadata_router() -> Router<AppState> {
    [
        ("/auth/me", get(me)),
        (
            "/auth/preferences",
            get(get_preferences).put(update_preferences),
        ),
        ("/auth/identities", get(identities)),
        (
            "/auth/preferences/map-style",
            axum::routing::put(update_map_style),
        ),
        (
            "/auth/preferences/chart-favorites",
            get(get_chart_favorites).put(update_chart_favorite),
        ),
    ]
    .into_iter()
    .fold(Router::new(), |routes, (path, handler)| {
        routes.route(path, handler)
    })
}

pub fn protected_router() -> Router<AppState> {
    [
        ("/auth/password", post(change_password)),
        ("/auth/password/oidc/start", post(oidc_password_start)),
        ("/auth/oidc/link/start", post(oidc_link_start)),
        ("/auth/oidc/unlink", post(oidc_unlink)),
    ]
    .into_iter()
    .fold(Router::new(), |routes, (path, handler)| {
        routes.route(path, handler)
    })
}

async fn setup(State(state): State<AppState>) -> Result<Json<SetupResponse>, AppError> {
    let setup_required =
        sqlx::query_scalar::<_, bool>("SELECT NOT EXISTS(SELECT 1 FROM panda.users)")
            .fetch_one(&state.pool)
            .await?;
    let response = SetupResponse {
        setup_proof_available: state.config.setup_proof_available(),
        setup_proof_required: state.config.is_production() && setup_required,
        setup_required,
    };
    Ok(Json(response))
}

async fn auth_config(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    let settings = authentication_settings::load_effective(&state.pool, &state.age_key).await?;
    let mut payload = serde_json::json!({
        "password_login_enabled": settings.password_login_enabled,
        "oidc_auto_login": settings.oidc_auto_login,
        "button_label": settings.button_label,
    });
    let available = settings.oidc_enabled && oidc::provider_configuration_ready(&settings);
    for key in ["oidc_enabled", "oidc_ready"] {
        payload[key] = available.into();
    }
    Ok(Json(payload))
}

fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    let header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    header
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn auth_cookie(name: &str, value: &str, max_age: u64, allow_insecure: bool) -> String {
    let mut attributes = vec![format!("{name}={value}"), "HttpOnly".into()];
    if !allow_insecure {
        attributes.push("Secure".into());
    }
    attributes.extend([
        "SameSite=Lax".into(),
        "Path=/v1/auth".into(),
        format!("Max-Age={max_age}"),
    ]);
    attributes.join("; ")
}

fn refresh_cookie(value: &str, max_age: u64, allow_insecure: bool) -> String {
    auth_cookie("refresh_token", value, max_age, allow_insecure)
}

fn oidc_state_cookie(value: &str, max_age: u64, allow_insecure: bool) -> String {
    auth_cookie("oidc_state", value, max_age, allow_insecure)
}

fn session_response(
    state: &AppState,
    refresh: &str,
    access_token: String,
    default_vehicle_id: Option<Uuid>,
    status: StatusCode,
) -> Response {
    let payload = AccessTokenResponse {
        access_token,
        expires_in: 900,
        default_vehicle_id,
    };
    (
        status,
        [(
            SET_COOKIE,
            refresh_cookie(
                refresh,
                2_592_000,
                state.config.allows_insecure_refresh_cookies(),
            ),
        )],
        Json(payload),
    )
        .into_response()
}

fn cleared_session_response(state: &AppState) -> Response {
    (
        StatusCode::NO_CONTENT,
        [(
            SET_COOKIE,
            refresh_cookie("", 0, state.config.allows_insecure_refresh_cookies()),
        )],
    )
        .into_response()
}

fn random_refresh_token() -> String {
    use rand::distributions::{Alphanumeric, DistString};
    Alphanumeric.sample_string(&mut rand::thread_rng(), 48)
}

async fn issue_new_session<'e, E>(executor: E, user_id: Uuid) -> Result<(String, Uuid), AppError>
where
    E: Executor<'e, Database = Postgres>,
{
    let session_id = Uuid::new_v4();
    let refresh = random_refresh_token();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "WITH eligible AS MATERIALIZED (
            SELECT id FROM panda.users WHERE id=$1 AND NOT is_disabled FOR SHARE
         ), session AS (
            INSERT INTO panda.session_families(id,user_id) SELECT $2,id FROM eligible RETURNING id,user_id
         ) INSERT INTO panda.refresh_tokens(token_hash,user_id,expires_at,family_id)
           SELECT $3,user_id,$4,id FROM session RETURNING family_id",
    ).bind(user_id).bind(session_id).bind(sha2_hash(&refresh))
        .bind(chrono::Utc::now() + chrono::Duration::days(30)).fetch_optional(executor).await?;
    match inserted {
        Some(id) => Ok((refresh, id)),
        None => Err(AppError::Unauthorized),
    }
}

async fn enrollment_response(state: &AppState, user_id: Uuid) -> Result<Response, AppError> {
    let (refresh, sid) = issue_new_session(&state.pool, user_id).await?;
    let access = issue_session_access_token(user_id, None, Some(sid), &state.jwt_keys)?;
    Ok(session_response(
        state,
        &refresh,
        access,
        None,
        StatusCode::CREATED,
    ))
}

fn account_insert_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(ref detail) = error {
        if detail.constraint() == Some("users_email_key") {
            return AppError::Validation("email already registered".into());
        }
    }
    AppError::Database(error)
}

async fn register(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<RegisterBody>,
) -> Result<Response, AppError> {
    match (
        body.email.len(),
        body.email.contains('@'),
        password_meets_minimum(&body.password),
    ) {
        (255.., _, _) => return Err(AppError::Validation("email too long".into())),
        (0, _, _) | (_, false, _) | (_, _, false) => {
            return Err(AppError::Validation(
                "valid email required, password min 12 chars".into(),
            ))
        }
        _ => {}
    }
    let password = argon2_hash(&body.password)?;
    let mut transaction = state.pool.begin().await?;
    transaction
        .execute("LOCK TABLE panda.users IN SHARE ROW EXCLUSIVE MODE")
        .await?;
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM panda.users)")
        .fetch_one(&mut *transaction)
        .await?;
    if occupied {
        return Err(AppError::Forbidden);
    }
    if state.config.is_production() {
        match body.setup_token.as_deref() {
            None => return Err(AppError::SetupProofRequired),
            Some(proof) if !state.config.verify_setup_token(proof)? => {
                return Err(AppError::SetupProofInvalid)
            }
            Some(_) => {}
        }
    }
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO panda.users(email,password_hash,role) VALUES($1,$2,'super_user') RETURNING id",
    )
    .bind(body.email.to_lowercase().trim())
    .bind(password)
    .fetch_one(&mut *transaction)
    .await
    .map_err(account_insert_error)?;
    // A failed preference insert still poisons this transaction, as in the original contract.
    let _ = sqlx::query(
        "INSERT INTO panda.user_preferences(user_id) VALUES($1) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await;
    SecurityAuditEvent::success("setup_claimed", Some(user_id))
        .target(format!("user:{user_id}"))
        .metadata(serde_json::json!({"role":"super_user"}))
        .request_id_from_headers(&headers)
        .record_tx(&mut transaction)
        .await?;
    transaction.commit().await?;
    enrollment_response(&state, user_id).await
}

#[derive(sqlx::FromRow)]
struct PasswordAccount {
    id: Uuid,
    password_hash: Option<String>,
    auth_methods: String,
    is_disabled: bool,
}

async fn login(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<Response, AppError> {
    if !authentication_settings::load_effective(&state.pool, &state.age_key)
        .await?
        .password_login_enabled
    {
        return Err(AppError::Validation("PASSWORD_LOGIN_DISABLED".into()));
    }
    let email = body.email.trim().to_lowercase();
    let account = sqlx::query_as::<_, PasswordAccount>(
        "SELECT id,password_hash,auth_methods,is_disabled FROM panda.users WHERE email=$1",
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await?;
    let verification = verify_password(
        &body.password,
        account
            .as_ref()
            .and_then(|a| a.password_hash.as_deref())
            .unwrap_or(DUMMY_HASH),
    );
    if let Err(error) = verification {
        tracing::warn!(email = %email, reason = "invalid_credentials", "auth.login_failed");
        if account.is_some() {
            if let Err(audit_error) = SecurityAuditEvent::failure("login_failure", None)
                .metadata(serde_json::json!({"known_account":true}))
                .request_id_from_headers(&headers)
                .record(&state.pool)
                .await
            {
                tracing::error!(error = ?audit_error, "auth.login_failure_audit_failed");
            }
        }
        return Err(error);
    }
    let account = match account {
        Some(account) if account.auth_methods != "sso" => account,
        _ => return Err(AppError::Unauthorized),
    };
    if account.is_disabled {
        tracing::warn!(email = %email, user_id = %account.id, reason = "disabled_account", "auth.login_failed");
        return Err(AppError::Forbidden);
    }
    let default_vehicle = get_default_vehicle_id(&state.pool, account.id).await?;
    let mut transaction = state.pool.begin().await?;
    let (refresh, session) = issue_new_session(&mut *transaction, account.id).await?;
    let access =
        issue_session_access_token(account.id, default_vehicle, Some(session), &state.jwt_keys)?;
    SecurityAuditEvent::success("login_success", Some(account.id))
        .target(format!("user:{}", account.id))
        .request_id_from_headers(&headers)
        .record_tx(&mut transaction)
        .await?;
    transaction.commit().await?;
    Ok(session_response(
        &state,
        &refresh,
        access,
        default_vehicle,
        StatusCode::OK,
    ))
}

#[derive(sqlx::FromRow)]
struct LockedSession {
    id: Uuid,
    user_id: Uuid,
    family_revoked: bool,
}

async fn lock_cookie_session(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    hash: &[u8],
) -> Result<Option<LockedSession>, AppError> {
    Ok(sqlx::query_as(
        "SELECT f.id,f.user_id,f.revoked_at IS NOT NULL AS family_revoked
         FROM panda.session_families f JOIN panda.refresh_tokens t ON t.family_id=f.id
         WHERE t.token_hash=$1 FOR UPDATE OF f",
    )
    .bind(hash)
    .fetch_optional(&mut **transaction)
    .await?)
}

async fn refresh(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, AppError> {
    match refresh_from_cookie(&state, &headers).await? {
        Some(response) => Ok(response),
        None => Err(AppError::Unauthorized),
    }
}

async fn bootstrap(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, AppError> {
    Ok(refresh_from_cookie(&state, &headers)
        .await?
        .unwrap_or_else(|| cleared_session_response(&state)))
}

async fn refresh_from_cookie(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<Option<Response>, AppError> {
    let hash = match cookie_value(headers, "refresh_token") {
        None => return Ok(None),
        Some(raw) => sha2_hash(raw),
    };
    let mut transaction = state.pool.begin().await?;
    let Some(session) = lock_cookie_session(&mut transaction, &hash).await? else {
        return Ok(None);
    };
    let token: (bool, bool, bool) = sqlx::query_as(
        "SELECT consumed_at IS NOT NULL,revoked_at IS NOT NULL,expires_at<=now()
         FROM panda.refresh_tokens WHERE token_hash=$1 FOR UPDATE",
    )
    .bind(&hash)
    .fetch_one(&mut *transaction)
    .await?;
    match token {
        (true, _, _) => {
            crate::services::sessions::revoke_family(&mut transaction, session.id).await?;
            transaction.commit().await?;
            if let Err(error) =
                SecurityAuditEvent::failure("refresh_token_replay", Some(session.user_id))
                    .target(format!("session:{}", session.id))
                    .request_id_from_headers(headers)
                    .record(&state.pool)
                    .await
            {
                tracing::error!(
                    ?error,
                    "refresh replay audit failed after session revocation"
                );
            }
            return Ok(None);
        }
        (_, true, _) | (_, _, true) => return Ok(None),
        _ if session.family_revoked => return Ok(None),
        _ => {}
    }
    let enabled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.users WHERE id=$1 AND NOT is_disabled)",
    )
    .bind(session.user_id)
    .fetch_one(&mut *transaction)
    .await?;
    if !enabled {
        return Ok(None);
    }
    let default_vehicle: Option<Uuid> = sqlx::query_scalar(
        "SELECT COALESCE((SELECT vehicle_id FROM panda.vehicle_memberships WHERE user_id=$1 AND is_default LIMIT 1),default_vehicle_id)
         FROM panda.users WHERE id=$1",
    ).bind(session.user_id).fetch_one(&mut *transaction).await?;
    let replacement = random_refresh_token();
    sqlx::query(
        "WITH parent AS (
            UPDATE panda.refresh_tokens SET consumed_at=now(),revoked_at=now() WHERE token_hash=$1
            RETURNING token_hash
         ) INSERT INTO panda.refresh_tokens(token_hash,user_id,family_id,parent_hash,expires_at)
           SELECT $2,$3,$4,token_hash,now()+INTERVAL '30 days' FROM parent",
    )
    .bind(&hash)
    .bind(sha2_hash(&replacement))
    .bind(session.user_id)
    .bind(session.id)
    .execute(&mut *transaction)
    .await?;
    let access = issue_session_access_token(
        session.user_id,
        default_vehicle,
        Some(session.id),
        &state.jwt_keys,
    )?;
    transaction.commit().await?;
    Ok(Some(session_response(
        state,
        &replacement,
        access,
        default_vehicle,
        StatusCode::OK,
    )))
}

async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let Some(raw) = cookie_value(&headers, "refresh_token") else {
        return Ok(cleared_session_response(&state));
    };
    let mut transaction = state.pool.begin().await?;
    if let Some(session) = lock_cookie_session(&mut transaction, &sha2_hash(raw)).await? {
        crate::services::sessions::revoke_family(&mut transaction, session.id).await?;
    }
    transaction.commit().await?;
    Ok(cleared_session_response(&state))
}

enum PasswordMutation<'a> {
    Replace { current: &'a str, new: &'a str },
    Initialize { verified_hash: &'a str },
}

async fn mutate_password(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    mutation: PasswordMutation<'_>,
    headers: &axum::http::HeaderMap,
) -> Result<(), AppError> {
    let mut transaction = pool.begin().await?;
    let (stored, methods): (Option<String>, String) =
        sqlx::query_as("SELECT password_hash,auth_methods FROM panda.users WHERE id=$1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(AppError::NotFound)?;
    if methods == "sso" {
        return Err(AppError::Forbidden);
    }
    let (replacement, event) = match mutation {
        PasswordMutation::Replace { current, new } => {
            let stored = stored
                .ok_or_else(|| AppError::Validation("set a password before changing it".into()))?;
            verify_password(current, &stored)
                .map_err(|_| AppError::Validation("current password is incorrect".into()))?;
            (hash_password(new)?, "password_changed")
        }
        PasswordMutation::Initialize { verified_hash } => {
            if stored.is_some() {
                return Err(AppError::Conflict(
                    "this account already has a password".into(),
                ));
            }
            (verified_hash.to_owned(), "password_set_with_oidc")
        }
    };
    sqlx::query("UPDATE panda.users SET password_hash=$2 WHERE id=$1")
        .bind(user_id)
        .bind(replacement)
        .execute(&mut *transaction)
        .await?;
    crate::services::sessions::revoke_user_sessions(&mut transaction, user_id).await?;
    SecurityAuditEvent::success(event, Some(user_id))
        .target(format!("user:{user_id}"))
        .metadata(serde_json::json!({"refresh_sessions_revoked":true}))
        .request_id_from_headers(headers)
        .record_tx(&mut transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn change_password(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: axum::http::HeaderMap,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Response, AppError> {
    require_password_length(&body.new_password)?;
    mutate_password(
        &state.pool,
        auth.user_id,
        PasswordMutation::Replace {
            current: &body.current_password,
            new: &body.new_password,
        },
        &headers,
    )
    .await?;
    Ok(cleared_session_response(&state))
}

async fn set_initial_password_after_oidc(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    password_hash: &str,
    headers: &axum::http::HeaderMap,
) -> Result<(), AppError> {
    mutate_password(
        pool,
        user_id,
        PasswordMutation::Initialize {
            verified_hash: password_hash,
        },
        headers,
    )
    .await
}

fn require_password_length(value: &str) -> Result<(), AppError> {
    if password_meets_minimum(value) {
        Ok(())
    } else {
        Err(AppError::Validation("password min 12 chars".into()))
    }
}

#[derive(sqlx::FromRow)]
struct Invitation {
    id: Uuid,
    invitee_email: String,
    auth_methods: String,
    vehicle_id: Option<Uuid>,
    expires_at: chrono::DateTime<chrono::Utc>,
    accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn invitation_status(revoked: bool, accepted: bool, expired: bool) -> Result<(), AppError> {
    let reason = [
        (revoked, "invitation revoked"),
        (accepted, "invitation already accepted"),
        (expired, "invitation expired"),
    ]
    .into_iter()
    .find_map(|(blocked, reason)| blocked.then_some(reason));
    match reason {
        Some(reason) => Err(AppError::Validation(reason.into())),
        None => Ok(()),
    }
}

fn validate_account_invitation(row: &sqlx::postgres::PgRow) -> Result<(), AppError> {
    invitation_status(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("revoked_at")
            .is_some(),
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("accepted_at")
            .is_some(),
        row.get::<chrono::DateTime<chrono::Utc>, _>("expires_at") <= chrono::Utc::now(),
    )
}

impl Invitation {
    fn validate(&self) -> Result<(), AppError> {
        invitation_status(
            self.revoked_at.is_some(),
            self.accepted_at.is_some(),
            self.expires_at <= chrono::Utc::now(),
        )
    }
}

async fn preview_account_invitation(
    State(state): State<AppState>,
    Json(body): Json<InvitationTokenBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let row = sqlx::query("SELECT invitee_email,auth_methods,expires_at,accepted_at,revoked_at FROM panda.account_invitations WHERE token_hash=$1")
        .bind(sha2_hash(body.token.trim())).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    validate_account_invitation(&row)?;
    let settings = authentication_settings::load_effective(&state.pool, &state.age_key).await?;
    let response = serde_json::json!({
        "email": row.get::<String,_>("invitee_email"),
        "auth_methods": row.get::<String,_>("auth_methods"),
        "expires_at": row.get::<chrono::DateTime<chrono::Utc>,_>("expires_at"),
        "password_available": settings.password_login_enabled,
        "sso_available": settings.oidc_enabled && oidc::provider_configuration_ready(&settings),
        "button_label": settings.button_label,
    });
    Ok(Json(response))
}

async fn invitation_vehicle_ids(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    invitation: &Invitation,
) -> Result<Vec<Uuid>, AppError> {
    let assigned: Vec<Uuid> = sqlx::query_scalar("SELECT vehicle_id FROM panda.account_invitation_vehicles WHERE invitation_id=$1 ORDER BY vehicle_id")
        .bind(invitation.id).fetch_all(&mut **transaction).await?;
    Ok(if assigned.is_empty() {
        invitation.vehicle_id.into_iter().collect()
    } else {
        assigned
    })
}

async fn activate_invited_account(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    invitation: &Invitation,
    email: &str,
    password: Option<&str>,
    identity: Option<&oidc::VerifiedIdentity>,
    headers: Option<&axum::http::HeaderMap>,
) -> Result<Uuid, AppError> {
    let vehicles = invitation_vehicle_ids(transaction, invitation).await?;
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO panda.users(email,password_hash,auth_methods,role) VALUES($1,$2,$3,'user') RETURNING id",
    ).bind(email).bind(password).bind(&invitation.auth_methods).fetch_one(&mut **transaction).await;
    let user_id = match identity {
        Some(_) => inserted?,
        None => inserted.map_err(account_insert_error)?,
    };
    sqlx::query("INSERT INTO panda.user_preferences(user_id) VALUES($1)")
        .bind(user_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("INSERT INTO panda.vehicle_memberships(vehicle_id,user_id,role,is_default) SELECT unnest($1::uuid[]),$2,'viewer',FALSE")
        .bind(&vehicles).bind(user_id).execute(&mut **transaction).await?;
    sqlx::query(
        "INSERT INTO panda.vehicle_user_settings(vehicle_id,user_id) SELECT unnest($1::uuid[]),$2",
    )
    .bind(&vehicles)
    .bind(user_id)
    .execute(&mut **transaction)
    .await?;
    if let Some(identity) = identity {
        attach_identity(transaction, user_id, identity, Some(email)).await?;
    }
    sqlx::query("UPDATE panda.account_invitations SET accepted_at=now(),created_user_id=$2,updated_at=now() WHERE id=$1")
        .bind(invitation.id).bind(user_id).execute(&mut **transaction).await?;
    let event = SecurityAuditEvent::success("account_invitation_accepted", Some(user_id))
        .target(format!("account_invitation:{}", invitation.id));
    match headers {
        Some(headers) => event.request_id_from_headers(headers),
        None => event,
    }
    .record_tx(transaction)
    .await?;
    Ok(user_id)
}

async fn accept_account_invitation(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<AcceptAccountInvitationBody>,
) -> Result<Response, AppError> {
    require_password_length(&body.password)?;
    let password = hash_password(&body.password)?;
    let mut transaction = state.pool.begin().await?;
    let invitation: Invitation = sqlx::query_as("SELECT id,invitee_email,auth_methods,vehicle_id,expires_at,accepted_at,revoked_at FROM panda.account_invitations WHERE token_hash=$1 FOR UPDATE")
        .bind(sha2_hash(body.token.trim())).fetch_optional(&mut *transaction).await?.ok_or(AppError::NotFound)?;
    invitation.validate()?;
    let settings =
        authentication_settings::load_effective(&mut *transaction, &state.age_key).await?;
    if invitation.auth_methods == "sso" || !settings.password_login_enabled {
        return Err(AppError::Validation(
            "invitation does not allow password activation".into(),
        ));
    }
    let user = activate_invited_account(
        &mut transaction,
        &invitation,
        &invitation.invitee_email,
        Some(&password),
        None,
        Some(&headers),
    )
    .await?;
    transaction.commit().await?;
    enrollment_response(&state, user).await
}

async fn accept_oidc_invitation(
    pool: &sqlx::PgPool,
    invitation_id: Uuid,
    identity: &oidc::VerifiedIdentity,
) -> Result<Uuid, AppError> {
    let email = verified_email(identity)?;
    let mut transaction = pool.begin().await?;
    let invitation: Invitation = sqlx::query_as("SELECT id,invitee_email,auth_methods,vehicle_id,expires_at,accepted_at,revoked_at FROM panda.account_invitations WHERE id=$1 FOR UPDATE")
        .bind(invitation_id).fetch_optional(&mut *transaction).await?.ok_or(AppError::NotFound)?;
    invitation.validate()?;
    if invitation.auth_methods == "password"
        || !invitation.invitee_email.eq_ignore_ascii_case(email)
    {
        return Err(AppError::Forbidden);
    }
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM panda.users WHERE lower(email)=lower($1) FOR UPDATE")
            .bind(email)
            .fetch_optional(&mut *transaction)
            .await?;
    if existing.is_some() {
        return Err(AppError::Conflict("email already registered".into()));
    }
    let linked: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM panda.user_oidc_identities WHERE issuer=$1 AND subject=$2 FOR UPDATE",
    )
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await?;
    if linked.is_some() {
        return Err(identity_conflict());
    }
    let user = activate_invited_account(
        &mut transaction,
        &invitation,
        email,
        None,
        Some(identity),
        None,
    )
    .await?;
    transaction.commit().await?;
    Ok(user)
}

fn verified_email(identity: &oidc::VerifiedIdentity) -> Result<&str, AppError> {
    match (identity.email_verified, identity.email.as_deref()) {
        (true, Some(email)) => Ok(email),
        _ => Err(AppError::Forbidden),
    }
}

fn identity_conflict() -> AppError {
    AppError::Conflict("this SSO identity is already linked to another account".into())
}

async fn attach_identity(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    user: Uuid,
    identity: &oidc::VerifiedIdentity,
    email: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO panda.user_oidc_identities(issuer,subject,user_id,email,last_login_at) VALUES($1,$2,$3,$4,now())")
        .bind(&identity.issuer).bind(&identity.subject).bind(user).bind(email).execute(&mut **transaction).await?;
    Ok(())
}

enum IdentityAction {
    Touch(Uuid),
    Attach(Uuid),
    Provision,
}

async fn resolve_oidc_identity(
    pool: &sqlx::PgPool,
    settings: &authentication_settings::EffectiveAuthenticationSettings,
    transaction: &oidc::Transaction,
    identity: &oidc::VerifiedIdentity,
) -> Result<Uuid, AppError> {
    let mut database = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(lower($1),0))")
        .bind(identity.email.as_deref().unwrap_or(&identity.subject))
        .execute(&mut *database)
        .await?;
    let mapped: Option<(Uuid, bool, String)> = sqlx::query_as(
        "SELECT i.user_id,u.is_disabled,u.auth_methods FROM panda.user_oidc_identities i
         JOIN panda.users u ON u.id=i.user_id WHERE i.issuer=$1 AND i.subject=$2 FOR UPDATE OF i,u",
    )
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *database)
    .await?;
    let action = match mapped {
        Some((_, true, _)) => {
            return Err(oidc_resolution_denied(
                "linked_account_disabled",
                AppError::Forbidden,
            ))
        }
        Some((_, _, ref methods)) if methods == "password" => return Err(AppError::Forbidden),
        Some((user, _, _)) => {
            if transaction.link && transaction.user_id != Some(user) {
                return Err(oidc_resolution_denied(
                    "identity_linked_to_other_account",
                    identity_conflict(),
                ));
            }
            IdentityAction::Touch(user)
        }
        None if transaction.link => {
            if transaction.pending_password_hash.is_some() {
                return Err(oidc_resolution_denied(
                    "reauthentication_identity_not_linked",
                    AppError::Forbidden,
                ));
            }
            let user = transaction.user_id.ok_or_else(|| {
                oidc_resolution_denied("link_account_missing", AppError::Unauthorized)
            })?;
            let methods: Option<String> = sqlx::query_scalar(
                "SELECT auth_methods FROM panda.users WHERE id=$1 AND NOT is_disabled",
            )
            .bind(user)
            .fetch_optional(&mut *database)
            .await?;
            match methods.as_deref() {
                Some("password") => return Err(AppError::Forbidden),
                None => {
                    return Err(oidc_resolution_denied(
                        "link_account_disabled_or_missing",
                        AppError::Forbidden,
                    ))
                }
                Some(_) => IdentityAction::Attach(user),
            }
        }
        None => {
            let email = verified_email(identity)
                .map_err(|error| oidc_resolution_denied("email_missing_or_unverified", error))?;
            if !oidc::domain_allowed(email, &settings.allowed_email_domains) {
                return Err(oidc_resolution_denied(
                    "email_domain_denied",
                    AppError::Forbidden,
                ));
            }
            let existing: Option<(Uuid, bool, String)> = sqlx::query_as("SELECT id,is_disabled,auth_methods FROM panda.users WHERE lower(email)=lower($1) FOR UPDATE")
                .bind(email).fetch_optional(&mut *database).await?;
            match existing {
                Some((_, true, _)) => {
                    return Err(oidc_resolution_denied(
                        "existing_account_disabled",
                        AppError::Forbidden,
                    ))
                }
                Some((_, _, ref methods)) if methods == "password" => {
                    return Err(AppError::Forbidden)
                }
                Some((user, _, _)) if settings.auto_link_verified_email => {
                    IdentityAction::Attach(user)
                }
                Some(_) => {
                    return Err(oidc_resolution_denied(
                        "existing_account_auto_link_disabled",
                        AppError::Forbidden,
                    ))
                }
                None if settings.auto_signup => IdentityAction::Provision,
                None => {
                    return Err(oidc_resolution_denied(
                        "account_missing_auto_signup_disabled",
                        AppError::Forbidden,
                    ))
                }
            }
        }
    };
    let user = match action {
        IdentityAction::Touch(user) => {
            sqlx::query("UPDATE panda.user_oidc_identities SET email=$3,last_login_at=now() WHERE issuer=$1 AND subject=$2")
                .bind(&identity.issuer).bind(&identity.subject).bind(&identity.email).execute(&mut *database).await?;
            user
        }
        action => {
            let user = match action {
                IdentityAction::Attach(user) => user,
                IdentityAction::Provision => {
                    provision_oidc_account(&mut database, verified_email(identity)?).await?
                }
                IdentityAction::Touch(_) => unreachable!(),
            };
            attach_identity(&mut database, user, identity, identity.email.as_deref()).await?;
            user
        }
    };
    database.commit().await?;
    Ok(user)
}

fn oidc_resolution_denied(reason: &'static str, error: AppError) -> AppError {
    tracing::warn!(target: "auth.oidc", stage = "identity_resolution", reason, "OIDC identity rejected");
    error
}

async fn provision_oidc_account(
    database: &mut sqlx::Transaction<'_, Postgres>,
    email: &str,
) -> Result<Uuid, AppError> {
    let pending: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM panda.account_invitations WHERE lower(invitee_email)=lower($1)
         AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now()
         ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(email)
    .fetch_optional(&mut **database)
    .await?;
    if pending.is_some() {
        return Err(AppError::Forbidden);
    }
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT id FROM panda.users WHERE role='super_user' AND NOT is_disabled LIMIT 1 FOR KEY SHARE")
        .fetch_optional(&mut **database).await?;
    if owner.is_none() {
        return Err(oidc_resolution_denied(
            "no_active_owner_for_signup",
            AppError::Forbidden,
        ));
    }
    let user: Uuid = sqlx::query_scalar(
        "WITH account AS (INSERT INTO panda.users(email,password_hash,role) VALUES($1,NULL,'user') RETURNING id)
         INSERT INTO panda.user_preferences(user_id) SELECT id FROM account RETURNING user_id",
    ).bind(email).fetch_one(&mut **database).await?;
    Ok(user)
}

async fn oidc_start(
    State(state): State<AppState>,
    Json(body): Json<OidcStartBody>,
) -> Result<Response, AppError> {
    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM panda.users WHERE role='super_user' AND NOT is_disabled LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;
    if oidc_public_start_available(owner.is_some()) {
        begin_oidc(
            &state,
            oidc::validate_return_to(body.return_to.as_deref())?,
            None,
            None,
            None,
        )
        .await
    } else {
        Err(AppError::NotFound)
    }
}

async fn oidc_invitation_start(
    State(state): State<AppState>,
    Json(body): Json<OidcInvitationStartBody>,
) -> Result<Response, AppError> {
    let settings = authentication_settings::load_effective(&state.pool, &state.age_key).await?;
    if !settings.oidc_enabled || !oidc::provider_configuration_ready(&settings) {
        return Err(AppError::NotFound);
    }
    let invitation: Invitation = sqlx::query_as("SELECT id,invitee_email,auth_methods,vehicle_id,expires_at,accepted_at,revoked_at FROM panda.account_invitations WHERE token_hash=$1")
        .bind(sha2_hash(body.token.trim())).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    invitation.validate()?;
    match invitation.auth_methods.as_str() {
        "password" => Err(AppError::Validation("invitation does not allow SSO".into())),
        _ => begin_oidc(&state, "/".into(), None, None, Some(invitation.id)).await,
    }
}

async fn oidc_link_start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<OidcStartBody>,
) -> Result<Response, AppError> {
    match sqlx::query_scalar::<_, String>(
        "SELECT auth_methods FROM panda.users WHERE id=$1 AND NOT is_disabled",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .as_deref()
    {
        None => Err(AppError::NotFound),
        Some("password") => Err(AppError::Forbidden),
        Some(_) => {
            begin_oidc(
                &state,
                oidc::validate_return_to(body.return_to.as_deref())?,
                Some(auth.user_id),
                None,
                None,
            )
            .await
        }
    }
}

async fn oidc_password_start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetPasswordWithOidcBody>,
) -> Result<Response, AppError> {
    require_password_length(&body.new_password)?;
    let account: (bool, String, bool) = sqlx::query_as(
        "SELECT password_hash IS NOT NULL,auth_methods,EXISTS(SELECT 1 FROM panda.user_oidc_identities WHERE user_id=$1)
         FROM panda.users WHERE id=$1 AND NOT is_disabled",
    ).bind(auth.user_id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    match account {
        (true, _, _) => {
            return Err(AppError::Conflict(
                "this account already has a password".into(),
            ))
        }
        (_, ref methods, _) if methods == "sso" => {
            return Err(AppError::Conflict(
                "this account already has a password".into(),
            ))
        }
        (_, _, false) => {
            return Err(AppError::Conflict(
                "link SSO before setting an initial password".into(),
            ))
        }
        _ => {}
    }
    let reauthentication = Some(hash_password(&body.new_password)?);
    begin_oidc(
        &state,
        "/settings?section=account".into(),
        Some(auth.user_id),
        reauthentication,
        None,
    )
    .await
}

async fn begin_oidc(
    state: &AppState,
    return_to: String,
    link_user: Option<Uuid>,
    pending_password_hash: Option<String>,
    invitation_id: Option<Uuid>,
) -> Result<Response, AppError> {
    tracing::info!(target: "auth.oidc", stage = "start", link = link_user.is_some(), "OIDC flow started");
    let mut settings = authentication_settings::load_effective(&state.pool, &state.age_key).await?;
    if !(settings.oidc_enabled || link_user.is_some()) {
        return Err(AppError::NotFound);
    }
    settings.oidc_enabled = true;
    let callback = authentication_settings::oidc_callback_url(settings.public_base_url.as_deref())?;
    let metadata = oidc::discover(&settings).await?;
    let browser = oidc::random_token();
    let transaction = oidc::Transaction {
        return_to,
        user_id: link_user,
        link: link_user.is_some(),
        pending_password_hash,
        invitation_id,
        browser_binding: oidc::browser_binding(&browser),
        state: oidc::random_token(),
        nonce: oidc::random_token(),
        verifier: oidc::random_token(),
    };
    let encoded =
        serde_json::to_string(&transaction).map_err(|error| AppError::Internal(error.into()))?;
    state
        .redis
        .get_multiplexed_async_connection()
        .await?
        .set_ex::<_, _, ()>(format!("panda:oidc:tx:{}", transaction.state), encoded, 600)
        .await?;
    let authorization = oidc::authorization_url(&metadata, &settings, &transaction, &callback)?;
    tracing::info!(target: "auth.oidc", stage = "authorization_prepared", link = transaction.link, "OIDC authorization prepared");
    let body = serde_json::json!({"authorization_url":authorization.as_str()});
    Ok((
        [(
            SET_COOKIE,
            oidc_state_cookie(
                &browser,
                600,
                state.config.allows_insecure_refresh_cookies(),
            ),
        )],
        Json(body),
    )
        .into_response())
}

fn oidc_stage<T>(
    result: Result<T, AppError>,
    stage: &'static str,
    reason: &'static str,
) -> Result<T, AppError> {
    result.inspect_err(|_| {
        tracing::warn!(target: "auth.oidc", stage, reason, "OIDC callback failed");
    })
}

async fn consume_oidc_transaction(
    state: &AppState,
    key: &str,
) -> Result<Option<oidc::Transaction>, AppError> {
    let mut redis = oidc_stage(
        state
            .redis
            .get_multiplexed_async_connection()
            .await
            .map_err(AppError::from),
        "transaction_load",
        "redis_connection_failed",
    )?;
    let raw: Option<String> = oidc_stage(
        redis
            .get_del(format!("panda:oidc:tx:{key}"))
            .await
            .map_err(AppError::from),
        "transaction_load",
        "redis_read_failed",
    )?;
    let result = raw
        .as_deref()
        .and_then(|value| serde_json::from_str(value).ok());
    if result.is_none() {
        let reason = if raw.is_some() {
            "transaction_invalid"
        } else {
            "transaction_missing"
        };
        tracing::warn!(target: "auth.oidc", stage = "transaction_load", reason, "OIDC callback rejected");
    }
    Ok(result)
}

async fn oidc_callback(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(q): axum::extract::Query<OidcCallback>,
) -> Result<Response, AppError> {
    let allow_insecure = state.config.allows_insecure_refresh_cookies();
    let Some(key) = q.state.as_deref() else {
        tracing::warn!(target: "auth.oidc", stage = "callback_input", reason = "state_missing", "OIDC callback rejected");
        return Ok(oidc_callback_error_response(
            false,
            "oidc_expired",
            allow_insecure,
        ));
    };
    let Some(transaction) = consume_oidc_transaction(&state, key).await? else {
        return Ok(oidc_callback_error_response(
            false,
            "oidc_expired",
            allow_insecure,
        ));
    };
    let binding = cookie_value(&headers, "oidc_state").map(oidc::browser_binding);
    let denial = if binding.as_deref() != Some(&transaction.browser_binding) {
        tracing::warn!(target: "auth.oidc", stage = "callback_binding", reason = "browser_binding_mismatch", "OIDC callback rejected");
        Some("oidc_failed")
    } else if q.error.is_some() {
        tracing::info!(target: "auth.oidc", stage = "provider_response", reason = "provider_denied", "OIDC provider denied callback");
        Some("oidc_denied")
    } else {
        None
    };
    if let Some(code) = denial {
        return Ok(oidc_callback_error_response(
            transaction.link,
            code,
            allow_insecure,
        ));
    }
    match finish_oidc_callback(&state, &transaction, q.code.as_deref(), &headers).await {
        Ok(refresh) => {
            let destination = match (
                transaction.pending_password_hash.is_some(),
                transaction.link,
            ) {
                (true, _) => "/settings?section=account&password=set",
                (_, true) => "/settings?section=account&oidc=linked",
                _ => &transaction.return_to,
            };
            tracing::info!(target: "auth.oidc", stage = "success", reason = "callback_complete", link = transaction.link, "OIDC callback completed");
            Ok(oidc_callback_success_response(
                &refresh,
                destination,
                allow_insecure,
            ))
        }
        Err(_) => {
            let audit = SecurityAuditEvent::failure("oidc_callback_failed", transaction.user_id)
                .target("oidc")
                .record(&state.pool)
                .await;
            let _ = oidc_stage(audit, "failure_audit", "record_failed");
            Ok(oidc_callback_error_response(
                transaction.link,
                "oidc_failed",
                allow_insecure,
            ))
        }
    }
}

async fn finish_oidc_callback(
    state: &AppState,
    transaction: &oidc::Transaction,
    code: Option<&str>,
    headers: &axum::http::HeaderMap,
) -> Result<String, AppError> {
    let settings = oidc_stage(
        authentication_settings::load_effective(&state.pool, &state.age_key).await,
        "settings_load",
        "load_failed",
    )?;
    let code = oidc_stage(
        code.ok_or(AppError::Unauthorized),
        "callback_input",
        "code_missing",
    )?;
    let mut provider = settings.clone();
    provider.oidc_enabled |= transaction.link;
    let identity = oidc_stage(
        oidc::exchange_and_verify(&provider, transaction, code).await,
        "exchange_verify",
        "exchange_or_verify_failed",
    )?;
    tracing::info!(target: "auth.oidc", stage = "exchange_verify", reason = "verified", "OIDC identity verified");
    if !oidc::claim_matches(
        &identity,
        settings.required_claim_name.as_deref(),
        settings.required_claim_value.as_deref(),
    ) {
        return oidc_stage(Err(AppError::Forbidden), "required_claim", "claim_mismatch");
    }
    let user = match transaction.invitation_id {
        Some(invitation) => accept_oidc_invitation(&state.pool, invitation, &identity).await,
        None => resolve_oidc_identity(&state.pool, &settings, transaction, &identity).await,
    }.inspect_err(|error| {
        if !matches!(error, AppError::Forbidden | AppError::Unauthorized | AppError::Conflict(_)) {
            tracing::warn!(target: "auth.oidc", stage = "identity_resolution", reason = "resolution_failed", "OIDC callback failed");
        }
    })?;
    tracing::info!(target: "auth.oidc", stage = "identity_resolution", reason = "resolved", "OIDC identity resolved");
    let disabled = oidc_stage(
        sqlx::query_scalar::<_, bool>("SELECT is_disabled FROM panda.users WHERE id=$1")
            .bind(user)
            .fetch_optional(&state.pool)
            .await
            .map_err(AppError::from),
        "account_status",
        "lookup_failed",
    )?
    .unwrap_or(true);
    if disabled {
        return oidc_stage(
            Err(AppError::Forbidden),
            "account_status",
            "account_disabled",
        );
    }
    if let Some(hash) = transaction.pending_password_hash.as_deref() {
        oidc_stage(
            set_initial_password_after_oidc(&state.pool, user, hash, headers).await,
            "password_setup",
            "setup_failed",
        )?;
    }
    let refresh = oidc_stage(
        issue_refresh_token(&state.pool, user).await,
        "refresh_issue",
        "issue_failed",
    )?;
    tracing::info!(target: "auth.oidc", stage = "refresh_issue", reason = "issued", "OIDC refresh session issued");
    if transaction.pending_password_hash.is_none() {
        let event = if transaction.link {
            "oidc_identity_linked"
        } else {
            "oidc_login"
        };
        oidc_stage(
            SecurityAuditEvent::success(event, Some(user))
                .target("oidc")
                .record(&state.pool)
                .await,
            "success_audit",
            "record_failed",
        )?;
    }
    Ok(refresh)
}

fn oidc_callback_success_response(
    refresh: &str,
    destination: &str,
    allow_insecure: bool,
) -> Response {
    oidc_redirect(
        destination,
        vec![
            refresh_cookie(refresh, 2_592_000, allow_insecure),
            oidc_state_cookie("", 0, allow_insecure),
        ],
    )
}

fn oidc_callback_error_response(link: bool, code: &str, allow_insecure: bool) -> Response {
    let path = match link {
        true => "/settings?section=account&error=",
        false => "/login?error=",
    };
    oidc_redirect(
        &format!("{path}{code}"),
        vec![oidc_state_cookie("", 0, allow_insecure)],
    )
}

fn oidc_redirect(destination: &str, cookies: Vec<String>) -> Response {
    let mut response = (
        StatusCode::SEE_OTHER,
        [(axum::http::header::LOCATION, destination)],
    )
        .into_response();
    for cookie in cookies {
        response = (AppendHeaders([(SET_COOKIE, cookie)]), response).into_response();
    }
    response
}

async fn oidc_unlink(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UnlinkOidcBody>,
) -> Result<StatusCode, AppError> {
    let mut transaction = state.pool.begin().await?;
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT password_hash FROM panda.users WHERE id=$1 FOR UPDATE")
            .bind(auth.user_id)
            .fetch_optional(&mut *transaction)
            .await?;
    match stored.flatten() {
        None => {
            return Err(AppError::Conflict(
                "set a password before unlinking your only OIDC sign-in".into(),
            ))
        }
        Some(hash) => verify_password(&body.current_password, &hash)
            .map_err(|_| AppError::Validation("current password is incorrect".into()))?,
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM panda.user_oidc_identities WHERE user_id=$1")
            .bind(auth.user_id)
            .fetch_one(&mut *transaction)
            .await?;
    if count == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("DELETE FROM panda.user_oidc_identities WHERE user_id=$1")
        .bind(auth.user_id)
        .execute(&mut *transaction)
        .await?;
    SecurityAuditEvent::success("oidc_identity_unlinked", Some(auth.user_id))
        .target("oidc")
        .record_tx(&mut transaction)
        .await?;
    transaction
        .commit()
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(AppError::from)
}

async fn identities(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let (password, methods, linked): (bool, String, bool) = sqlx::query_as(
        "SELECT password_hash IS NOT NULL,auth_methods,EXISTS(SELECT 1 FROM panda.user_oidc_identities WHERE user_id=$1) FROM panda.users WHERE id=$1",
    ).bind(auth.user_id).fetch_one(&state.pool).await?;
    let settings = authentication_settings::load_effective(&state.pool, &state.age_key).await?;
    let permissions = (methods != "sso", methods != "password");
    Ok(Json(serde_json::json!({
        "password_configured": password, "oidc_linked": linked,
        "password_setup_allowed": permissions.0, "oidc_link_allowed": permissions.1,
        "oidc_link_available": oidc::provider_configuration_ready(&settings), "button_label": settings.button_label,
    })))
}

async fn me(State(state): State<AppState>, auth: AuthUser) -> Result<impl IntoResponse, AppError> {
    let (email, role, password_configured, oidc_linked): (String, String, bool, bool) = sqlx::query_as(
        "SELECT email,role,password_hash IS NOT NULL,EXISTS(SELECT 1 FROM panda.user_oidc_identities WHERE user_id=$1) FROM panda.users WHERE id=$1",
    ).bind(auth.user_id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    let mut payload = serde_json::json!({"user_id":auth.user_id,"email":email,"role":role,"password_configured":password_configured,"oidc_linked":oidc_linked});
    payload["default_vehicle_id"] =
        serde_json::to_value(get_default_vehicle_id(&state.pool, auth.user_id).await?)
            .map_err(|e| AppError::Internal(e.into()))?;
    Ok(Json(payload))
}

const IMPERIAL_UNITS: [&str; 7] = [
    "miles",
    "mph",
    "fahrenheit",
    "psi",
    "feet",
    "feet",
    "distance_per_energy",
];
const METRIC_UNITS: [&str; 7] = [
    "kilometers",
    "kmh",
    "celsius",
    "kpa",
    "meters",
    "meters",
    "distance_per_energy",
];

fn unit_payload(mode: &str, fields: [&str; 7]) -> UnitPreferencesPayload {
    let [distance_unit, speed_unit, temperature_unit, pressure_unit, altitude_unit, place_radius_unit, efficiency_display] =
        fields.map(str::to_owned);
    UnitPreferencesPayload {
        mode: mode.into(),
        distance_unit,
        speed_unit,
        temperature_unit,
        pressure_unit,
        altitude_unit,
        place_radius_unit,
        efficiency_display,
    }
}

#[allow(clippy::too_many_arguments)]
fn resolved_units_payload(
    mode: &str,
    custom_distance: Option<&str>,
    custom_speed: Option<&str>,
    custom_temperature: Option<&str>,
    custom_pressure: Option<&str>,
    custom_altitude: Option<&str>,
    custom_place_radius: Option<&str>,
    custom_efficiency_display: Option<&str>,
    legacy_distance: &str,
    legacy_temperature: &str,
) -> UnitPreferencesPayload {
    if mode == "custom" {
        let custom = [
            custom_distance,
            custom_speed,
            custom_temperature,
            custom_pressure,
            custom_altitude,
            custom_place_radius,
            custom_efficiency_display,
        ];
        return unit_payload(
            "custom",
            std::array::from_fn(|i| custom[i].unwrap_or(IMPERIAL_UNITS[i])),
        );
    }
    let metric = mode == "metric"
        || legacy_distance.eq_ignore_ascii_case("kilometers")
        || legacy_temperature.eq_ignore_ascii_case("celsius");
    let (mode, fields) = if metric {
        ("metric", METRIC_UNITS)
    } else {
        ("imperial", IMPERIAL_UNITS)
    };
    unit_payload(mode, fields)
}

fn normalize_theme_payload(
    input: ThemePreferencesPayload,
) -> Result<ThemePreferencesPayload, AppError> {
    for (value, allowed, reason) in [
        (
            input.mode.as_str(),
            &["light", "dark", "system"][..],
            "invalid theme mode",
        ),
        (
            input.palette.as_str(),
            &["classic", "rad"][..],
            "invalid theme palette",
        ),
    ] {
        if !allowed.contains(&value) {
            return Err(AppError::Validation(reason.into()));
        }
    }
    Ok(input)
}

fn normalize_units_payload(
    input: UnitPreferencesPayload,
) -> Result<UnitPreferencesPayload, AppError> {
    if !["imperial", "metric", "custom"].contains(&input.mode.as_str()) {
        return Err(AppError::Validation("invalid unit mode".into()));
    }
    let selected = [
        &input.distance_unit,
        &input.speed_unit,
        &input.temperature_unit,
        &input.pressure_unit,
        &input.altitude_unit,
        &input.place_radius_unit,
        &input.efficiency_display,
    ];
    for (i, value) in selected.into_iter().enumerate() {
        let alternate = if i == 6 {
            "energy_per_distance"
        } else {
            METRIC_UNITS[i]
        };
        if value != IMPERIAL_UNITS[i] && value != alternate {
            return Err(AppError::Validation("invalid unit preference value".into()));
        }
    }
    Ok(input)
}

fn stored_preference(row: Option<&sqlx::postgres::PgRow>, field: &str, fallback: &str) -> String {
    row.and_then(|row| row.try_get::<String, _>(field).ok())
        .unwrap_or_else(|| fallback.to_owned())
}

async fn get_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PreferencesResponse>, AppError> {
    let stored = sqlx::query(
        "SELECT unit_mode,distance_unit,temperature_unit,custom_distance_unit,custom_speed_unit,custom_temperature_unit,
         custom_pressure_unit,custom_altitude_unit,custom_place_radius_unit,custom_efficiency_display,theme_mode,theme_palette,map_style
         FROM panda.user_preferences WHERE user_id=$1",
    ).bind(auth.user_id).fetch_optional(&state.pool).await?;
    let row = stored.as_ref();
    let mode = stored_preference(row, "unit_mode", "imperial");
    let legacy_distance = stored_preference(row, "distance_unit", "miles");
    let legacy_temperature = stored_preference(row, "temperature_unit", "fahrenheit");
    let custom = [
        "custom_distance_unit",
        "custom_speed_unit",
        "custom_temperature_unit",
        "custom_pressure_unit",
        "custom_altitude_unit",
        "custom_place_radius_unit",
        "custom_efficiency_display",
    ]
    .map(|field| row.and_then(|row| row.try_get::<Option<String>, _>(field).ok().flatten()));
    let units = resolved_units_payload(
        &mode,
        custom[0].as_deref(),
        custom[1].as_deref(),
        custom[2].as_deref(),
        custom[3].as_deref(),
        custom[4].as_deref(),
        custom[5].as_deref(),
        custom[6].as_deref(),
        &legacy_distance,
        &legacy_temperature,
    );
    let theme = normalize_theme_payload(ThemePreferencesPayload {
        mode: stored_preference(row, "theme_mode", "dark"),
        palette: stored_preference(row, "theme_palette", "classic"),
    })
    .unwrap_or_else(|_| default_theme_preferences());
    let map_style = stored_preference(row, "map_style", "follow-theme");
    Ok(Json(PreferencesResponse {
        units,
        theme,
        map_style: if is_valid_map_style(&map_style) {
            map_style
        } else {
            "follow-theme".into()
        },
    }))
}

async fn update_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PreferencesUpdateBody>,
) -> Result<Json<PreferencesResponse>, AppError> {
    let (units, theme) = match (body.units, body.theme) {
        (None, None) => {
            return Err(AppError::Validation(
                "at least one preference group is required".into(),
            ))
        }
        (units, theme) => {
            let theme = theme.map(normalize_theme_payload).transpose()?;
            (units.map(normalize_units_payload).transpose()?, theme)
        }
    };
    if let Some(units) = units {
        persist_units(&state.pool, auth.user_id, units).await?;
    }
    if let Some(theme) = theme {
        persist_theme(&state.pool, auth.user_id, theme).await?;
    }
    get_preferences(State(state), auth).await
}

async fn persist_units(
    pool: &sqlx::PgPool,
    user: Uuid,
    units: UnitPreferencesPayload,
) -> Result<(), AppError> {
    let legacy = match units.mode.as_str() {
        "metric" => ("kilometers", "celsius"),
        "custom" => (
            units.distance_unit.as_str(),
            units.temperature_unit.as_str(),
        ),
        _ => ("miles", "fahrenheit"),
    };
    let values = serde_json::to_value(&units).map_err(|error| AppError::Internal(error.into()))?;
    sqlx::query(
        "INSERT INTO panda.user_preferences(user_id,unit_mode,distance_unit,temperature_unit,
         custom_distance_unit,custom_speed_unit,custom_temperature_unit,custom_pressure_unit,custom_altitude_unit,custom_place_radius_unit,custom_efficiency_display,updated_at)
         SELECT $1,p.mode,$3,$4,p.distance_unit,p.speed_unit,p.temperature_unit,p.pressure_unit,p.altitude_unit,p.place_radius_unit,p.efficiency_display,now()
         FROM jsonb_to_record($2::jsonb) AS p(mode text,distance_unit text,speed_unit text,temperature_unit text,pressure_unit text,altitude_unit text,place_radius_unit text,efficiency_display text)
         ON CONFLICT(user_id) DO UPDATE SET
         unit_mode=EXCLUDED.unit_mode,distance_unit=EXCLUDED.distance_unit,temperature_unit=EXCLUDED.temperature_unit,
         custom_distance_unit=EXCLUDED.custom_distance_unit,custom_speed_unit=EXCLUDED.custom_speed_unit,
         custom_temperature_unit=EXCLUDED.custom_temperature_unit,custom_pressure_unit=EXCLUDED.custom_pressure_unit,
         custom_altitude_unit=EXCLUDED.custom_altitude_unit,custom_place_radius_unit=EXCLUDED.custom_place_radius_unit,
         custom_efficiency_display=EXCLUDED.custom_efficiency_display,updated_at=now()",
    ).bind(user).bind(values).bind(legacy.0).bind(legacy.1).execute(pool).await?;
    Ok(())
}

async fn persist_theme(
    pool: &sqlx::PgPool,
    user: Uuid,
    theme: ThemePreferencesPayload,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO panda.user_preferences AS current(user_id,theme_mode,theme_palette,theme_selection_kind,theme_builtin_id,theme_custom_id,theme_custom_revision,theme_etag_version,updated_at)
         VALUES($1,$2,$3,'builtin',$3,NULL,NULL,1,now())
         ON CONFLICT(user_id) DO UPDATE SET theme_mode=EXCLUDED.theme_mode,theme_palette=EXCLUDED.theme_palette,
         theme_selection_kind=CASE WHEN current.theme_palette=EXCLUDED.theme_palette THEN current.theme_selection_kind ELSE 'builtin' END,
         theme_builtin_id=CASE WHEN current.theme_palette=EXCLUDED.theme_palette THEN current.theme_builtin_id ELSE EXCLUDED.theme_palette END,
         theme_custom_id=CASE WHEN current.theme_palette=EXCLUDED.theme_palette THEN current.theme_custom_id ELSE NULL END,
         theme_custom_revision=CASE WHEN current.theme_palette=EXCLUDED.theme_palette THEN current.theme_custom_revision ELSE NULL END,
         theme_etag_version=current.theme_etag_version+1,updated_at=now()",
    ).bind(user).bind(theme.mode).bind(theme.palette).execute(pool).await?;
    Ok(())
}

async fn update_map_style(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<MapStyleUpdateBody>,
) -> Result<Json<MapStyleResponse>, AppError> {
    if !is_valid_map_style(&body.map_style) {
        return Err(AppError::Validation("invalid map style".into()));
    }
    let map_style = sqlx::query_scalar(
        "INSERT INTO panda.user_preferences(user_id,map_style,updated_at) VALUES($1,$2,now())
         ON CONFLICT(user_id) DO UPDATE SET map_style=EXCLUDED.map_style,updated_at=now() RETURNING map_style",
    ).bind(auth.user_id).bind(body.map_style).fetch_one(&state.pool).await?;
    Ok(Json(MapStyleResponse { map_style }))
}

async fn get_chart_favorites(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<DashboardChartFavoritesResponse>, AppError> {
    let result: Option<Option<serde_json::Value>> = sqlx::query_scalar(
        "SELECT dashboard_chart_favorites FROM panda.user_preferences WHERE user_id=$1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(DashboardChartFavoritesResponse {
        chart_favorites: result.flatten().unwrap_or_else(|| serde_json::json!({})),
    }))
}

async fn update_chart_favorite(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<DashboardChartFavoriteUpdateBody>,
) -> Result<Json<DashboardChartFavoritesResponse>, AppError> {
    for (value, limit) in [(&body.key, 200), (&body.chart_id, 120)] {
        if !(1..=limit).contains(&value.len()) {
            return Err(AppError::Validation(
                "invalid dashboard chart favorite".into(),
            ));
        }
    }
    let entry = serde_json::json!({body.key: body.chart_id});
    sqlx::query(
        "INSERT INTO panda.user_preferences(user_id,dashboard_chart_favorites,updated_at) VALUES($1,$2,now())
         ON CONFLICT(user_id) DO UPDATE SET dashboard_chart_favorites=COALESCE(panda.user_preferences.dashboard_chart_favorites,'{}'::jsonb)||EXCLUDED.dashboard_chart_favorites,updated_at=now()",
    ).bind(auth.user_id).bind(entry).execute(&state.pool).await?;
    get_chart_favorites(State(state), auth).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn authorization_refresh_issuance_cannot_race_user_deletion_or_disabling() {
        let f = crate::authorization_test_support::Fixture::new().await;
        let user = f.user("user").await;
        let token = issue_refresh_token(&f.state.pool, user).await.unwrap();
        assert_eq!(f.refresh(&token).await.status(), http::StatusCode::OK);
        for delete in [false, true] {
            let user = f.user("user").await;
            let mut tx = f.state.pool.begin().await.unwrap();
            let query = if delete {
                "DELETE FROM panda.users WHERE id = $1"
            } else {
                "UPDATE panda.users SET is_disabled = TRUE WHERE id = $1"
            };
            sqlx::query(query)
                .bind(user)
                .execute(&mut *tx)
                .await
                .unwrap();
            let pool = f.state.pool.clone();
            let pending = tokio::spawn(async move { issue_refresh_token(&pool, user).await });
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            assert!(
                !pending.is_finished(),
                "refresh must wait for the user row lock"
            );
            tx.commit().await.unwrap();
            assert!(matches!(
                pending.await.unwrap(),
                Err(AppError::Unauthorized)
            ));
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM panda.refresh_tokens WHERE user_id = $1")
                    .bind(user)
                    .fetch_one(&f.state.pool)
                    .await
                    .unwrap();
            assert_eq!(count, 0);
        }
        f.cleanup().await;
    }

    #[test]
    fn public_oidc_requires_a_first_owner() {
        assert!(!oidc_public_start_available(false));
        assert!(oidc_public_start_available(true));
    }
    use axum::body::Body;
    use http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    #[test]
    fn register_body_keeps_setup_token_additive() {
        let legacy: RegisterBody = serde_json::from_value(serde_json::json!({
            "email": "owner@example.test",
            "password": "correct-password"
        }))
        .expect("legacy registration body");
        assert!(legacy.setup_token.is_none());

        let protected: RegisterBody = serde_json::from_value(serde_json::json!({
            "email": "owner@example.test",
            "password": "correct-password",
            "setup_token": "not-an-actual-secret"
        }))
        .expect("protected registration body");
        assert!(protected.setup_token.is_some());
    }

    #[test]
    fn setup_response_serializes_additive_proof_flags() {
        let value = serde_json::to_value(SetupResponse {
            setup_required: true,
            setup_proof_required: true,
            setup_proof_available: false,
        })
        .expect("setup response serializes");
        assert_eq!(value["setup_required"], true);
        assert_eq!(value["setup_proof_required"], true);
        assert_eq!(value["setup_proof_available"], false);
    }

    #[test]
    fn theme_preferences_default_to_classic_dark() {
        let value = serde_json::to_value(default_theme_preferences()).expect("theme defaults");
        assert_eq!(value["mode"], "dark");
        assert_eq!(value["palette"], "classic");
    }

    #[test]
    fn theme_preferences_validate_mode_and_palette_independently() {
        let valid = normalize_theme_payload(ThemePreferencesPayload {
            mode: "system".into(),
            palette: "rad".into(),
        })
        .expect("valid theme preference");
        assert_eq!(valid.mode, "system");
        assert_eq!(valid.palette, "rad");

        assert!(normalize_theme_payload(ThemePreferencesPayload {
            mode: "sepia".into(),
            palette: "classic".into(),
        })
        .is_err());
        assert!(normalize_theme_payload(ThemePreferencesPayload {
            mode: "dark".into(),
            palette: "neon".into(),
        })
        .is_err());
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a full router backed by a real database.
    /// Reads DATABASE_URL + REDIS_URL from the environment (set in CI).
    async fn make_app() -> axum::Router {
        use crate::middleware::auth::JwtKeys;
        use std::sync::Arc;
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());

        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let redis = redis::Client::open(redis_url).expect("redis client");

        let (private_pem, public_pem) = generate_test_rsa_keys();
        let jwt_keys = Arc::new(JwtKeys::new(&private_pem, &public_pem).expect("jwt keys"));

        let config = crate::config::Config {
            database_url: database_url.clone(),
            redis_url: "redis://127.0.0.1/".into(),
            jwt_secret: None,
            jwt_public_key: None,
            age_encryption_key: None,
            port: 3001,
            allowed_origins: vec!["http://localhost:3000".into()],
            s3_endpoint: None,
            s3_access_key: None,
            s3_secret_key: None,
            backup_artifact_dir: std::env::temp_dir()
                .join("panda-route-test-backups")
                .to_string_lossy()
                .into_owned(),
            vehicle_image_cache_dir: std::env::temp_dir()
                .join("panda-route-test-vehicle-images")
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
            rivian_persist_raw_events: true,
            rivian_suppress_duplicate_telemetry: true,
            panda_env: None,
            cookie_insecure: None,
            allow_insecure_lan_http_auth: false,
            rate_limit: crate::config::RateLimitConfig::default(),
        };

        let state = AppState {
            pool,
            redis,
            jwt_keys,
            age_key: "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
                .to_string(),
            config,
            nominatim_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            resources: Default::default(),
        };

        crate::routes::build_router(state)
    }

    /// Generate an RSA-2048 key pair in PEM format for testing.
    fn generate_test_rsa_keys() -> (String, String) {
        let keys = crate::keys::generate_keys().expect("generate test keys");
        (keys.jwt_private_pem, keys.jwt_public_pem)
    }

    /// Send a POST request with a JSON body.
    async fn post_json(
        app: axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        app.oneshot(req).await.unwrap()
    }

    /// Send a GET request.
    async fn get(app: axum::Router, uri: &str) -> axum::response::Response {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap()
    }

    async fn seed_oidc_callback_transaction(transaction: &oidc::Transaction) {
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());
        let client = redis::Client::open(redis_url).expect("redis client");
        let mut connection = client
            .get_multiplexed_async_connection()
            .await
            .expect("redis connection");
        connection
            .set_ex::<_, _, ()>(
                format!("panda:oidc:tx:{}", transaction.state),
                serde_json::to_string(transaction).expect("serialize OIDC transaction"),
                600,
            )
            .await
            .expect("seed OIDC transaction");
    }

    async fn oidc_callback_request(
        app: axum::Router,
        uri: &str,
        cookie: Option<&str>,
    ) -> axum::response::Response {
        let mut request = Request::builder().method("GET").uri(uri);
        if let Some(cookie) = cookie {
            request = request.header("cookie", format!("oidc_state={cookie}"));
        }
        app.oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn login_access_token(app: axum::Router, email: &str) -> String {
        let resp = post_json(
            app,
            "/v1/auth/login",
            serde_json::json!({ "email": email, "password": "correctpassword123" }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["access_token"]
            .as_str()
            .expect("login should return an access token")
            .to_string()
    }

    async fn get_authenticated(
        app: axum::Router,
        uri: &str,
        access_token: &str,
    ) -> serde_json::Value {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {access_token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn put_authenticated(
        app: axum::Router,
        uri: &str,
        access_token: &str,
        payload: serde_json::Value,
    ) -> serde_json::Value {
        let req = Request::builder()
            .method("PUT")
            .uri(uri)
            .header("authorization", format!("Bearer {access_token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn seed_test_user(email: &str, password: &str) {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let password_hash = argon2_hash(password).expect("hash test password");
        sqlx::query("INSERT INTO panda.users (email, password_hash, role) VALUES ($1, $2, 'user')")
            .bind(email)
            .bind(password_hash)
            .execute(&pool)
            .await
            .expect("seed test user");
    }

    async fn delete_test_user(email: &str) {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        sqlx::query("DELETE FROM panda.users WHERE email = $1")
            .bind(email)
            .execute(&pool)
            .await
            .expect("delete test user");
    }

    fn test_oidc_settings(
        auto_signup: bool,
        auto_link_verified_email: bool,
    ) -> authentication_settings::EffectiveAuthenticationSettings {
        authentication_settings::EffectiveAuthenticationSettings {
            oidc_enabled: true,
            password_login_enabled: true,
            issuer_url: Some("https://issuer.example".into()),
            public_base_url: Some("https://panda.example".into()),
            client_id: Some("client".into()),
            client_secret: Some("secret".into()),
            button_label: "Sign in with SSO".into(),
            scopes: "openid email profile".into(),
            token_auth_method: "auto".into(),
            auto_signup,
            auto_link_verified_email,
            oidc_auto_login: false,
            allowed_email_domains: vec!["example.com".into()],
            required_claim_name: None,
            required_claim_value: None,
        }
    }

    fn test_oidc_transaction() -> oidc::Transaction {
        oidc::Transaction {
            state: "state".into(),
            browser_binding: "binding".into(),
            nonce: "nonce".into(),
            verifier: "verifier".into(),
            return_to: "/".into(),
            user_id: None,
            link: false,
            pending_password_hash: None,
            invitation_id: None,
        }
    }

    fn test_oidc_identity(email: &str) -> oidc::VerifiedIdentity {
        oidc::VerifiedIdentity {
            issuer: "https://issuer.example".into(),
            subject: Uuid::new_v4().to_string(),
            email: Some(email.into()),
            email_verified: true,
            claims: serde_json::json!({}),
        }
    }

    // ── pure unit tests (no DB needed) ───────────────────────────────────────

    #[test]
    fn map_style_validation_accepts_only_the_public_preference_union() {
        for style in [
            "follow-theme",
            "positron",
            "bright",
            "liberty",
            "dark",
            "fiord",
            "3d",
        ] {
            assert!(is_valid_map_style(style));
        }
        assert!(!is_valid_map_style("satellite"));
        assert!(!is_valid_map_style(""));
    }

    #[test]
    fn register_validation_rejects_empty_email() {
        let email = "";
        let password = "strongpassword123";
        assert!(
            email.is_empty() || !password_meets_minimum(password),
            "expected validation to fire for empty email"
        );
    }

    #[test]
    fn register_validation_rejects_short_password() {
        let email = "user@example.com";
        let password = "short";
        assert!(
            email.is_empty() || !password_meets_minimum(password),
            "expected validation to fire for short password"
        );
    }

    #[test]
    fn register_validation_passes_for_valid_input() {
        let email = "user@example.com";
        let password = "strongpass123";
        assert!(
            !(email.is_empty() || !password_meets_minimum(password)),
            "valid input should not trigger validation error"
        );
    }

    #[test]
    fn password_minimum_is_twelve_characters() {
        assert!(!password_meets_minimum("elevenchars"));
        assert!(password_meets_minimum("twelve-chars"));
    }

    #[test]
    fn refresh_cookie_format_contains_httponly() {
        let cookie = refresh_cookie("mytoken", 3600, false);
        assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly");
        assert!(
            cookie.contains("SameSite=Lax"),
            "cookie must have SameSite=Lax"
        );
        assert!(
            cookie.contains("refresh_token=mytoken"),
            "cookie must contain token value"
        );
        assert!(cookie.contains("Max-Age=3600"), "cookie must set Max-Age");
        assert!(cookie.contains("Secure"), "cookies are secure by default");
    }

    #[test]
    fn refresh_cookie_omits_secure_only_for_explicitly_allowed_lan_http_auth() {
        let cookie = refresh_cookie("mytoken", 3600, true);
        assert!(!cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
    }

    #[test]
    fn refresh_cookie_clear_sets_zero_max_age() {
        let cookie = refresh_cookie("", 0, false);
        assert!(
            cookie.contains("Max-Age=0"),
            "clearing cookie must set Max-Age=0"
        );
        assert!(
            cookie.contains("refresh_token="),
            "clearing cookie must have empty value"
        );
    }

    #[test]
    fn oidc_callback_success_response_preserves_both_session_cookies() {
        let response = oidc_callback_success_response("issued-refresh", "/dashboard", false);
        let cookies: Vec<&str> = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("Set-Cookie is valid ASCII"))
            .collect();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get("location").unwrap(), "/dashboard");
        assert_eq!(cookies.len(), 2);
        assert_eq!(
            cookies
                .iter()
                .filter(|cookie| cookie.starts_with("refresh_token="))
                .count(),
            1
        );
        assert_eq!(
            cookies
                .iter()
                .filter(|cookie| cookie.starts_with("oidc_state="))
                .count(),
            1
        );
        assert!(cookies.iter().any(|cookie| {
            cookie.starts_with("refresh_token=issued-refresh;")
                && cookie.contains("Max-Age=2592000")
        }));
        assert!(cookies
            .iter()
            .any(|cookie| cookie.starts_with("oidc_state=;") && cookie.contains("Max-Age=0")));
    }

    #[test]
    fn oidc_state_cookie_is_secure_by_default() {
        let cookie = oidc_state_cookie("binding", 600, false);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/v1/auth"));
    }

    #[test]
    fn oidc_state_cookie_allows_explicit_http_development() {
        let cookie = oidc_state_cookie("binding", 600, true);
        assert!(!cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
    }

    #[test]
    fn sha2_hash_is_deterministic() {
        let h1 = sha2_hash("hello");
        let h2 = sha2_hash("hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn sha2_hash_differs_for_different_inputs() {
        let h1 = sha2_hash("hello");
        let h2 = sha2_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn argon2_hash_and_verify_roundtrip() {
        let password = "supersecretpassword";
        let hash = argon2_hash(password).expect("hash should succeed");
        assert!(verify_password(password, &hash).is_ok());
    }

    #[test]
    fn verify_password_rejects_wrong_password() {
        let hash = argon2_hash("correctpassword").expect("hash");
        assert!(verify_password("wrongpassword", &hash).is_err());
    }

    // ── integration tests (require DATABASE_URL) ─────────────────────────────
    // Run with: cargo test -- --ignored

    #[tokio::test]
    #[ignore = "requires DATABASE_URL and REDIS_URL"]
    async fn oidc_callback_wrong_browser_cookie_consumes_the_one_time_transaction() {
        let mut transaction = test_oidc_transaction();
        transaction.state = format!("wrong-cookie-{}", Uuid::new_v4());
        transaction.browser_binding = oidc::browser_binding("expected-browser");
        seed_oidc_callback_transaction(&transaction).await;
        let app = make_app().await;

        let wrong_cookie = oidc_callback_request(
            app.clone(),
            &format!("/v1/auth/oidc/callback?state={}", transaction.state),
            Some("different-browser"),
        )
        .await;
        assert_eq!(wrong_cookie.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            wrong_cookie.headers().get("location").unwrap(),
            "/login?error=oidc_failed"
        );

        let replay = oidc_callback_request(
            app,
            &format!("/v1/auth/oidc/callback?state={}", transaction.state),
            Some("expected-browser"),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            replay.headers().get("location").unwrap(),
            "/login?error=oidc_expired"
        );
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL and REDIS_URL"]
    async fn oidc_provider_denial_consumes_the_transaction_without_token_exchange() {
        let browser_cookie = "denied-browser";
        let mut transaction = test_oidc_transaction();
        transaction.state = format!("provider-denial-{}", Uuid::new_v4());
        transaction.browser_binding = oidc::browser_binding(browser_cookie);
        seed_oidc_callback_transaction(&transaction).await;
        let app = make_app().await;

        let denied = oidc_callback_request(
            app.clone(),
            &format!(
                "/v1/auth/oidc/callback?state={}&error=access_denied",
                transaction.state
            ),
            Some(browser_cookie),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            denied.headers().get("location").unwrap(),
            "/login?error=oidc_denied"
        );

        let replay = oidc_callback_request(
            app,
            &format!("/v1/auth/oidc/callback?state={}", transaction.state),
            Some(browser_cookie),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            replay.headers().get("location").unwrap(),
            "/login?error=oidc_expired"
        );
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn me_requires_auth() {
        let app = make_app().await;
        let resp = get(app, "/v1/auth/me").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn register_fails_empty_email_http() {
        let app = make_app().await;
        let resp = post_json(
            app,
            "/v1/auth/register",
            serde_json::json!({"email": "", "password": "strongpassword123"}),
        )
        .await;
        assert!(
            resp.status() == StatusCode::UNPROCESSABLE_ENTITY
                || resp.status() == StatusCode::BAD_REQUEST,
            "expected 422 or 400, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn register_fails_short_password_http() {
        let app = make_app().await;
        let resp = post_json(
            app,
            "/v1/auth/register",
            serde_json::json!({"email": "test@example.com", "password": "short"}),
        )
        .await;
        assert!(
            resp.status() == StatusCode::UNPROCESSABLE_ENTITY
                || resp.status() == StatusCode::BAD_REQUEST,
            "expected 422 or 400, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn register_succeeds() {
        let app = make_app().await;
        let unique_email = format!("test_{}@example.com", uuid::Uuid::new_v4());
        let resp = post_json(
            app,
            "/v1/auth/register",
            serde_json::json!({
                "email": unique_email,
                "password": "strongpassword123"
            }),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "register should return 201"
        );
        let set_cookie = resp
            .headers()
            .get("set-cookie")
            .expect("should have Set-Cookie header")
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            set_cookie.contains("refresh_token="),
            "Set-Cookie should contain refresh_token"
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            body.get("access_token").is_some(),
            "body should contain access_token"
        );
        delete_test_user(&unique_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn login_wrong_password() {
        let unique_email = format!("test_{}@example.com", uuid::Uuid::new_v4());
        seed_test_user(&unique_email, "correctpassword123").await;
        let app = make_app().await;
        // Try to log in with wrong password
        let resp = post_json(
            app,
            "/v1/auth/login",
            serde_json::json!({
                "email": unique_email,
                "password": "wrongpassword456"
            }),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "wrong password should return 401"
        );
        delete_test_user(&unique_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn login_succeeds() {
        let unique_email = format!("test_{}@example.com", uuid::Uuid::new_v4());
        seed_test_user(&unique_email, "correctpassword123").await;
        let app = make_app().await;
        // Login
        let resp = post_json(
            app,
            "/v1/auth/login",
            serde_json::json!({
                "email": unique_email,
                "password": "correctpassword123"
            }),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "valid login should return 200"
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            body.get("access_token").is_some(),
            "login response should have access_token"
        );
        delete_test_user(&unique_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn logout_clears_cookie() {
        let unique_email = format!("test_{}@example.com", uuid::Uuid::new_v4());
        seed_test_user(&unique_email, "correctpassword123").await;
        let app = make_app().await;

        let login_resp = post_json(
            app.clone(),
            "/v1/auth/login",
            serde_json::json!({
                "email": unique_email,
                "password": "correctpassword123"
            }),
        )
        .await;
        let set_cookie = login_resp
            .headers()
            .get("set-cookie")
            .expect("should have Set-Cookie after login")
            .to_str()
            .unwrap()
            .to_string();

        // Extract the refresh token value from the Set-Cookie header
        let token_value = set_cookie
            .split(';')
            .next()
            .and_then(|s| s.strip_prefix("refresh_token="))
            .expect("should extract refresh_token value")
            .to_string();

        // Logout
        let logout_req = Request::builder()
            .method("POST")
            .uri("/v1/auth/logout")
            .header("cookie", format!("refresh_token={token_value}"))
            .body(Body::empty())
            .unwrap();
        let logout_resp = app.clone().oneshot(logout_req).await.unwrap();
        assert_eq!(
            logout_resp.status(),
            StatusCode::NO_CONTENT,
            "logout should return 204"
        );

        // After logout, refreshing should return 401
        let refresh_req = Request::builder()
            .method("POST")
            .uri("/v1/auth/refresh")
            .header("cookie", format!("refresh_token={token_value}"))
            .body(Body::empty())
            .unwrap();
        let refresh_resp = app.clone().oneshot(refresh_req).await.unwrap();
        assert_eq!(
            refresh_resp.status(),
            StatusCode::UNAUTHORIZED,
            "refresh after logout should return 401"
        );

        let bootstrap_req = Request::builder()
            .method("POST")
            .uri("/v1/auth/bootstrap")
            .header("cookie", format!("refresh_token={token_value}"))
            .body(Body::empty())
            .unwrap();
        let bootstrap_resp = app.oneshot(bootstrap_req).await.unwrap();
        assert_eq!(
            bootstrap_resp.status(),
            StatusCode::NO_CONTENT,
            "bootstrap after logout should quietly return 204"
        );
        delete_test_user(&unique_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn theme_preferences_round_trip_preserves_units_and_scopes_to_account() {
        let first_email = format!("theme_first_{}@example.com", uuid::Uuid::new_v4());
        let second_email = format!("theme_second_{}@example.com", uuid::Uuid::new_v4());
        seed_test_user(&first_email, "correctpassword123").await;
        seed_test_user(&second_email, "correctpassword123").await;
        let app = make_app().await;

        let first_token = login_access_token(app.clone(), &first_email).await;
        let defaults = get_authenticated(app.clone(), "/v1/auth/preferences", &first_token).await;
        assert_eq!(
            defaults["theme"],
            serde_json::json!({ "mode": "dark", "palette": "classic" })
        );

        let units = serde_json::json!({
            "units": {
                "mode": "metric",
                "distance_unit": "kilometers",
                "speed_unit": "kmh",
                "temperature_unit": "celsius",
                "pressure_unit": "kpa",
                "altitude_unit": "meters",
                "place_radius_unit": "meters",
                "efficiency_display": "distance_per_energy"
            }
        });
        put_authenticated(app.clone(), "/v1/auth/preferences", &first_token, units).await;
        let theme_only = put_authenticated(
            app.clone(),
            "/v1/auth/preferences",
            &first_token,
            serde_json::json!({ "theme": { "mode": "system", "palette": "rad" } }),
        )
        .await;
        assert_eq!(theme_only["units"]["mode"], "metric");
        assert_eq!(
            theme_only["theme"],
            serde_json::json!({ "mode": "system", "palette": "rad" })
        );

        let second_token = login_access_token(app.clone(), &second_email).await;
        let second_defaults = get_authenticated(app, "/v1/auth/preferences", &second_token).await;
        assert_eq!(
            second_defaults["theme"],
            serde_json::json!({ "mode": "dark", "palette": "classic" })
        );
        assert_eq!(second_defaults["units"]["mode"], "imperial");

        delete_test_user(&first_email).await;
        delete_test_user(&second_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn oidc_verified_email_auto_link_requires_an_enabled_existing_user() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let email = format!("oidc_disabled_{}@example.com", Uuid::new_v4());
        seed_test_user(&email, "correctpassword123").await;
        sqlx::query("UPDATE panda.users SET is_disabled=TRUE WHERE email=$1")
            .bind(&email)
            .execute(&pool)
            .await
            .expect("disable test user");
        let identity = test_oidc_identity(&email);
        let mut broad_settings = test_oidc_settings(false, true);
        broad_settings.allowed_email_domains.clear();

        let result =
            resolve_oidc_identity(&pool, &broad_settings, &test_oidc_transaction(), &identity)
                .await;
        assert!(matches!(result, Err(AppError::Forbidden)));
        let mapping_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.user_oidc_identities WHERE issuer=$1 AND subject=$2",
        )
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .fetch_one(&pool)
        .await
        .expect("count identity mappings");
        assert_eq!(mapping_count, 0);

        sqlx::query("UPDATE panda.users SET is_disabled=FALSE WHERE email=$1")
            .bind(&email)
            .execute(&pool)
            .await
            .expect("enable test user");
        let linked_user =
            resolve_oidc_identity(&pool, &broad_settings, &test_oidc_transaction(), &identity)
                .await
                .expect("verified email should link an enabled account");
        let expected_user: Uuid = sqlx::query_scalar("SELECT id FROM panda.users WHERE email=$1")
            .bind(&email)
            .fetch_one(&pool)
            .await
            .expect("load test user");
        assert_eq!(linked_user, expected_user);
        delete_test_user(&email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn oidc_auto_signup_rejects_pending_invitation_without_token() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let owner_email = format!("oidc_owner_{}@example.com", Uuid::new_v4());
        let user_email = format!("oidc_signup_{}@example.com", Uuid::new_v4());
        let owner_hash = argon2_hash("correctpassword123").expect("hash owner password");
        let owner_id: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.users(email,password_hash,role) VALUES($1,$2,'super_user') RETURNING id",
        )
        .bind(&owner_email)
        .bind(owner_hash)
        .fetch_one(&pool)
        .await
        .expect("seed super user");
        let invitation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.account_invitations(invited_by,invitee_email,token_hash,expires_at) VALUES($1,$2,$3,now()+interval '1 day') RETURNING id",
        )
        .bind(owner_id)
        .bind(&user_email)
        .bind(sha2_hash("oidc-test-invitation").as_slice())
        .fetch_one(&pool)
        .await
        .expect("seed account invitation");
        let identity = test_oidc_identity(&user_email);

        let result = resolve_oidc_identity(
            &pool,
            &test_oidc_settings(true, false),
            &test_oidc_transaction(),
            &identity,
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden)));
        let user_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM panda.users WHERE lower(email)=lower($1)")
                .bind(&user_email)
                .fetch_one(&pool)
                .await
                .expect("count blocked auto-signup account");
        assert_eq!(user_count, 0);
        let invitation: (Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>) = sqlx::query_as(
            "SELECT accepted_at,created_user_id FROM panda.account_invitations WHERE id=$1",
        )
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .expect("load pending invitation");
        assert!(invitation.0.is_none());
        assert!(invitation.1.is_none());

        delete_test_user(&user_email).await;
        delete_test_user(&owner_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn oidc_invitation_requires_matching_email_and_accepts_once_with_access_and_audit() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let owner_email = format!("oidc_invite_owner_{}@example.com", Uuid::new_v4());
        let invitee_email = format!("oidc_invitee_{}@example.com", Uuid::new_v4());
        let owner_id: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.users(email,password_hash,role) VALUES($1,$2,'super_user') RETURNING id",
        )
        .bind(&owner_email)
        .bind(argon2_hash("correctpassword123").expect("hash owner password"))
        .fetch_one(&pool)
        .await
        .expect("seed owner");
        let vehicle_id: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.vehicles(user_id,rivian_vehicle_id,model,name) VALUES($1,$2,'R1T','Invited R1T') RETURNING id",
        )
        .bind(owner_id)
        .bind(format!("oidc-invite-{}", Uuid::new_v4()))
        .fetch_one(&pool)
        .await
        .expect("seed vehicle");
        let invitation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.account_invitations(invited_by,invitee_email,vehicle_id,token_hash,expires_at,auth_methods) VALUES($1,$2,$3,$4,now()+interval '1 day','sso') RETURNING id",
        )
        .bind(owner_id)
        .bind(&invitee_email)
        .bind(vehicle_id)
        .bind(sha2_hash(&format!("oidc-invite-{}", Uuid::new_v4())))
        .fetch_one(&pool)
        .await
        .expect("seed SSO invitation");
        let mut wrong_identity = test_oidc_identity("other@example.com");
        assert!(matches!(
            accept_oidc_invitation(&pool, invitation_id, &wrong_identity).await,
            Err(AppError::Forbidden)
        ));
        let still_pending: bool = sqlx::query_scalar(
            "SELECT accepted_at IS NULL FROM panda.account_invitations WHERE id=$1",
        )
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .expect("pending invitation");
        assert!(still_pending);

        wrong_identity.email = Some(invitee_email.clone());
        let user_id = accept_oidc_invitation(&pool, invitation_id, &wrong_identity)
            .await
            .expect("accept SSO invitation");
        let account: (Option<String>, String) =
            sqlx::query_as("SELECT password_hash,auth_methods FROM panda.users WHERE id=$1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .expect("created account");
        assert!(account.0.is_none());
        assert_eq!(account.1, "sso");
        let linked_user: Uuid = sqlx::query_scalar(
            "SELECT user_id FROM panda.user_oidc_identities WHERE issuer=$1 AND subject=$2",
        )
        .bind(&wrong_identity.issuer)
        .bind(&wrong_identity.subject)
        .fetch_one(&pool)
        .await
        .expect("linked identity");
        assert_eq!(linked_user, user_id);
        let role: String = sqlx::query_scalar(
            "SELECT role FROM panda.vehicle_memberships WHERE vehicle_id=$1 AND user_id=$2",
        )
        .bind(vehicle_id)
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("viewer membership");
        assert_eq!(role, "viewer");
        let vehicle_settings: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.vehicle_user_settings WHERE vehicle_id=$1 AND user_id=$2",
        )
        .bind(vehicle_id)
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("vehicle settings");
        assert_eq!(vehicle_settings, 1);
        let accepted_user: Uuid = sqlx::query_scalar(
            "SELECT created_user_id FROM panda.account_invitations WHERE id=$1 AND accepted_at IS NOT NULL",
        )
        .bind(invitation_id)
        .fetch_one(&pool)
        .await
        .expect("accepted invitation");
        assert_eq!(accepted_user, user_id);
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.security_events WHERE event_type='account_invitation_accepted' AND user_id=$1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("acceptance audit");
        assert_eq!(audit_count, 1);
        assert!(
            accept_oidc_invitation(&pool, invitation_id, &wrong_identity)
                .await
                .is_err()
        );

        delete_test_user(&invitee_email).await;
        delete_test_user(&owner_email).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn oidc_reauthentication_sets_initial_password_and_revokes_refresh_sessions() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let email = format!("oidc_password_{}@example.com", Uuid::new_v4());
        let user_id: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.users(email,password_hash,role) \
             VALUES($1,NULL,'user') RETURNING id",
        )
        .bind(&email)
        .fetch_one(&pool)
        .await
        .expect("seed passwordless user");
        sqlx::query(
            "INSERT INTO panda.user_oidc_identities(user_id,issuer,subject,email) \
             VALUES($1,'https://issuer.example','linked-subject',$2)",
        )
        .bind(user_id)
        .bind(&email)
        .execute(&pool)
        .await
        .expect("seed linked identity");
        issue_refresh_token(&pool, user_id)
            .await
            .expect("seed refresh session");

        let hash = hash_password("newrecoverypassword123").expect("hash recovery password");
        set_initial_password_after_oidc(&pool, user_id, &hash, &axum::http::HeaderMap::new())
            .await
            .expect("set initial password after OIDC reauthentication");

        let stored_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM panda.users WHERE id=$1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .expect("load stored password hash");
        verify_password("newrecoverypassword123", &stored_hash)
            .expect("new recovery password should verify");
        let active_refresh_sessions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.refresh_tokens \
             WHERE user_id=$1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("count active refresh sessions");
        assert_eq!(active_refresh_sessions, 0);

        let second_attempt =
            set_initial_password_after_oidc(&pool, user_id, &hash, &axum::http::HeaderMap::new())
                .await;
        assert!(matches!(second_attempt, Err(AppError::Conflict(_))));

        let mut password_setup = test_oidc_transaction();
        password_setup.link = true;
        password_setup.user_id = Some(user_id);
        password_setup.pending_password_hash = Some(hash);
        let different_identity = test_oidc_identity(&email);
        let result = resolve_oidc_identity(
            &pool,
            &test_oidc_settings(false, false),
            &password_setup,
            &different_identity,
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden)));
        let unexpected_mapping: i64 =
            sqlx::query_scalar("SELECT count(*) FROM panda.user_oidc_identities WHERE subject=$1")
                .bind(&different_identity.subject)
                .fetch_one(&pool)
                .await
                .expect("count unexpected identity mappings");
        assert_eq!(unexpected_mapping, 0);

        delete_test_user(&email).await;
    }
}

#[cfg(test)]
#[path = "auth_full_replacement_tests.rs"]
mod full_replacement_tests;

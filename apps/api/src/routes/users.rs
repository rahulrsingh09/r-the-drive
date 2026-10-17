use axum::{
    extract::{Path, Query, State},
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::users::{
        can_manage_user, get_user_role, require_admin_or_super_user, require_super_user, UserRole,
    },
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    routes::users_support::{parse_membership_role, parse_role},
    services::security_audit::SecurityAuditEvent,
    services::{authentication_settings, oidc},
};

#[cfg(test)]
#[path = "users_authorization_tests.rs"]
mod authorization_tests;

pub fn router() -> Router<AppState> {
    [
        ("/admin/users", get(list_users)),
        ("/admin/vehicles", get(list_admin_vehicle_options)),
        (
            "/admin/account-invitations",
            get(list_account_invitations).post(create_account_invitation),
        ),
        (
            "/admin/account-invitations/{id}",
            axum::routing::delete(revoke_account_invitation),
        ),
        ("/admin/users/{id}", patch(update_user).delete(delete_user)),
        ("/admin/users/{id}/detail", get(get_user_detail)),
        (
            "/admin/users/{id}/vehicles",
            get(list_user_vehicle_memberships),
        ),
        ("/admin/users/{id}/invites", get(list_user_invites)),
        (
            "/admin/users/{id}/invites/{invite_id}/revoke",
            post(revoke_user_invite),
        ),
        (
            "/admin/users/{id}/vehicles/{vehicle_id}",
            post(grant_user_vehicle_membership)
                .patch(update_user_vehicle_membership)
                .delete(remove_user_vehicle_membership),
        ),
    ]
    .into_iter()
    .fold(Router::new(), |router, (path, methods)| {
        router.route(path, methods)
    })
}

#[derive(Deserialize)]
struct ListUsersQuery {
    search: Option<String>,
}

#[derive(Deserialize)]
struct CreateAccountInvitationBody {
    email: String,
    #[serde(default)]
    vehicle_ids: Option<Vec<Uuid>>,
    /// Legacy clients may continue sending one vehicle.
    vehicle_id: Option<Uuid>,
    expires_in_days: Option<i32>,
    auth_methods: Option<String>,
}

#[derive(Deserialize)]
struct UpdateUserBody {
    email: Option<String>,
    role: Option<String>,
    is_disabled: Option<bool>,
}

#[derive(Deserialize)]
struct RepairMembershipBody {
    role: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct AccountInvitationPayload {
    id: Uuid,
    invitee_email: String,
    vehicle_id: Option<Uuid>,
    vehicle_name: Option<String>,
    vehicle_ids: Vec<Uuid>,
    vehicle_names: Vec<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
    accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    auth_methods: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct UserPayload {
    id: Uuid,
    email: String,
    role: String,
    is_disabled: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ListedUser {
    #[serde(flatten)]
    #[sqlx(flatten)]
    user: UserPayload,
    vehicle_count: i32,
}

async fn lookup_user(pool: &sqlx::PgPool, id: Uuid) -> Result<UserPayload, AppError> {
    sqlx::query_as(
        "SELECT id,email,role,is_disabled,created_at,updated_at FROM panda.users WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn list_users(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let term = query
        .search
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let users = sqlx::query_as::<_, ListedUser>(
        "SELECT u.id,u.email,u.role,u.is_disabled,u.created_at,u.updated_at,
                (SELECT count(*)::int FROM panda.vehicle_memberships m WHERE m.user_id=u.id) AS vehicle_count
         FROM panda.users u WHERE lower(u.email) LIKE $1 ORDER BY u.created_at DESC")
        .bind(format!("%{term}%")).fetch_all(&state.pool).await?;
    Ok(Json(serde_json::json!({"users":users})))
}

async fn list_admin_vehicle_options(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let vehicles: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',id,'display_name',COALESCE(name,model),'model',model)
         FROM panda.vehicles ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({"vehicles":vehicles})))
}

struct InvitationPlan {
    email: String,
    vehicles: Vec<Uuid>,
    days: i32,
    requested_method: Option<String>,
}

impl TryFrom<CreateAccountInvitationBody> for InvitationPlan {
    type Error = AppError;
    fn try_from(body: CreateAccountInvitationBody) -> Result<Self, AppError> {
        let email = body.email.trim().to_lowercase();
        if email.is_empty() || !email.contains('@') {
            return Err(AppError::Validation("valid email required".into()));
        }
        let requested = body
            .vehicle_ids
            .filter(|ids| !ids.is_empty())
            .unwrap_or_else(|| body.vehicle_id.into_iter().collect());
        let vehicles = requested
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(Self {
            email,
            vehicles,
            days: body.expires_in_days.unwrap_or(7),
            requested_method: body.auth_methods,
        })
    }
}

fn invitation_method(
    plan: &InvitationPlan,
    settings: &authentication_settings::EffectiveAuthenticationSettings,
) -> Result<&'static str, AppError> {
    let available = match (
        settings.password_login_enabled,
        settings.oidc_enabled && oidc::provider_configuration_ready(settings),
    ) {
        (true, true) => Some("both"),
        (true, false) => Some("password"),
        (false, true) => Some("sso"),
        _ => None,
    };
    let Some(requested) = plan.requested_method.as_deref().map(str::trim) else {
        return available
            .ok_or_else(|| AppError::Validation("no authentication method is available".into()));
    };
    match (available, requested) {
        (Some("both"), "both") => Ok("both"),
        (Some("both" | "password"), "password") => Ok("password"),
        (Some("both" | "sso"), "sso") => Ok("sso"),
        _ => Err(AppError::Validation(
            "requested authentication method is unavailable".into(),
        )),
    }
}

async fn create_account_invitation(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateAccountInvitationBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let plan = InvitationPlan::try_from(body)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(lower($1),0))")
        .bind(&plan.email)
        .execute(&mut *tx)
        .await?;
    let registered: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM panda.users WHERE lower(email)=$1)")
            .bind(&plan.email)
            .fetch_one(&mut *tx)
            .await?;
    if registered {
        return Err(AppError::Validation("email already registered".into()));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.vehicles WHERE id=ANY($1)")
        .bind(&plan.vehicles)
        .fetch_one(&mut *tx)
        .await?;
    if count != plan.vehicles.len() as i64 {
        return Err(AppError::Validation(
            "one or more vehicles not found".into(),
        ));
    }
    if !(1..=30).contains(&plan.days) {
        return Err(AppError::Validation(
            "expires_in_days must be between 1 and 30".into(),
        ));
    }
    let settings = authentication_settings::load_effective(&mut *tx, &state.age_key).await?;
    let method = invitation_method(&plan, &settings)?;
    let token = random_invitation_token();
    let expires = chrono::Utc::now() + chrono::Duration::days(i64::from(plan.days));
    let primary = plan.vehicles.first().copied();
    let result = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO panda.account_invitations(invited_by,invitee_email,vehicle_id,token_hash,expires_at,auth_methods)
         VALUES($1,$2,$3,$4,$5,$6) RETURNING id")
        .bind(auth.user_id).bind(&plan.email).bind(primary).bind(hash_token(&token)).bind(expires).bind(method)
        .fetch_one(&mut *tx).await;
    let id = match result {
        Ok(id) => id,
        Err(sqlx::Error::Database(error))
            if error.constraint() == Some("account_invitations_active_email_idx") =>
        {
            return Err(AppError::Validation(
                "an active invitation already exists for this email".into(),
            ))
        }
        Err(error) => return Err(error.into()),
    };
    sqlx::query("INSERT INTO panda.account_invitation_vehicles(invitation_id,vehicle_id) SELECT $1,unnest($2::uuid[]) ON CONFLICT DO NOTHING")
        .bind(id).bind(&plan.vehicles).execute(&mut *tx).await?;
    tx.commit().await?;
    support_audit(
        state.pool.clone(),
        "account_invitation_created",
        Some(auth.user_id),
        String::new(),
    );
    Ok(Json(
        serde_json::json!({"id":id,"invitee_email":plan.email,"vehicle_id":primary,"vehicle_ids":plan.vehicles,"expires_at":expires,"auth_methods":method,"activation_token":token}),
    ))
}

async fn list_account_invitations(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let invitations = sqlx::query_as::<_, AccountInvitationPayload>(
        "SELECT ai.id,ai.invitee_email,ai.vehicle_id,COALESCE(v.name,v.model) AS vehicle_name,
           ARRAY(SELECT vehicle_id FROM panda.account_invitation_vehicles WHERE invitation_id=ai.id ORDER BY vehicle_id) AS vehicle_ids,
           ARRAY(SELECT COALESCE(v2.name,v2.model) FROM panda.account_invitation_vehicles aiv JOIN panda.vehicles v2 ON v2.id=aiv.vehicle_id WHERE aiv.invitation_id=ai.id ORDER BY aiv.vehicle_id) AS vehicle_names,
           ai.expires_at,ai.accepted_at,ai.revoked_at,ai.created_at,ai.auth_methods
         FROM panda.account_invitations ai LEFT JOIN panda.vehicles v ON v.id=ai.vehicle_id ORDER BY ai.created_at DESC")
        .fetch_all(&state.pool).await?;
    Ok(Json(serde_json::json!({"invitations":invitations})))
}

async fn revoke_account_invitation(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    finish_support_change(&state, &auth, "account_invitation_revoked", sqlx::query("UPDATE panda.account_invitations SET revoked_at=now(),updated_at=now() WHERE id=$1 AND accepted_at IS NULL").bind(id)).await
}

fn random_invitation_token() -> String {
    use rand::distributions::DistString;
    rand::distributions::Alphanumeric.sample_string(&mut rand::thread_rng(), 48)
}

fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

async fn update_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(target): Path<Uuid>,
    Json(body): Json<UpdateUserBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let target_role = get_user_role(&state.pool, target).await?;
    if !can_manage_user(actor, target_role) {
        return Err(AppError::Forbidden);
    }
    let proposed = body.role.as_deref().map(parse_role).transpose()?;
    if let Some(role) = proposed {
        if !can_manage_user(actor, role) {
            return Err(AppError::Forbidden);
        }
        if target == auth.user_id
            && target_role == UserRole::SuperUser
            && role != UserRole::SuperUser
        {
            return Err(AppError::Validation(
                "cannot demote yourself from super_user".into(),
            ));
        }
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('panda:user-administration',0))")
        .execute(&mut *tx)
        .await?;
    if target_role == UserRole::SuperUser && body.is_disabled == Some(true) {
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.users WHERE role='super_user' AND is_disabled=FALSE",
        )
        .fetch_one(&mut *tx)
        .await?;
        if active <= 1 {
            return Err(AppError::Validation(
                "cannot disable the last super_user".into(),
            ));
        }
    }
    sqlx::query("UPDATE panda.users SET email=COALESCE($2,email),role=COALESCE($3,role),is_disabled=COALESCE($4,is_disabled),updated_at=now() WHERE id=$1")
        .bind(target).bind(body.email.map(|s| s.trim().to_lowercase())).bind(proposed.map(UserRole::as_str)).bind(body.is_disabled)
        .execute(&mut *tx).await?;
    if body.is_disabled == Some(true) {
        crate::services::sessions::revoke_user_sessions(&mut tx, target).await?;
        sqlx::query("UPDATE panda.api_keys SET revoked_at=now(),updated_at=now() WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(target).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    support_audit(
        state.pool,
        "admin_user_update",
        Some(auth.user_id),
        String::new(),
    );
    Ok(Json(serde_json::json!({"ok":true})))
}

async fn delete_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(target): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    if target == auth.user_id {
        return Err(AppError::Validation("cannot delete yourself".into()));
    }
    require_super_user(&state.pool, auth.user_id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('panda:user-administration',0))")
        .execute(&mut *tx)
        .await?;
    let account = sqlx::query_as::<_, UserPayload>("SELECT id,email,role,is_disabled,created_at,updated_at FROM panda.users WHERE id=$1 FOR UPDATE")
        .bind(target).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
    if account.role == "super_user" {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM panda.users WHERE role='super_user'")
                .fetch_one(&mut *tx)
                .await?;
        if count <= 1 {
            return Err(AppError::Validation(
                "cannot delete the last super_user".into(),
            ));
        }
    }
    for statement in [
        "DELETE FROM panda.vehicle_memberships WHERE user_id=$1",
        "DELETE FROM panda.vehicle_user_settings WHERE user_id=$1",
        "DELETE FROM panda.refresh_tokens WHERE user_id=$1",
        "DELETE FROM panda.api_keys WHERE user_id=$1",
    ] {
        sqlx::query(statement)
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE panda.vehicle_invites SET revoked_at=now(),updated_at=now() WHERE (invited_by=$1 OR lower(invitee_email)=lower($2)) AND accepted_at IS NULL AND revoked_at IS NULL")
        .bind(target).bind(&account.email).execute(&mut *tx).await?;
    sqlx::query("UPDATE panda.account_invitations SET revoked_at=now(),updated_at=now() WHERE lower(invitee_email)=lower($1) AND accepted_at IS NULL AND revoked_at IS NULL")
        .bind(&account.email).execute(&mut *tx).await?;
    for statement in [
        "UPDATE panda.authentication_settings SET updated_by=NULL WHERE updated_by=$1",
        "DELETE FROM panda.user_preferences WHERE user_id=$1",
    ] {
        sqlx::query(statement)
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    // The existing theme guard permits only this transaction's account cleanup.
    sqlx::query("SELECT set_config('panda.allow_theme_revision_delete','on',true)")
        .execute(&mut *tx)
        .await?;
    for statement in [
        "DELETE FROM panda.user_theme_publications WHERE theme_id IN (SELECT id FROM panda.user_themes WHERE owner_id=$1)",
        "DELETE FROM panda.user_theme_revisions WHERE theme_id IN (SELECT id FROM panda.user_themes WHERE owner_id=$1)",
        "DELETE FROM panda.user_themes WHERE owner_id=$1",
        "DELETE FROM panda.users WHERE id=$1",
    ] { sqlx::query(statement).bind(target).execute(&mut *tx).await?; }
    tx.commit().await?;
    match state.redis.get_multiplexed_async_connection().await {
        Ok(mut connection) => {
            let _ = redis::cmd("DEL")
                .arg(format!("rivian:connect:{target}"))
                .query_async::<()>(&mut connection)
                .await;
        }
        Err(_) => tracing::debug!("account staging cleanup deferred to expiry"),
    }
    support_audit(
        state.pool,
        "admin_user_delete",
        Some(auth.user_id),
        String::new(),
    );
    Ok(Json(serde_json::json!({"ok":true})))
}

async fn get_user_detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(target): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let user = lookup_user(&state.pool, target).await?;
    let memberships = list_user_memberships_payload(&state.pool, target).await?;
    let invites = list_user_invites_payload(&state.pool, &user.email).await?;
    Ok(Json(
        serde_json::json!({"user":user,"memberships":memberships,"invites":invites}),
    ))
}

async fn list_user_vehicle_memberships(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(target): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    list_user_memberships_payload(&state.pool, target)
        .await
        .map(|rows| Json(serde_json::json!({"memberships":rows})))
}

async fn list_user_invites(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(target): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let user = lookup_user(&state.pool, target).await?;
    list_user_invites_payload(&state.pool, &user.email)
        .await
        .map(|rows| Json(serde_json::json!({"invites":rows})))
}

async fn revoke_user_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((target, invite)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let user = lookup_user(&state.pool, target).await?;
    finish_support_change(&state,&auth,"admin_invite_revoke",sqlx::query("UPDATE panda.vehicle_invites SET revoked_at=now(),updated_at=now() WHERE id=$1 AND lower(invitee_email)=lower($2) AND accepted_at IS NULL").bind(invite).bind(user.email)).await
}

async fn finish_support_change<'q>(
    state: &AppState,
    auth: &AuthUser,
    event: &'static str,
    change: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
) -> Result<Json<serde_json::Value>, AppError> {
    change.execute(&state.pool).await?;
    support_audit(state.pool.clone(), event, Some(auth.user_id), String::new());
    Ok(Json(serde_json::json!({"ok":true})))
}

enum MembershipChange {
    Grant(String),
    Update(String),
    Remove,
}

async fn change_membership(
    state: AppState,
    auth: AuthUser,
    target: Uuid,
    vehicle: Uuid,
    change: MembershipChange,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM panda.vehicles WHERE id=$1 FOR UPDATE")
        .bind(vehicle)
        .execute(&mut *tx)
        .await?;
    let event = match change {
        MembershipChange::Grant(role) => {
            sqlx::query("INSERT INTO panda.vehicle_memberships(vehicle_id,user_id,role,is_default) VALUES($1,$2,$3,FALSE) ON CONFLICT(vehicle_id,user_id) DO UPDATE SET role=EXCLUDED.role,updated_at=now()")
                .bind(vehicle).bind(target).bind(role).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO panda.vehicle_user_settings(vehicle_id,user_id) VALUES($1,$2) ON CONFLICT(vehicle_id,user_id) DO NOTHING")
                .bind(vehicle).bind(target).execute(&mut *tx).await?;
            "admin_membership_grant"
        }
        MembershipChange::Update(role) => {
            sqlx::query("UPDATE panda.vehicle_memberships SET role=$3,updated_at=now() WHERE user_id=$1 AND vehicle_id=$2")
                .bind(target).bind(vehicle).bind(role).execute(&mut *tx).await?;
            "admin_membership_update"
        }
        MembershipChange::Remove => {
            let role: String = sqlx::query_scalar(
                "SELECT role FROM panda.vehicle_memberships WHERE user_id=$1 AND vehicle_id=$2",
            )
            .bind(target)
            .bind(vehicle)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;
            if role == "owner" {
                let count:i64=sqlx::query_scalar("SELECT count(*) FROM panda.vehicle_memberships WHERE vehicle_id=$1 AND role='owner'")
                    .bind(vehicle).fetch_one(&mut *tx).await?;
                if count <= 1 {
                    return Err(AppError::Validation(
                        "vehicle must keep at least one owner".into(),
                    ));
                }
            }
            sqlx::query("DELETE FROM panda.vehicle_memberships WHERE user_id=$1 AND vehicle_id=$2")
                .bind(target)
                .bind(vehicle)
                .execute(&mut *tx)
                .await?;
            "admin_membership_remove"
        }
    };
    tx.commit().await?;
    support_audit(state.pool, event, Some(auth.user_id), String::new());
    Ok(Json(serde_json::json!({"ok":true})))
}

async fn grant_user_vehicle_membership(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((target, vehicle)): Path<(Uuid, Uuid)>,
    Json(body): Json<RepairMembershipBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let change = MembershipChange::Grant(parse_membership_role(&body.role)?.to_owned());
    change_membership(state, auth, target, vehicle, change).await
}
async fn update_user_vehicle_membership(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((target, vehicle)): Path<(Uuid, Uuid)>,
    Json(body): Json<RepairMembershipBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let change = MembershipChange::Update(parse_membership_role(&body.role)?.to_owned());
    change_membership(state, auth, target, vehicle, change).await
}
async fn remove_user_vehicle_membership(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((target, vehicle)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    change_membership(state, auth, target, vehicle, MembershipChange::Remove).await
}

#[derive(Serialize, sqlx::FromRow)]
struct MembershipPayload {
    vehicle_id: Uuid,
    role: String,
    is_default: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    model: String,
    display_name: Option<String>,
}
#[derive(Serialize, sqlx::FromRow)]
struct InvitePayload {
    id: Uuid,
    vehicle_id: Uuid,
    vehicle_name: String,
    invitee_email: String,
    role: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    accepted_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}
async fn list_user_memberships_payload(
    pool: &sqlx::PgPool,
    target: Uuid,
) -> Result<Vec<serde_json::Value>, AppError> {
    let rows=sqlx::query_as::<_,MembershipPayload>("SELECT m.vehicle_id,m.role,m.is_default,m.created_at,v.model,COALESCE(s.display_name,v.name) AS display_name FROM panda.vehicle_memberships m JOIN panda.vehicles v ON v.id=m.vehicle_id LEFT JOIN panda.vehicle_user_settings s ON s.vehicle_id=m.vehicle_id AND s.user_id=m.user_id WHERE m.user_id=$1 ORDER BY m.created_at DESC")
        .bind(target).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| serde_json::to_value(row).map_err(|error| AppError::Internal(error.into())))
        .collect()
}
async fn list_user_invites_payload(
    pool: &sqlx::PgPool,
    email: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let rows=sqlx::query_as::<_,InvitePayload>("SELECT i.id,i.vehicle_id,COALESCE(v.name,v.model) AS vehicle_name,i.invitee_email,i.role,i.expires_at,i.accepted_at,i.revoked_at,i.created_at FROM panda.vehicle_invites i JOIN panda.vehicles v ON v.id=i.vehicle_id WHERE lower(i.invitee_email)=lower($1) ORDER BY i.created_at DESC")
        .bind(email).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| serde_json::to_value(row).map_err(|error| AppError::Internal(error.into())))
        .collect()
}

fn support_audit(pool: sqlx::PgPool, event: &'static str, user_id: Option<Uuid>, _detail: String) {
    let audit = SecurityAuditEvent::success(event, user_id);
    tokio::spawn(async move {
        match audit.record(&pool).await {
            Ok(()) => (),
            Err(error) => tracing::warn!(%error,event,"support audit insert failed"),
        }
    });
}

#[cfg(test)]
#[path = "full_replacement_identity_policy_tests.rs"]
mod full_replacement_identity_policy_tests;

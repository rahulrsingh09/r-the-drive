use crate::{
    errors::AppError,
    middleware::auth::{require_vehicle_access, AuthUser},
};
use sqlx::PgPool;
use uuid::Uuid;

/// Require that a session user belongs to a vehicle.
///
/// This checks `vehicle_memberships`, not `vehicles.user_id`; role checks must
/// be applied separately for mutations.
pub async fn require_vehicle_membership(
    pool: &PgPool,
    user_id: Uuid,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    check_membership(pool, user_id, vehicle_id, None).await
}

/// Authorize a vehicle read for both a session member and a scoped API key.
pub async fn require_vehicle_read_access(
    pool: &PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    authorize(pool, auth, vehicle_id, VehicleOperation::Read).await
}

/// Authorize an operational vehicle change. Viewers are deliberately read-only.
pub async fn require_vehicle_manager_access(
    pool: &PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    authorize(pool, auth, vehicle_id, VehicleOperation::Manage).await
}

/// Authorize a session-authenticated member without granting any API-key
/// capability. Use this for member preferences that are mutations but do not
/// require a manager/owner role.
pub async fn require_vehicle_session_access(
    pool: &PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    authorize(pool, auth, vehicle_id, VehicleOperation::MemberPreferences).await
}

/// Authorize an owner-only vehicle operation. API keys never authorize
/// mutations, including owner-level administration.
pub async fn require_vehicle_owner_access(
    pool: &PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    authorize(pool, auth, vehicle_id, VehicleOperation::Administer).await
}

pub async fn require_vehicle_role(
    pool: &PgPool,
    user_id: Uuid,
    vehicle_id: Uuid,
    allowed_roles: &[&str],
) -> Result<(), AppError> {
    check_membership(pool, user_id, vehicle_id, Some(allowed_roles)).await
}

enum VehicleOperation {
    Read,
    MemberPreferences,
    Manage,
    Administer,
}

async fn authorize(
    pool: &PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
    operation: VehicleOperation,
) -> Result<(), AppError> {
    let roles: Option<&[&str]> = match operation {
        VehicleOperation::Read => {
            require_vehicle_access(auth, vehicle_id)?;
            None
        }
        _ if auth.api_access_level.is_some() => return Err(AppError::Forbidden),
        VehicleOperation::MemberPreferences => None,
        VehicleOperation::Manage => Some(&["owner", "manager"]),
        VehicleOperation::Administer => Some(&["owner"]),
    };
    check_membership(pool, auth.user_id, vehicle_id, roles).await
}

async fn check_membership(
    pool: &PgPool,
    user_id: Uuid,
    vehicle_id: Uuid,
    roles: Option<&[&str]>,
) -> Result<(), AppError> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM panda.vehicle_memberships
            WHERE vehicle_id = $1 AND user_id = $2
              AND ($3::text[] IS NULL OR role = ANY($3))
        )",
    )
    .bind(vehicle_id)
    .bind(user_id)
    .bind(roles)
    .fetch_one(pool)
    .await?;
    allowed.then_some(()).ok_or(AppError::Forbidden)
}

pub async fn get_default_vehicle_id(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT COALESCE(
            (SELECT vehicle_id FROM panda.vehicle_memberships WHERE user_id = $1 AND is_default = TRUE LIMIT 1),
            (SELECT default_vehicle_id FROM panda.users WHERE id = $1)
        )")
        .bind(user_id).fetch_one(pool).await.map_err(AppError::from)
}

pub async fn get_vehicle_battery_capacity(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<Option<f64>, AppError> {
    let cap = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT battery_capacity_wh FROM panda.vehicles WHERE id = $1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(cap)
}

pub async fn get_vehicle_owner_id(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    let owner_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id
         FROM panda.vehicle_memberships
         WHERE vehicle_id = $1 AND role = 'owner'
         ORDER BY created_at
         LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;

    Ok(owner_id)
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;

    #[tokio::test]
    async fn api_key_rejections_precede_database_access() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://synthetic:unused@127.0.0.1/synthetic")
            .unwrap();
        pool.close().await;
        let vehicle = Uuid::from_u128(1);
        let key = AuthUser {
            user_id: Uuid::from_u128(2),
            default_vehicle_id: None,
            api_access_level: Some("read".into()),
            api_vehicle_id: Some(vehicle),
        };
        assert!(matches!(
            require_vehicle_read_access(&pool, &key, Uuid::nil()).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            require_vehicle_manager_access(&pool, &key, vehicle).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            require_vehicle_session_access(&pool, &key, vehicle).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            require_vehicle_owner_access(&pool, &key, vehicle).await,
            Err(AppError::Forbidden)
        ));
        assert!(!matches!(
            require_vehicle_read_access(&pool, &key, vehicle).await,
            Err(AppError::Forbidden)
        ));
    }
}

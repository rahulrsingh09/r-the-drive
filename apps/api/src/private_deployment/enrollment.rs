use crate::errors::AppError;
use uuid::Uuid;

pub(crate) async fn lock_enrollment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    rivian_vehicle_id: &str,
) -> Result<Option<Uuid>, AppError> {
    lock_active_enrollment_user(tx, user_id).await?;
    // Rivian IDs are unique per user, so first enrollment needs a global lock.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1))")
        .bind(rivian_vehicle_id)
        .execute(&mut **tx)
        .await?;
    let existing =
        sqlx::query_scalar("SELECT id FROM panda.vehicles WHERE rivian_vehicle_id = $1 FOR UPDATE")
            .bind(rivian_vehicle_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(vehicle_id) = existing {
        lock_credential_membership(tx, user_id, vehicle_id).await?;
    }
    Ok(existing)
}

pub(crate) async fn lock_active_enrollment_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> Result<(), AppError> {
    match sqlx::query_scalar::<_, bool>(
        "SELECT is_disabled FROM panda.users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        Some(false) => Ok(()),
        Some(true) => Err(AppError::Forbidden),
        None => Err(AppError::Unauthorized),
    }
}

pub(crate) async fn lock_credential_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM panda.vehicle_memberships
         WHERE vehicle_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(vehicle_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    match role.as_deref() {
        Some("owner" | "manager") => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}

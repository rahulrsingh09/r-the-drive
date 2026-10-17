use crate::errors::AppError;
use uuid::Uuid;

#[cfg(test)]
#[path = "full_replacement_key_custody_enrollment_tests.rs"]
mod full_replacement_key_custody_tests;

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
    let existing: Option<(Uuid, bool)> = sqlx::query_as(
        "WITH candidate AS MATERIALIZED (
            SELECT id FROM panda.vehicles WHERE rivian_vehicle_id = $1 FOR UPDATE
         )
         SELECT id, COALESCE((
            SELECT role IN ('owner', 'manager') FROM panda.vehicle_memberships
            WHERE vehicle_id = candidate.id AND user_id = $2 FOR SHARE
         ), false) FROM candidate",
    )
    .bind(rivian_vehicle_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    match existing {
        Some((vehicle, true)) => Ok(Some(vehicle)),
        Some((_, false)) => Err(AppError::Forbidden),
        None => Ok(None),
    }
}

pub(crate) async fn lock_active_enrollment_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> Result<(), AppError> {
    let disabled = sqlx::query_scalar::<_, bool>(
        "SELECT is_disabled FROM panda.users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AppError::Unauthorized)?;
    if disabled {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub(crate) async fn lock_credential_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    let permitted = sqlx::query_scalar::<_, bool>(
        "SELECT role IN ('owner', 'manager') FROM panda.vehicle_memberships
         WHERE vehicle_id = $1 AND user_id = $2 FOR SHARE",
    )
    .bind(vehicle_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    if permitted.unwrap_or(false) {
        return Ok(());
    }
    Err(AppError::Forbidden)
}

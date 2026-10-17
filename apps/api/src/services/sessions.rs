//! Session authorization shared by HTTP and live connections.
use sqlx::PgPool;
use uuid::Uuid;

use crate::{errors::AppError, middleware::auth::Claims};

pub async fn require_enabled_session(pool: &PgPool, claims: &Claims) -> Result<(), AppError> {
    if chrono::Utc::now().timestamp() >= claims.exp {
        return Err(AppError::Unauthorized);
    }
    let account = sqlx::query_scalar::<_, bool>("SELECT is_disabled FROM panda.users WHERE id=$1")
        .bind(claims.sub)
        .fetch_optional(pool)
        .await?;
    match account {
        None => return Err(AppError::Unauthorized),
        Some(true) => return Err(AppError::Forbidden),
        Some(false) => {}
    }
    let Some(family) = claims.sid else {
        return Ok(());
    };
    let active = sqlx::query_scalar::<_, Uuid>(
        "SELECT f.id FROM panda.session_families f
         JOIN panda.refresh_tokens t ON t.family_id=f.id
         WHERE f.id=$1 AND f.user_id=$2 AND f.revoked_at IS NULL
           AND t.revoked_at IS NULL AND t.expires_at > now()
         LIMIT 1",
    )
    .bind(family)
    .bind(claims.sub)
    .fetch_optional(pool)
    .await?;
    active.map(|_| ()).ok_or(AppError::Unauthorized)
}

pub async fn revoke_family(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    family_id: Uuid,
) -> Result<(), AppError> {
    revoke_families(tx, &[family_id]).await
}

pub async fn revoke_user_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> Result<(), AppError> {
    let locked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM panda.session_families WHERE user_id=$1 ORDER BY id FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    revoke_families(tx, &locked).await
}

async fn revoke_families(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    families: &[Uuid],
) -> Result<(), AppError> {
    if families.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE panda.session_families SET revoked_at=COALESCE(revoked_at, now()) WHERE id=ANY($1)",
    )
    .bind(families)
    .execute(&mut **tx)
    .await?;
    // A fresh statement snapshot must include descendants committed while the family lock waited.
    sqlx::query("UPDATE panda.refresh_tokens SET revoked_at=COALESCE(revoked_at, now()) WHERE family_id=ANY($1)")
        .bind(families).execute(&mut **tx).await?;
    Ok(())
}

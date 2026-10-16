use anyhow::Result;
use sqlx::PgConnection;
use uuid::Uuid;

/// Serialize charge checkpoint and maintenance transactions for one vehicle.
pub async fn lock_charge_history(conn: &mut PgConnection, vehicle_id: Uuid) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock($1,hashtext($2))")
        .bind(0x5043_4847_i32)
        .bind(vehicle_id.to_string())
        .execute(conn)
        .await?;
    Ok(())
}

pub(crate) async fn charge_sessions_are_unpinned(
    conn: &mut PgConnection,
    vehicle_id: Uuid,
    ids: &[Uuid],
) -> Result<bool> {
    let rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM panda.charge_sessions WHERE vehicle_id=$1 AND id=ANY($2) ORDER BY id FOR UPDATE",
    ).bind(vehicle_id).bind(ids).fetch_all(&mut *conn).await?;
    if rows.len() != ids.len() {
        return Ok(false);
    }
    let pinned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.active_charge_checkpoints WHERE active_session_id=ANY($1)) \
         OR EXISTS(SELECT 1 FROM panda.pending_charge_completions WHERE session_id=ANY($1))",
    ).bind(ids).fetch_one(conn).await?;
    Ok(!pinned)
}

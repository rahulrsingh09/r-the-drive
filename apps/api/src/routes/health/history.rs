use super::{AppError, SoftwareEntry, Uuid};

pub(super) async fn fetch_sw_history(
    pool: &sqlx::PgPool,
    vid: Uuid,
) -> Result<Vec<SoftwareEntry>, AppError> {
    let rows = sqlx::query_as::<_, SoftwareEntry>(
        r#"SELECT version, installed_at, observed_until
           FROM panda.software_versions
           WHERE vehicle_id = $1
           ORDER BY installed_at DESC
           LIMIT 20"#,
    )
    .bind(vid)
    .fetch_all(pool)
    .await
    .map_err(AppError::from)?;
    Ok(rows)
}

pub(super) async fn fetch_thermal_count(pool: &sqlx::PgPool, vid: Uuid) -> Result<i64, AppError> {
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM timeseries.telemetry
           WHERE vehicle_id = $1
             AND hv_thermal_event IS NOT NULL
             AND hv_thermal_event != 'none'
             AND ts >= now() - interval '30 days'"#,
    )
    .bind(vid)
    .fetch_one(pool)
    .await
    .map_err(AppError::from)?;
    Ok(count)
}

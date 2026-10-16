use super::*;

pub(super) async fn open_software_version(
    pool: &PgPool,
    vehicle_id: Uuid,
    version: &str,
    installed_at: chrono::DateTime<Utc>,
) {
    let current_open = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT version
           FROM panda.software_versions
           WHERE vehicle_id = $1 AND observed_until IS NULL
           ORDER BY installed_at DESC
           LIMIT 1"#,
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if current_open.as_ref().and_then(|v| v.as_deref()) == Some(version) {
        return;
    }

    if current_open.is_some() {
        let _ = sqlx::query(
            r#"UPDATE panda.software_versions
               SET observed_until = $1
               WHERE vehicle_id = $2 AND observed_until IS NULL"#,
        )
        .bind(installed_at)
        .bind(vehicle_id)
        .execute(pool)
        .await;
    }

    let _ = sqlx::query(
        r#"INSERT INTO panda.software_versions (vehicle_id, version, installed_at)
           VALUES ($1, $2, $3)"#,
    )
    .bind(vehicle_id)
    .bind(version)
    .bind(installed_at)
    .execute(pool)
    .await;
}

pub(super) async fn close_software_version(
    pool: &PgPool,
    vehicle_id: Uuid,
    version: &str,
    observed_until: chrono::DateTime<Utc>,
) {
    let _ = sqlx::query(
        r#"UPDATE panda.software_versions
           SET observed_until = $1
           WHERE vehicle_id = $2 AND version = $3 AND observed_until IS NULL"#,
    )
    .bind(observed_until)
    .bind(vehicle_id)
    .bind(version)
    .execute(pool)
    .await;
}

#[derive(Default)]
pub(super) struct SoftwareHistory {
    current: Option<(String, DateTime<Utc>)>,
}
impl SoftwareHistory {
    pub(super) async fn observe(
        &mut self,
        pool: &PgPool,
        vehicle_id: Uuid,
        version: &str,
        at: DateTime<Utc>,
    ) {
        if self
            .current
            .as_ref()
            .is_some_and(|(current, _)| current == version)
        {
            return;
        }
        if let Some((previous, _)) = self.current.take() {
            close_software_version(pool, vehicle_id, &previous, at).await;
        }
        open_software_version(pool, vehicle_id, version, at).await;
        self.current = Some((version.into(), at));
    }
}

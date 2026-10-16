use super::*;

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
        let previous = self.current.as_ref().map(|(version, _)| version.as_str());
        match record_observation(pool, vehicle_id, previous, version, at).await {
            Ok(()) => self.current = Some((version.into(), at)),
            Err(error) => {
                tracing::warn!(%vehicle_id,%error,"software history observation failed; retaining retry state")
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Rotation {
    Retain,
    Replace,
}
impl Rotation {
    fn plan(open: &[Option<String>], previous: Option<&str>, version: &str) -> Self {
        let surviving = open
            .iter()
            .find(|item| previous.is_none() || item.as_deref() != previous);
        if surviving.and_then(|item| item.as_deref()) == Some(version) {
            Self::Retain
        } else {
            Self::Replace
        }
    }
}

async fn record_observation(
    pool: &PgPool,
    vehicle_id: Uuid,
    previous: Option<&str>,
    version: &str,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let value = vehicle_id.as_u128();
    sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
        .bind(0x5253_5748_i32)
        .bind((value ^ (value >> 64)) as i32)
        .execute(&mut *tx)
        .await?;
    let open:Vec<Option<String>>=sqlx::query_scalar("SELECT version FROM panda.software_versions WHERE vehicle_id=$1 AND observed_until IS NULL ORDER BY installed_at DESC FOR UPDATE")
        .bind(vehicle_id).fetch_all(&mut *tx).await?;
    match Rotation::plan(&open, previous, version) {
        Rotation::Retain => {
            if let Some(previous) = previous {
                sqlx::query("UPDATE panda.software_versions SET observed_until=$1 WHERE vehicle_id=$2 AND version=$3 AND observed_until IS NULL")
                    .bind(at).bind(vehicle_id).bind(previous).execute(&mut *tx).await?;
            }
        }
        Rotation::Replace => {
            sqlx::query("UPDATE panda.software_versions SET observed_until=$1 WHERE vehicle_id=$2 AND observed_until IS NULL")
                .bind(at).bind(vehicle_id).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO panda.software_versions(vehicle_id,version,installed_at) VALUES($1,$2,$3)")
                .bind(vehicle_id).bind(version).bind(at).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_after_cached_previous_is_closed_with_nullable_and_duplicate_rows() {
        assert_eq!(Rotation::plan(&[], None, "v1"), Rotation::Replace);
        assert_eq!(
            Rotation::plan(&[Some("v1".into())], None, "v1"),
            Rotation::Retain
        );
        assert_eq!(
            Rotation::plan(&[Some("v1".into()), Some("v2".into())], Some("v1"), "v2"),
            Rotation::Retain
        );
        assert_eq!(
            Rotation::plan(&[Some("v1".into()), None], Some("v1"), "v2"),
            Rotation::Replace
        );
    }
}

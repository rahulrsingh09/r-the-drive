use super::*;
use chrono::{DateTime, Duration, Utc};

#[derive(sqlx::FromRow)]
struct AdmissionRow {
    rivian_vehicle_id: String,
    encrypted_tokens: Option<Vec<u8>>,
    history_backfill_status: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    database_now: DateTime<Utc>,
}

enum Admission {
    Claimed(ClaimedChargeBackfill),
    Running,
    MissingCredentials,
}

impl Admission {
    fn decide(vehicle_id: Uuid, row: Option<AdmissionRow>, now: DateTime<Utc>) -> Self {
        let Some(row) = row else {
            return Self::MissingCredentials;
        };
        let running = row.history_backfill_status.as_deref() == Some("running");
        let stale_after = Duration::minutes(BACKFILL_STALE_AFTER_MINUTES);
        let eligible = !running
            || row
                .updated_at
                .is_some_and(|at| at < row.database_now - stale_after);
        if eligible {
            if let Some(encrypted_tokens) = row.encrypted_tokens {
                return Self::Claimed(ClaimedChargeBackfill {
                    vehicle_id,
                    rivian_vehicle_id: row.rivian_vehicle_id,
                    encrypted_tokens,
                });
            }
        }
        if running && row.updated_at.is_some_and(|at| at > now - stale_after) {
            Self::Running
        } else {
            Self::MissingCredentials
        }
    }
}

pub(super) async fn claim(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<ClaimedChargeBackfill, ChargeBackfillError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, AdmissionRow>(
        "SELECT v.rivian_vehicle_id, c.encrypted_tokens, v.history_backfill_status,
                v.updated_at, now() AS database_now
         FROM panda.vehicles v LEFT JOIN panda.vehicle_credentials c ON c.vehicle_id=v.id
         WHERE v.id=$1 FOR UPDATE OF v",
    )
    .bind(vehicle_id)
    .fetch_optional(&mut *tx)
    .await?;
    match Admission::decide(vehicle_id, row, Utc::now()) {
        Admission::Claimed(claimed) => {
            sqlx::query(
                "UPDATE panda.vehicles SET history_backfill_status='running',
                     history_backfilled_at=NULL, history_session_count=NULL, updated_at=now()
                 WHERE id=$1",
            )
            .bind(vehicle_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(claimed)
        }
        Admission::Running => Err(ChargeBackfillError::AlreadyRunning),
        Admission::MissingCredentials => Err(ChargeBackfillError::CredentialsNotFound),
    }
}

pub(super) struct BackfillTask {
    pool: PgPool,
    age_key: String,
    claimed: ClaimedChargeBackfill,
}

impl BackfillTask {
    pub async fn admit(
        pool: PgPool,
        age_key: String,
        vehicle_id: Uuid,
    ) -> Result<Self, ChargeBackfillError> {
        let claimed = claim(&pool, vehicle_id).await?;
        Ok(Self {
            pool,
            age_key,
            claimed,
        })
    }

    pub fn launch(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let vehicle_id = self.claimed.vehicle_id;
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default();
            let result = run_claimed(&self.pool, &client, &self.age_key, self.claimed).await;
            match result {
                Ok(count) => {
                    tracing::info!(vehicle_id=%vehicle_id, count, "charge history backfill complete")
                }
                Err(error) => {
                    tracing::warn!(vehicle_id=%vehicle_id, error=%error, "charge history backfill task failed")
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "../__tests__/charge_backfill_decisions.rs"]
mod tests;

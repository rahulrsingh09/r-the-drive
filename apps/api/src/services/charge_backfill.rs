use sqlx::PgPool;
use uuid::Uuid;

use crate::ingestion::{rivian_poll, session_store};
use crate::services::charge_sessions::canonicalize_charge_sessions;

const BACKFILL_STALE_AFTER_MINUTES: i64 = 120;

#[derive(Debug, Clone)]
pub struct ClaimedChargeBackfill {
    pub vehicle_id: Uuid,
    pub rivian_vehicle_id: String,
    encrypted_tokens: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChargeBackfillStatus {
    pub vehicle_id: Uuid,
    pub history_backfilled_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: Option<String>,
    pub rivian_session_count: Option<i32>,
    pub local_session_count: i64,
    pub missing_source_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChargeBackfillStart {
    pub ok: bool,
    pub status: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum ChargeBackfillError {
    #[error("backfill already running")]
    AlreadyRunning,
    #[error("vehicle credentials not found")]
    CredentialsNotFound,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub async fn get_status(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<ChargeBackfillStatus, ChargeBackfillError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
        Option<i32>,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT v.history_backfilled_at, v.history_backfill_status, v.history_session_count,
                    COUNT(cs.id),
                    COUNT(*) FILTER (WHERE cs.source IS NULL)
             FROM panda.vehicles v
             LEFT JOIN panda.charge_sessions cs ON cs.vehicle_id = v.id
             WHERE v.id = $1
             GROUP BY v.history_backfilled_at, v.history_backfill_status, v.history_session_count",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;

    let (
        history_backfilled_at,
        status,
        rivian_session_count,
        local_session_count,
        missing_source_count,
    ) = row.unwrap_or((None, None, None, 0, 0));

    Ok(ChargeBackfillStatus {
        vehicle_id,
        history_backfilled_at,
        status,
        rivian_session_count,
        local_session_count,
        missing_source_count,
    })
}

pub async fn claim(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<ClaimedChargeBackfill, ChargeBackfillError> {
    let claimed: Option<(String, Vec<u8>)> = sqlx::query_as(
        "UPDATE panda.vehicles v
         SET history_backfill_status = 'running',
             history_backfilled_at = NULL,
             history_session_count = NULL,
             updated_at = now()
         FROM panda.vehicle_credentials c
         WHERE v.id = $1
           AND c.vehicle_id = v.id
                     AND (
                                 COALESCE(v.history_backfill_status, '') <> 'running'
                                 OR v.updated_at < now() - ($2::int * interval '1 minute')
                             )
         RETURNING v.rivian_vehicle_id, c.encrypted_tokens",
    )
    .bind(vehicle_id)
    .bind(BACKFILL_STALE_AFTER_MINUTES as i32)
    .fetch_optional(pool)
    .await?;

    if let Some((rivian_vehicle_id, encrypted_tokens)) = claimed {
        return Ok(ClaimedChargeBackfill {
            vehicle_id,
            rivian_vehicle_id,
            encrypted_tokens,
        });
    }

    let current_status: Option<(Option<String>, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            "SELECT history_backfill_status, updated_at FROM panda.vehicles WHERE id = $1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?;

    let is_fresh_running = current_status
        .and_then(|(status, updated_at)| {
            status.map(|s| {
                s == "running"
                    && updated_at
                        .map(|ts| {
                            ts > chrono::Utc::now()
                                - chrono::Duration::minutes(BACKFILL_STALE_AFTER_MINUTES)
                        })
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    if is_fresh_running {
        Err(ChargeBackfillError::AlreadyRunning)
    } else {
        Err(ChargeBackfillError::CredentialsNotFound)
    }
}

pub async fn run_claimed(
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
    claimed: ClaimedChargeBackfill,
) -> Result<usize, ChargeBackfillError> {
    let result = run_claimed_inner(pool, client, age_key, &claimed).await;
    match &result {
        Ok(count) => {
            sqlx::query(
                "UPDATE panda.vehicles
                 SET history_backfill_status = 'done',
                     history_backfilled_at = now(),
                     history_session_count = $2,
                     updated_at = now()
                 WHERE id = $1",
            )
            .bind(claimed.vehicle_id)
            .bind(*count as i32)
            .execute(pool)
            .await?;
        }
        Err(error) => {
            tracing::warn!(
                vehicle_id = %claimed.vehicle_id,
                error = %error,
                "charge history backfill failed"
            );
            let _ = sqlx::query(
                "UPDATE panda.vehicles
                 SET history_backfill_status = 'error',
                     updated_at = now()
                 WHERE id = $1",
            )
            .bind(claimed.vehicle_id)
            .execute(pool)
            .await;
        }
    }

    result
}

pub async fn run(
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
    vehicle_id: Uuid,
) -> Result<usize, ChargeBackfillError> {
    let claimed = claim(pool, vehicle_id).await?;
    run_claimed(pool, client, age_key, claimed).await
}

pub async fn spawn(
    pool: PgPool,
    age_key: String,
    vehicle_id: Uuid,
) -> Result<ChargeBackfillStart, ChargeBackfillError> {
    let claimed = claim(&pool, vehicle_id).await?;
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        match run_claimed(&pool, &client, &age_key, claimed).await {
            Ok(count) => {
                tracing::info!(vehicle_id=%vehicle_id, count, "charge history backfill complete");
            }
            Err(error) => {
                tracing::warn!(vehicle_id=%vehicle_id, error=%error, "charge history backfill task failed");
            }
        }
    });

    Ok(ChargeBackfillStart {
        ok: true,
        status: "running",
    })
}

async fn run_claimed_inner(
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
    claimed: &ClaimedChargeBackfill,
) -> Result<usize, ChargeBackfillError> {
    let identity = age_key
        .parse::<age::x25519::Identity>()
        .map_err(|e| anyhow::anyhow!("age key parse failed: {e}"))?;
    let tokens = session_store::decrypt_tokens(&claimed.encrypted_tokens, &identity)
        .map_err(|e| anyhow::anyhow!("token decrypt failed: {e}"))?;

    rivian_poll::fetch_charge_history_full(
        &claimed.rivian_vehicle_id,
        claimed.vehicle_id,
        pool,
        client,
        &tokens,
    )
    .await
    .map_err(ChargeBackfillError::Other)?;

    canonicalize_charge_sessions(pool, Some(claimed.vehicle_id))
        .await
        .map_err(ChargeBackfillError::Other)?;

    let canonical_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::int8 FROM panda.charge_sessions WHERE vehicle_id = $1",
    )
    .bind(claimed.vehicle_id)
    .fetch_one(pool)
    .await?;

    Ok(canonical_count as usize)
}

#[cfg(test)]
mod charge_history_backfill_contract {
    use super::*;
    use crate::services::charge_sessions::charge_history_contract::{
        cleanup, database, id, vehicle,
    };
    use serde_json::json;

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL; never contacts a provider"]
    async fn charge_history_claim_status_retry_and_local_errors() {
        let (pool, admin, name) = database().await;
        let v = vehicle(&pool, 20).await;
        assert!(matches!(
            claim(&pool, id(999)).await,
            Err(ChargeBackfillError::CredentialsNotFound)
        ));
        assert!(matches!(
            claim(&pool, v).await,
            Err(ChargeBackfillError::CredentialsNotFound)
        ));
        assert_eq!(
            serde_json::to_value(get_status(&pool, id(999)).await.unwrap()).unwrap(),
            json!({"vehicle_id":id(999),"history_backfilled_at":null,"status":null,"rivian_session_count":null,"local_session_count":0,"missing_source_count":0})
        );
        // The left join reports one missing source for an existing vehicle with no sessions.
        assert_eq!(get_status(&pool, v).await.unwrap().missing_source_count, 1);
        sqlx::query("INSERT INTO panda.vehicle_credentials(vehicle_id,encrypted_tokens,token_created_at) VALUES($1,$2,now())")
            .bind(v).bind(vec![0u8,1,2]).execute(&pool).await.unwrap();
        let claimed = claim(&pool, v).await.unwrap();
        assert_eq!(claimed.rivian_vehicle_id, "history-20");
        assert!(matches!(
            claim(&pool, v).await,
            Err(ChargeBackfillError::AlreadyRunning)
        ));
        let err = run_claimed(&pool, &reqwest::Client::new(), "invalid-key", claimed)
            .await
            .unwrap_err();
        assert!(err.to_string().starts_with("age key parse failed:"));
        assert_eq!(
            get_status(&pool, v).await.unwrap().status.as_deref(),
            Some("error")
        );
        let _ = claim(&pool, v).await.unwrap();
        sqlx::query(
            "UPDATE panda.vehicles SET updated_at=now()-interval '121 minutes' WHERE id=$1",
        )
        .bind(v)
        .execute(&pool)
        .await
        .unwrap();
        assert!(claim(&pool, v).await.is_ok());
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,source) VALUES($1,$2,'2026-08-01T10:00Z',NULL),($3,$2,'2026-08-02T10:00Z','telemetry')")
            .bind(id(2001)).bind(v).bind(id(2002)).execute(&pool).await.unwrap();
        let status = get_status(&pool, v).await.unwrap();
        assert_eq!(
            (status.local_session_count, status.missing_source_count),
            (2, 1)
        );
        cleanup(pool, admin, name).await;
    }
}

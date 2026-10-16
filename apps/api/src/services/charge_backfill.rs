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

#[derive(sqlx::FromRow)]
struct ChargeHistoryStatusRow {
    history_backfilled_at: Option<chrono::DateTime<chrono::Utc>>,
    status: Option<String>,
    rivian_session_count: Option<i32>,
    local_session_count: i64,
    missing_source_count: i64,
}

pub async fn get_status(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<ChargeBackfillStatus, ChargeBackfillError> {
    let row = sqlx::query_as::<_, ChargeHistoryStatusRow>(
        "SELECT v.history_backfilled_at, v.history_backfill_status AS status,
                v.history_session_count AS rivian_session_count,
                COUNT(cs.id) AS local_session_count,
                COUNT(*) FILTER (WHERE cs.source IS NULL) AS missing_source_count
         FROM panda.vehicles v
         LEFT JOIN panda.charge_sessions cs ON cs.vehicle_id = v.id
         WHERE v.id = $1
         GROUP BY v.history_backfilled_at, v.history_backfill_status, v.history_session_count",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(row) => ChargeBackfillStatus {
            vehicle_id,
            history_backfilled_at: row.history_backfilled_at,
            status: row.status,
            rivian_session_count: row.rivian_session_count,
            local_session_count: row.local_session_count,
            missing_source_count: row.missing_source_count,
        },
        None => ChargeBackfillStatus {
            vehicle_id,
            history_backfilled_at: None,
            status: None,
            rivian_session_count: None,
            local_session_count: 0,
            missing_source_count: 0,
        },
    })
}

pub async fn claim(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Result<ClaimedChargeBackfill, ChargeBackfillError> {
    jobs::claim(pool, vehicle_id).await
}

pub async fn run_claimed(
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
    claimed: ClaimedChargeBackfill,
) -> Result<usize, ChargeBackfillError> {
    execute_charge_history_backfill(pool, claimed.vehicle_id, async {
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
        Ok(())
    })
    .await
}

async fn execute_charge_history_backfill(
    pool: &PgPool,
    vehicle_id: Uuid,
    fetch: impl std::future::Future<Output = Result<(), ChargeBackfillError>>,
) -> Result<usize, ChargeBackfillError> {
    let outcome: Result<usize, ChargeBackfillError> = async {
        fetch.await?;
        canonicalize_charge_sessions(pool, Some(vehicle_id)).await?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*)::int8 FROM panda.charge_sessions WHERE vehicle_id = $1",
        )
        .bind(vehicle_id)
        .fetch_one(pool)
        .await?;
        Ok(count as usize)
    }
    .await;
    match outcome {
        Ok(count) => {
            sqlx::query(
                "UPDATE panda.vehicles SET history_backfill_status = 'done',
                     history_backfilled_at = now(), history_session_count = $2, updated_at = now()
                 WHERE id = $1",
            )
            .bind(vehicle_id)
            .bind(count as i32)
            .execute(pool)
            .await?;
            Ok(count)
        }
        Err(error) => {
            tracing::warn!(vehicle_id = %vehicle_id, error = %error, "charge history backfill failed");
            // A status write must not replace the original fetch/reconciliation error.
            let _ = sqlx::query(
                "UPDATE panda.vehicles SET history_backfill_status = 'error', updated_at = now() WHERE id = $1",
            ).bind(vehicle_id).execute(pool).await;
            Err(error)
        }
    }
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
    let task = jobs::BackfillTask::admit(pool, age_key, vehicle_id).await?;
    drop(task.launch());
    Ok(ChargeBackfillStart {
        ok: true,
        status: "running",
    })
}

#[cfg(test)]
mod charge_history_backfill_contract {
    use super::*;
    use crate::services::weather_enrichment::job_fixture::{cleanup, database, id, vehicle};
    use serde_json::json;

    #[tokio::test]
    #[ignore = "requires unique restricted JOBS_DATABASE_URL; never contacts a provider"]
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

    #[tokio::test]
    #[ignore = "requires unique restricted JOBS_DATABASE_URL; synthetic fetch only"]
    async fn charge_history_completion_records_counts_and_preserves_failure_types() {
        let (pool, admin, name) = database().await;
        let v = vehicle(&pool, 50).await;
        let other = vehicle(&pool, 51).await;
        sqlx::query("INSERT INTO panda.vehicle_credentials(vehicle_id,encrypted_tokens,token_created_at) VALUES($1,$2,now())")
            .bind(v).bind(vec![0u8]).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at) VALUES($1,$3,'2026-08-06T10:00Z','2026-08-06T11:00Z'),($2,$3,'2026-08-06T10:10Z','2026-08-06T10:40Z')")
            .bind(id(5001)).bind(id(5002)).bind(v).execute(&pool).await.unwrap();
        let _ = claim(&pool, v).await.unwrap();
        assert_eq!(
            execute_charge_history_backfill(&pool, v, async { Ok(()) })
                .await
                .unwrap(),
            1
        );
        let status = get_status(&pool, v).await.unwrap();
        assert_eq!(status.status.as_deref(), Some("done"));
        assert!(status.history_backfilled_at.is_some());
        assert_eq!(
            (
                status.rivian_session_count,
                status.local_session_count,
                status.missing_source_count
            ),
            (Some(1), 1, 0)
        );
        assert!(get_status(&pool, other).await.unwrap().status.is_none());

        let _ = claim(&pool, v).await.unwrap();
        let error = execute_charge_history_backfill(&pool, v, async {
            Err(ChargeBackfillError::Other(anyhow::anyhow!(
                "synthetic fetch failure"
            )))
        })
        .await
        .unwrap_err();
        assert!(matches!(error, ChargeBackfillError::Other(_)));
        assert_eq!(error.to_string(), "synthetic fetch failure");
        let status = get_status(&pool, v).await.unwrap();
        assert_eq!(status.status.as_deref(), Some("error"));
        assert!(status.history_backfilled_at.is_none());
        assert_eq!(status.rivian_session_count, None);

        let _ = claim(&pool, v).await.unwrap();
        sqlx::query("ALTER TABLE panda.vehicles ADD CONSTRAINT fixture_reject_done CHECK (history_backfill_status IS DISTINCT FROM 'done')").execute(&pool).await.unwrap();
        assert!(matches!(
            execute_charge_history_backfill(&pool, v, async { Ok(()) }).await,
            Err(ChargeBackfillError::Database(_))
        ));
        assert_eq!(
            get_status(&pool, v).await.unwrap().status.as_deref(),
            Some("running")
        );
        sqlx::query("ALTER TABLE panda.vehicles DROP CONSTRAINT fixture_reject_done")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE panda.vehicles ADD CONSTRAINT fixture_reject_error CHECK (history_backfill_status IS DISTINCT FROM 'error')").execute(&pool).await.unwrap();
        let error = execute_charge_history_backfill(&pool, v, async {
            Err(ChargeBackfillError::Other(anyhow::anyhow!(
                "original error survives status failure"
            )))
        })
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "original error survives status failure");
        cleanup(pool, admin, name).await;
    }
}

#[cfg(test)]
#[path = "__tests__/charge_backfill_jobs.rs"]
mod job_tests;

#[path = "charge_backfill/jobs.rs"]
mod jobs;

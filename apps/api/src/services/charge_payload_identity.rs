//! Restart-safe backfill for semantic charge-payload identities.
//!
//! Migration 0007 adds the durable identity schema. This worker fills
//! historical rows and periodically clears cache pointers to JSON evidence
//! that TimescaleDB retention has expired.

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::time::Duration;

const JOB_KEY: &str = "charge_payload_identity";
const ADVISORY_LOCK_KEY: &str = "panda-charge-payload-identity-backfill";
const PENDING_INDEX_NAME: &str = "rivian_charge_payloads_identity_pending_idx";
const DEFAULT_BATCH_SIZE: i64 = 1_000;
const DEFAULT_PAUSE_MS: u64 = 100;
const MIN_BATCH_SIZE: i64 = 100;
const MAX_BATCH_SIZE: i64 = 10_000;
const MAX_PAUSE_MS: u64 = 5_000;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
const RETENTION_RECONCILE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RETENTION_RECONCILE_BATCH_SIZE: i64 = 1_000;
const DROP_PENDING_INDEX_SQL: &str =
    "DROP INDEX CONCURRENTLY IF EXISTS panda.rivian_charge_payloads_identity_pending_idx";
const DROP_PENDING_INDEX_LOCKING_SQL: &str =
    "DROP INDEX IF EXISTS panda.rivian_charge_payloads_identity_pending_idx";
const CREATE_PENDING_INDEX_SQL: &str =
    "CREATE INDEX CONCURRENTLY IF NOT EXISTS rivian_charge_payloads_identity_pending_idx
     ON panda.rivian_charge_payloads (captured_at, id)
     WHERE payload_fingerprint IS NULL";
const CREATE_PENDING_INDEX_LOCKING_SQL: &str =
    "CREATE INDEX IF NOT EXISTS rivian_charge_payloads_identity_pending_idx
     ON panda.rivian_charge_payloads (captured_at, id)
     WHERE payload_fingerprint IS NULL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillConfig {
    pub batch_size: i64,
    pub pause: Duration,
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            pause: Duration::from_millis(DEFAULT_PAUSE_MS),
        }
    }
}

impl BackfillConfig {
    pub fn from_env() -> Result<Self> {
        let batch_size = parse_i64_env("CHARGE_IDENTITY_BACKFILL_BATCH_SIZE", DEFAULT_BATCH_SIZE)?;
        if !(MIN_BATCH_SIZE..=MAX_BATCH_SIZE).contains(&batch_size) {
            anyhow::bail!(
                "CHARGE_IDENTITY_BACKFILL_BATCH_SIZE must be between {MIN_BATCH_SIZE} and {MAX_BATCH_SIZE}"
            );
        }

        let pause_ms = parse_u64_env("CHARGE_IDENTITY_BACKFILL_PAUSE_MS", DEFAULT_PAUSE_MS)?;
        if pause_ms > MAX_PAUSE_MS {
            anyhow::bail!("CHARGE_IDENTITY_BACKFILL_PAUSE_MS must be between 0 and {MAX_PAUSE_MS}");
        }

        Ok(Self {
            batch_size,
            pause: Duration::from_millis(pause_ms),
        })
    }
}

fn parse_i64_env(name: &str, default: i64) -> Result<i64> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<i64>()
            .with_context(|| format!("{name} must be an integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("read {name}")),
    }
}

fn parse_u64_env(name: &str, default: u64) -> Result<u64> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .with_context(|| format!("{name} must be an unsigned integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("read {name}")),
    }
}

pub fn start_worker(pool: PgPool, config: BackfillConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut retry_delay = Duration::from_secs(1);
        loop {
            match run_until_complete(&pool, config).await {
                Ok(WorkerResult::Complete) => {
                    retry_delay = Duration::from_secs(1);
                    tokio::time::sleep(RETENTION_RECONCILE_INTERVAL).await;
                }
                Ok(WorkerResult::Busy) => {
                    retry_delay = Duration::from_secs(1);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                Err(error) => {
                    let error_text = error.to_string();
                    tracing::error!(
                        error = %error,
                        retry_in_ms = retry_delay.as_millis() as u64,
                        "charge_payload_identity_worker_failed"
                    );
                    if let Err(status_error) = mark_error(&pool, &error_text).await {
                        tracing::error!(error = %status_error, "charge_payload_identity_backfill_status_failed");
                    }
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = next_retry_delay(retry_delay);
                }
            }
        }
    })
}

fn next_retry_delay(current: Duration) -> Duration {
    current
        .checked_mul(2)
        .unwrap_or(MAX_RETRY_DELAY)
        .min(MAX_RETRY_DELAY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerResult {
    Complete,
    Busy,
}

async fn run_until_complete(pool: &PgPool, config: BackfillConfig) -> Result<WorkerResult> {
    let mut lock_connection = pool.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 0))")
        .bind(ADVISORY_LOCK_KEY)
        .fetch_one(&mut *lock_connection)
        .await?;
    if !locked {
        return Ok(WorkerResult::Busy);
    }

    let result = run_locked(pool, config).await;
    let unlock_result =
        sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(ADVISORY_LOCK_KEY)
            .fetch_one(&mut *lock_connection)
            .await;

    match (result, unlock_result) {
        (Ok(result), Ok(true)) => Ok(result),
        (Ok(_), Ok(false)) => anyhow::bail!("charge payload identity backfill lock was not held"),
        (Ok(_), Err(error)) => Err(error).context("release charge payload identity backfill lock"),
        (Err(error), Ok(_)) => Err(error),
        (Err(error), Err(unlock_error)) => Err(error.context(format!(
            "also failed to release backfill lock: {unlock_error}"
        ))),
    }
}

async fn run_locked(pool: &PgPool, config: BackfillConfig) -> Result<WorkerResult> {
    let mut remaining_estimate = pending_count(pool).await?;
    if remaining_estimate != 0 {
        ensure_pending_index(pool).await?;
        mark_started(pool).await?;
        tracing::info!(
            pending = remaining_estimate,
            "charge_payload_identity_backfill_started"
        );
        loop {
            let (rows_filled, identities_inserted) = process_batch(pool, config.batch_size).await?;
            if rows_filled != 0 {
                remaining_estimate = remaining_estimate.saturating_sub(rows_filled);
                tracing::info!(
                    rows_filled,
                    identities_inserted,
                    remaining_estimate,
                    "charge_payload_identity_backfill_progress"
                );
                tokio::time::sleep(config.pause).await;
            }
            // A short claim may mean locked rows, so only an exact count ends the run.
            if rows_filled == 0 || rows_filled < config.batch_size {
                if pending_count(pool).await? == 0 {
                    break;
                }
                if rows_filled == 0 {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
        mark_complete(pool).await?;
        reconcile_retained_payload_references(pool, RETENTION_RECONCILE_BATCH_SIZE).await?;
        tracing::info!("charge_payload_identity_backfill_complete");
    } else {
        mark_complete(pool).await?;
        reconcile_retained_payload_references(pool, RETENTION_RECONCILE_BATCH_SIZE).await?;
    }
    Ok(WorkerResult::Complete)
}

async fn ensure_pending_index(pool: &PgPool) -> Result<()> {
    let is_hypertable: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1
             FROM timescaledb_information.hypertables
             WHERE hypertable_schema = 'panda'
               AND hypertable_name = 'rivian_charge_payloads'
         )",
    )
    .fetch_one(pool)
    .await?;
    let validity: Option<bool> = sqlx::query_scalar(
        "SELECT i.indisvalid
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indexrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'panda' AND c.relname = $1",
    )
    .bind(PENDING_INDEX_NAME)
    .fetch_optional(pool)
    .await?;

    if validity == Some(false) {
        sqlx::query(if is_hypertable {
            DROP_PENDING_INDEX_LOCKING_SQL
        } else {
            DROP_PENDING_INDEX_SQL
        })
        .execute(pool)
        .await?;
    }

    // TimescaleDB does not support CREATE INDEX CONCURRENTLY on a hypertable.
    // This branch still runs only after the API is serving traffic; ordinary
    // PostgreSQL tables retain the non-blocking concurrent build.
    sqlx::query(if is_hypertable {
        CREATE_PENDING_INDEX_LOCKING_SQL
    } else {
        CREATE_PENDING_INDEX_SQL
    })
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ChargeHistoryFilledPayload {
    id: uuid::Uuid,
    vehicle_id: uuid::Uuid,
    operation: String,
    rivian_transaction_id: Option<String>,
    rivian_vehicle_id: Option<String>,
    payload_fingerprint: Vec<u8>,
}

async fn process_batch(pool: &PgPool, batch_size: i64) -> Result<(i64, i64)> {
    let mut transaction = pool.begin().await?;
    let filled = sqlx::query_as::<_, ChargeHistoryFilledPayload>(
        r#"WITH claim AS MATERIALIZED (
               SELECT id, payload
               FROM panda.rivian_charge_payloads
               WHERE payload_fingerprint IS NULL
               ORDER BY captured_at, id
               FOR UPDATE SKIP LOCKED LIMIT $1
           ), fingerprints AS MATERIALIZED (
               SELECT id, panda.charge_payload_fingerprint(payload) AS fingerprint FROM claim
           )
           UPDATE panda.rivian_charge_payloads payload
           SET payload_fingerprint = fingerprints.fingerprint
           FROM fingerprints
           WHERE payload.id = fingerprints.id AND payload.payload_fingerprint IS NULL
           RETURNING payload.id, payload.vehicle_id, payload.operation,
                     payload.rivian_transaction_id, payload.rivian_vehicle_id, payload.payload_fingerprint"#,
    ).bind(batch_size).fetch_all(&mut *transaction).await?;
    let rows_filled = filled.len() as i64;
    let ids: Vec<_> = filled.iter().map(|p| p.id).collect();
    let vehicles: Vec<_> = filled.iter().map(|p| p.vehicle_id).collect();
    let operations: Vec<_> = filled.iter().map(|p| p.operation.as_str()).collect();
    let transactions: Vec<_> = filled
        .iter()
        .map(|p| p.rivian_transaction_id.as_deref())
        .collect();
    let upstream_vehicles: Vec<_> = filled
        .iter()
        .map(|p| p.rivian_vehicle_id.as_deref())
        .collect();
    let fingerprints: Vec<_> = filled
        .iter()
        .map(|p| p.payload_fingerprint.as_slice())
        .collect();
    let identities_inserted = sqlx::query(
        r#"INSERT INTO panda.rivian_charge_payload_identities
               (identity_key, vehicle_id, operation, payload_fingerprint, canonical_payload_id)
           SELECT panda.charge_payload_identity_key(vehicle_id, operation, transaction_id, upstream_vehicle, fingerprint),
                  vehicle_id, operation, fingerprint, id
           FROM unnest($1::uuid[], $2::uuid[], $3::text[], $4::text[], $5::text[], $6::bytea[])
               AS batch(id, vehicle_id, operation, transaction_id, upstream_vehicle, fingerprint)
           ON CONFLICT (identity_key) DO NOTHING"#,
    ).bind(&ids).bind(&vehicles).bind(&operations).bind(&transactions).bind(&upstream_vehicles).bind(&fingerprints)
     .execute(&mut *transaction).await?.rows_affected() as i64;

    sqlx::query(
        "UPDATE panda.charge_payload_identity_backfill_status
         SET status = 'running',
             rows_scanned = rows_scanned + $1,
             fingerprints_filled = fingerprints_filled + $1,
             identities_inserted = identities_inserted + $2,
             last_error = NULL,
             heartbeat_at = now(),
             updated_at = now()
         WHERE job_key = $3",
    )
    .bind(rows_filled)
    .bind(identities_inserted)
    .bind(JOB_KEY)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;
    Ok((rows_filled, identities_inserted))
}

/// Clear only cache pointers whose target JSON has already disappeared. This
/// never creates payloads or deletes identities: the identity key and
/// fingerprint are the durable replay ledger after retention expiry.
pub(crate) async fn reconcile_retained_payload_references(
    pool: &PgPool,
    batch_size: i64,
) -> Result<(i64, i64)> {
    let mut transaction = pool.begin().await?;
    let (identities_cleared, aliases_cleared): (i64, i64) = sqlx::query_as(
        r#"WITH orphaned_identities AS MATERIALIZED (
               SELECT identity.identity_key, identity.canonical_payload_id
               FROM panda.rivian_charge_payload_identities identity
               LEFT JOIN panda.rivian_charge_payloads payload
                 ON payload.id = identity.canonical_payload_id
               WHERE identity.canonical_payload_id IS NOT NULL
                 AND payload.id IS NULL
               ORDER BY identity.created_at, identity.identity_key
               FOR UPDATE OF identity SKIP LOCKED
               LIMIT $1
           ), orphaned_aliases AS MATERIALIZED (
               SELECT alias.charge_session_id, alias.external_id
               FROM panda.charge_session_external_aliases alias
               LEFT JOIN panda.rivian_charge_payloads payload
                 ON payload.id = alias.latest_payload_id
               WHERE alias.latest_payload_id IS NOT NULL
                 AND payload.id IS NULL
               ORDER BY alias.updated_at, alias.charge_session_id, alias.external_id
               FOR UPDATE OF alias SKIP LOCKED
               LIMIT $2
           ), cleared_identities AS (
               UPDATE panda.rivian_charge_payload_identities identity
               SET canonical_payload_id = NULL
               FROM orphaned_identities orphaned
               WHERE identity.identity_key = orphaned.identity_key
               RETURNING 1
           ), cleared_aliases AS (
               UPDATE panda.charge_session_external_aliases alias
               SET latest_payload_id = NULL,
                   latest_payload_captured_at = NULL
               FROM orphaned_aliases orphaned
               WHERE alias.charge_session_id = orphaned.charge_session_id
                 AND alias.external_id = orphaned.external_id
               RETURNING 1
           )
           SELECT
               (SELECT count(*) FROM cleared_identities)::bigint,
               (SELECT count(*) FROM cleared_aliases)::bigint"#,
    )
    .bind(batch_size)
    .bind(batch_size)
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;

    tracing::info!(
        identities_cleared,
        aliases_cleared,
        "charge_payload_retention_reconciled"
    );
    Ok((identities_cleared, aliases_cleared))
}

async fn pending_count(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*)::bigint
         FROM panda.rivian_charge_payloads
         WHERE payload_fingerprint IS NULL",
    )
    .fetch_one(pool)
    .await?)
}

async fn mark_started(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "UPDATE panda.charge_payload_identity_backfill_status
         SET status = 'running',
             last_error = NULL,
             started_at = now(),
             completed_at = NULL,
             heartbeat_at = now(),
             updated_at = now()
         WHERE job_key = $1",
    )
    .bind(JOB_KEY)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_complete(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "UPDATE panda.charge_payload_identity_backfill_status
         SET status = 'complete',
             last_error = NULL,
             completed_at = COALESCE(completed_at, now()),
             heartbeat_at = now(),
             updated_at = now()
         WHERE job_key = $1",
    )
    .bind(JOB_KEY)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_error(pool: &PgPool, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE panda.charge_payload_identity_backfill_status
         SET status = 'error',
             last_error = $1,
             heartbeat_at = now(),
             updated_at = now()
         WHERE job_key = $2",
    )
    .bind(error)
    .bind(JOB_KEY)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod charge_history_identity_contract {
    use super::*;
    use crate::services::charge_sessions::charge_history_contract::{
        cleanup, database, evidence, id, vehicle,
    };
    use serde_json::json;

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL"]
    async fn charge_history_identity_batches_retention_and_atomic_failure() {
        let (pool, admin, name) = database().await;
        let v = vehicle(&pool, 10).await;
        let other = vehicle(&pool, 11).await;
        let payloads = [
            (
                id(1001),
                v,
                "op",
                Some("txn"),
                json!({"energy":1,"nested":{"updatedAt":"one"},"array":[1,null]}),
            ),
            (
                id(1002),
                v,
                "op",
                Some("txn"),
                json!({"nested":{"updatedAt":"two"},"array":[1,null],"energy":1.0}),
            ),
            (
                id(1003),
                v,
                "op",
                None,
                json!({"energy":1,"array":[1,null],"nested":{}}),
            ),
            (
                id(1004),
                other,
                "op",
                Some("txn"),
                json!({"energy":1,"array":[1,null],"nested":{}}),
            ),
            (
                id(1005),
                v,
                "other",
                Some("txn"),
                json!({"energy":1,"array":[1,null],"nested":{}}),
            ),
            (
                id(1006),
                v,
                "op",
                Some("txn"),
                json!({"energy":null,"array":[null,1],"nested":{}}),
            ),
            (
                id(1007),
                v,
                "op",
                Some("txn"),
                json!({"energy":1,"nested":{"updatedAt":"three"},"array":[1,null]}),
            ),
        ];
        for (i, (p, v, op, txn, payload)) in payloads.iter().enumerate() {
            sqlx::query("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,operation,rivian_transaction_id,captured_at,payload) VALUES($1,$2,$3,$4,'2026-08-01T10:00Z'::timestamptz+$5*interval '1 second',$6)")
                .bind(p).bind(v).bind(op).bind(txn).bind(i as f64).bind(payload).execute(&pool).await.unwrap();
        }
        // The durable status and the payload writes must roll back together.
        sqlx::query("ALTER TABLE panda.charge_payload_identity_backfill_status ADD CONSTRAINT fixture_reject_progress CHECK (rows_scanned=0)").execute(&pool).await.unwrap();
        assert!(process_batch(&pool, 2).await.is_err());
        assert_eq!(pending_count(&pool).await.unwrap(), 7);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM panda.rivian_charge_payload_identities"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        sqlx::query("ALTER TABLE panda.charge_payload_identity_backfill_status DROP CONSTRAINT fixture_reject_progress").execute(&pool).await.unwrap();
        let mut lock = pool.acquire().await.unwrap();
        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1,0))")
            .bind(ADVISORY_LOCK_KEY)
            .execute(&mut *lock)
            .await
            .unwrap();
        assert_eq!(
            run_until_complete(&pool, BackfillConfig::default())
                .await
                .unwrap(),
            WorkerResult::Busy
        );
        sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1,0))")
            .bind(ADVISORY_LOCK_KEY)
            .execute(&mut *lock)
            .await
            .unwrap();
        drop(lock);
        mark_started(&pool).await.unwrap();
        let first = process_batch(&pool, 1).await.unwrap();
        let rest = process_batch(&pool, 10).await.unwrap();
        assert_eq!(first, (1, 1));
        // PostgreSQL jsonb numeric text retains 1 versus 1.0 for fingerprints.
        assert_eq!(rest, (6, 5));
        assert_eq!(process_batch(&pool, 10).await.unwrap(), (0, 0));
        assert_eq!(
            run_until_complete(
                &pool,
                BackfillConfig {
                    batch_size: 2,
                    pause: Duration::ZERO
                }
            )
            .await
            .unwrap(),
            WorkerResult::Complete
        );
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at) VALUES($1,$2,'2026-08-01T10:00Z')")
            .bind(id(1050)).bind(v).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,latest_payload_id,latest_payload_captured_at) VALUES($1,'expired','txn',$2,'2026-08-01T10:00Z')")
            .bind(id(1050)).bind(id(1001)).execute(&pool).await.unwrap();
        evidence(&pool, "identity-filled", json!({"first":first,"rest":rest})).await;
        sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=$1")
            .bind(id(1001))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            reconcile_retained_payload_references(&pool, 1)
                .await
                .unwrap(),
            (1, 1)
        );
        assert_eq!(
            reconcile_retained_payload_references(&pool, 1)
                .await
                .unwrap(),
            (0, 0)
        );
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM panda.rivian_charge_payload_identities WHERE canonical_payload_id IS NULL").fetch_one(&pool).await.unwrap(),1);
        evidence(
            &pool,
            "identity-retained",
            json!({"pending":pending_count(&pool).await.unwrap()}),
        )
        .await;
        cleanup(pool, admin, name).await;
    }
}

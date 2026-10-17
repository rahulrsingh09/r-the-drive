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
        let mut config = Self::default();
        config.batch_size =
            parse_i64_env("CHARGE_IDENTITY_BACKFILL_BATCH_SIZE", config.batch_size)?;
        match config.batch_size {
            MIN_BATCH_SIZE..=MAX_BATCH_SIZE => {},
            _ => anyhow::bail!("CHARGE_IDENTITY_BACKFILL_BATCH_SIZE must be between {MIN_BATCH_SIZE} and {MAX_BATCH_SIZE}"),
        }
        let pause = parse_u64_env("CHARGE_IDENTITY_BACKFILL_PAUSE_MS", DEFAULT_PAUSE_MS)?;
        config.pause = match pause {
            0..=MAX_PAUSE_MS => Duration::from_millis(pause),
            _ => anyhow::bail!(
                "CHARGE_IDENTITY_BACKFILL_PAUSE_MS must be between 0 and {MAX_PAUSE_MS}"
            ),
        };
        Ok(config)
    }
}

fn parse_i64_env(name: &str, default: i64) -> Result<i64> {
    parse_env(name, default, "an integer")
}

fn parse_u64_env(name: &str, default: u64) -> Result<u64> {
    parse_env(name, default, "an unsigned integer")
}

pub fn start_worker(pool: PgPool, config: BackfillConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut retry = Duration::from_secs(1);
        loop {
            let delay = match run_until_complete(&pool, config).await {
                Ok(outcome) => {
                    retry = Duration::from_secs(1);
                    match outcome {
                        WorkerResult::Complete => RETENTION_RECONCILE_INTERVAL,
                        WorkerResult::Busy => Duration::from_secs(30),
                    }
                }
                Err(error) => {
                    tracing::error!(error = %error, retry_in_ms = retry.as_millis() as u64,
                        "charge_payload_identity_worker_failed");
                    if let Err(status_error) = mark_error(&pool, &error.to_string()).await {
                        tracing::error!(error = %status_error, "charge_payload_identity_backfill_status_failed");
                    }
                    let delay = retry;
                    retry = next_retry_delay(retry);
                    delay
                }
            };
            tokio::time::sleep(delay).await;
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
    let mut connection = pool.acquire().await?;
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 0))")
        .bind(ADVISORY_LOCK_KEY)
        .fetch_one(&mut *connection)
        .await?;
    if !acquired {
        return Ok(WorkerResult::Busy);
    }
    let work = run_locked(pool, config).await;
    let release =
        sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(ADVISORY_LOCK_KEY)
            .fetch_one(&mut *connection)
            .await;
    match work {
        Err(error) => match release {
            Err(unlock_error) => Err(error.context(format!(
                "also failed to release backfill lock: {unlock_error}"
            ))),
            Ok(_) => Err(error),
        },
        Ok(outcome) => {
            if release.context("release charge payload identity backfill lock")? {
                Ok(outcome)
            } else {
                anyhow::bail!("charge payload identity backfill lock was not held")
            }
        }
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
    let (hypertable, valid): (bool, Option<bool>) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM timescaledb_information.hypertables
             WHERE hypertable_schema='panda' AND hypertable_name='rivian_charge_payloads'),
           (SELECT i.indisvalid FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid
            JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='panda' AND c.relname=$1)",
    )
    .bind(PENDING_INDEX_NAME)
    .fetch_one(pool)
    .await?;
    let (drop_index, create_index) = if hypertable {
        (
            DROP_PENDING_INDEX_LOCKING_SQL,
            CREATE_PENDING_INDEX_LOCKING_SQL,
        )
    } else {
        (DROP_PENDING_INDEX_SQL, CREATE_PENDING_INDEX_SQL)
    };
    for statement in [
        (valid == Some(false)).then_some(drop_index),
        Some(create_index),
    ]
    .into_iter()
    .flatten()
    {
        sqlx::query(statement).execute(pool).await?;
    }
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
               WHERE identity.canonical_payload_id IS NOT NULL
                 AND NOT EXISTS (SELECT 1 FROM panda.rivian_charge_payloads payload
                                 WHERE payload.id=identity.canonical_payload_id)
               ORDER BY identity.created_at, identity.identity_key
               FOR UPDATE OF identity SKIP LOCKED
               LIMIT $1
           ), orphaned_aliases AS MATERIALIZED (
               SELECT alias.charge_session_id, alias.external_id
               FROM panda.charge_session_external_aliases alias
               WHERE alias.latest_payload_id IS NOT NULL
                 AND NOT EXISTS (SELECT 1 FROM panda.rivian_charge_payloads payload
                                 WHERE payload.id=alias.latest_payload_id)
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
    write_backfill_status(pool, "running", None).await
}

async fn mark_complete(pool: &PgPool) -> Result<()> {
    write_backfill_status(pool, "complete", None).await
}

async fn mark_error(pool: &PgPool, error: &str) -> Result<()> {
    write_backfill_status(pool, "error", Some(error)).await
}

fn parse_env<T>(name: &str, default: T, expected: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let raw = match std::env::var(name) {
        Ok(raw) => Some(raw),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error).with_context(|| format!("read {name}")),
    };
    raw.map_or(Ok(default), |raw| {
        raw.parse()
            .with_context(|| format!("{name} must be {expected}"))
    })
}

async fn write_backfill_status(pool: &PgPool, status: &str, error: Option<&str>) -> Result<()> {
    sqlx::query(
        "UPDATE panda.charge_payload_identity_backfill_status
         SET status=$1, last_error=$2, heartbeat_at=now(), updated_at=now(),
             started_at=CASE WHEN $1='running' THEN now() ELSE started_at END,
             completed_at=CASE WHEN $1='running' THEN NULL
                               WHEN $1='complete' THEN COALESCE(completed_at,now()) ELSE completed_at END
         WHERE job_key=$3"
    ).bind(status).bind(error).bind(JOB_KEY).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "__tests__/charge_payload_identity_charge_history_identity_contract.rs"]
mod charge_history_identity_contract;

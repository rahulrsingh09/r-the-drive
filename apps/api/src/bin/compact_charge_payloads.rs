//! Report or remove exact duplicate Rivian charge-history payloads.
//!
//! Usage:
//!   cargo run --bin compact_charge_payloads -- [--vehicle <uuid>] [--apply] [--batch-size <count>]

use anyhow::{anyhow, Context, Result};
use panda_api::db::pool::create_pool;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Args {
    vehicle_id: Option<Uuid>,
    apply: bool,
    batch_size: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let pool = create_pool(&database_url).await?;
    let outcome = async {
        let (relation_bytes, payload_bytes, duplicate_count, pending_identity_count) =
            storage_snapshot(&pool, args.vehicle_id).await?;
        if !args.apply {
            println!("Charge payload storage: {relation_bytes} relation bytes, {payload_bytes} payload bytes.");
            println!("Found {duplicate_count} semantically duplicate charge payloads. Re-run with --apply to remove them.");
            println!("Identity backfill has {pending_identity_count} payloads remaining; compaction will refuse to apply until it reaches zero.");
            return Ok(());
        }
        if pending_identity_count > 0 {
            return Err(anyhow!("charge-payload compaction is unsafe while {pending_identity_count} payloads lack semantic identities; wait for the identity backfill to complete"));
        }
        let removed = compact_batch(&pool, &args).await?;
        println!("Removed {} exact duplicate charge payloads; retained one linked payload when available.", removed);
        if removed as i64 == args.batch_size && duplicate_count > args.batch_size {
            println!("More duplicates remain; run the same command again for the next bounded batch.");
        }
        Ok(())
    }.await;
    pool.close().await;
    outcome
}

async fn storage_snapshot(
    pool: &sqlx::PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<(i64, i64, i64, i64)> {
    Ok(sqlx::query_as(r#"
        WITH measured AS (
            SELECT pg_column_size(payload) AS bytes, payload_fingerprint IS NULL AS pending,
                   row_number() OVER (PARTITION BY panda.charge_payload_identity_key(
                       vehicle_id, operation, rivian_transaction_id, rivian_vehicle_id, payload_fingerprint
                   )) AS ordinal
            FROM panda.rivian_charge_payloads
            WHERE ($1::uuid IS NULL OR vehicle_id = $1)
        )
        SELECT pg_total_relation_size('panda.rivian_charge_payloads')::bigint,
               COALESCE(sum(bytes), 0)::bigint,
               count(*) FILTER (WHERE ordinal > 1)::bigint,
               count(*) FILTER (WHERE pending)::bigint
        FROM measured
    "#).bind(vehicle_id).fetch_one(pool).await?)
}

async fn compact_batch(pool: &sqlx::PgPool, args: &Args) -> Result<u64> {
    let mut transaction = pool.begin().await?;
    let acquired: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtextextended('panda-charge-payload-compaction', 0))",
    )
    .fetch_one(&mut *transaction)
    .await?;
    if !acquired {
        transaction.rollback().await?;
        return Err(anyhow!(
            "charge-payload compaction is already running; retry after it finishes"
        ));
    }
    let deletion = sqlx::query(
        r#"
        WITH payloads AS MATERIALIZED (
            SELECT payload.id, payload.captured_at, session.id IS NOT NULL AS linked,
                   panda.charge_payload_identity_key(
                       payload.vehicle_id, payload.operation, payload.rivian_transaction_id,
                       payload.rivian_vehicle_id, payload.payload_fingerprint
                   ) AS identity_key
            FROM panda.rivian_charge_payloads payload
            LEFT JOIN panda.charge_sessions session
              ON session.id = payload.charge_session_id AND session.vehicle_id = payload.vehicle_id
            WHERE ($1::uuid IS NULL OR payload.vehicle_id = $1)
        ), ranked AS (
            SELECT id, captured_at, identity_key,
                   first_value(id) OVER keeper AS keeper_id,
                   first_value(captured_at) OVER keeper AS keeper_captured_at
            FROM payloads
            WINDOW keeper AS (PARTITION BY identity_key ORDER BY linked DESC, captured_at, id)
        ), victims AS (
            SELECT id, captured_at, keeper_id, keeper_captured_at
            FROM ranked
            WHERE identity_key IS NOT NULL
              AND (id, captured_at) <> (keeper_id, keeper_captured_at)
            ORDER BY captured_at, id LIMIT $2
        ), repointed_aliases AS (
            UPDATE panda.charge_session_external_aliases alias
            SET latest_payload_id = victims.keeper_id,
                latest_payload_captured_at = victims.keeper_captured_at
            FROM victims
            WHERE alias.latest_payload_id = victims.id
              AND (alias.latest_payload_id, alias.latest_payload_captured_at)
                  IS DISTINCT FROM (victims.keeper_id, victims.keeper_captured_at)
            RETURNING alias.charge_session_id
        ), repointed_identities AS (
            UPDATE panda.rivian_charge_payload_identities identities
            SET canonical_payload_id = victims.keeper_id
            FROM victims
            WHERE identities.canonical_payload_id = victims.id
            RETURNING identities.identity_key
        )
        DELETE FROM panda.rivian_charge_payloads payload USING victims
        WHERE payload.id = victims.id AND payload.captured_at = victims.captured_at
    "#,
    )
    .bind(args.vehicle_id)
    .bind(args.batch_size)
    .execute(&mut *transaction)
    .await;
    match deletion {
        Ok(result) => {
            transaction.commit().await?;
            Ok(result.rows_affected())
        }
        Err(error) => {
            transaction.rollback().await?;
            Err(error.into())
        }
    }
}
fn parse_args() -> Result<Args> {
    use panda_api::db::maintenance::{command_options, OptionKind};
    let options = command_options(&[OptionKind::Vehicle, OptionKind::Apply, OptionKind::BatchSize],
        "Usage: cargo run --bin compact_charge_payloads -- [--vehicle <uuid>] [--apply] [--batch-size <count>]", "unknown argument")?;
    Ok(Args {
        vehicle_id: options.vehicle_id,
        apply: options.apply,
        batch_size: options.batch_size,
    })
}

#[cfg(test)]
#[path = "__tests__/process_contracts.rs"]
mod process_contracts;

#[cfg(test)]
#[path = "__tests__/interruption_contracts.rs"]
mod interruption_contracts;

#[cfg(test)]
#[path = "__tests__/support.rs"]
mod contract_support;

#[cfg(test)]
#[path = "__tests__/evidence_contracts.rs"]
mod evidence_contracts;

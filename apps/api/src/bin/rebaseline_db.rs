use std::{env, path::Path};

use anyhow::{bail, Context};
use panda_api::{
    db::migrations::{self, MigrationIdentity, MIGRATOR},
    services::{backups, restore_compatibility},
};
use sqlx::{postgres::PgPoolOptions, AssertSqlSafe};
use tokio::process::Command;
use url::Url;
use uuid::Uuid;

const ADOPTION_LOCK: &str = "panda-schema-rebaseline";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (confirm, backup_path) = parse_args()?;
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .context("connect to the database")?;
    let outcome = async {
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtext($1))")
            .bind(ADOPTION_LOCK)
            .fetch_one(&pool)
            .await?;
        if !locked {
            bail!("another baseline adoption process holds the database lock");
        }
        let result = adopt(&database_url, &pool, confirm, backup_path.as_deref()).await;
        let _ = sqlx::query("SELECT pg_advisory_unlock(hashtext($1))")
            .bind(ADOPTION_LOCK)
            .execute(&pool)
            .await;
        result
    }
    .await;
    pool.close().await;
    outcome
}

async fn adopt(
    database_url: &str,
    pool: &sqlx::PgPool,
    confirm: bool,
    backup_path: Option<&str>,
) -> anyhow::Result<()> {
    let compiled = migrations::compiled_migration_ledger();
    if is_adopted(pool, &compiled).await? {
        println!("Database already uses the compiled migration chain; no adoption is needed.");
        return Ok(());
    }

    verify_existing_ledger(pool).await?;
    refuse_active_clients(pool).await?;
    let live_contract = restore_compatibility::schema_contract_description(pool)
        .await
        .context("describe live database schema")?;
    let live_fingerprint = contract_fingerprint(&live_contract)?;
    let (baseline_fingerprint, baseline_contract) = scratch_baseline_contract(database_url).await?;
    if live_fingerprint != baseline_fingerprint {
        bail!(
            "database schema contract differs from the compiled baseline (live={live_fingerprint}, baseline={baseline_fingerprint}): {}",
            contract_difference(&live_contract, &baseline_contract)
        );
    }

    if !confirm {
        println!(
            "Preflight passed. The live schema exactly matches the compiled baseline. No changes were made."
        );
        return Ok(());
    }

    let backup_path = backup_path.context("--backup <path> is required with --yes")?;
    verify_backup_evidence(backup_path).await?;
    refuse_active_clients(pool).await?;

    let ledger_remains: bool = sqlx::query_scalar(
        "SELECT to_regclass('public._sqlx_migrations') IS NOT NULL OR to_regclass('panda._sqlx_migrations') IS NOT NULL",
    ).fetch_one(pool).await?;
    if !ledger_remains {
        bail!("the current SQLx migration ledger is missing");
    }
    migrations::restore_baseline_ledger(pool).await?;

    // The flattened baseline represents the complete pre-release schema, but
    // later migrations may add data or schema changes after that baseline.
    // Re-run the compiled catalog from the adopted baseline so those changes
    // are applied and recorded normally instead of silently skipping them.
    MIGRATOR
        .run(pool)
        .await
        .context("apply migrations added after the flattened baseline")?;

    let adopted = read_public_ledger(pool).await?;
    migrations::validate_complete_ledger(&adopted).context("validate adopted migration ledger")?;
    println!(
        "Database adopted migration chain {} at catalog {}.",
        migrations::MIGRATION_CHAIN_ID,
        migrations::migration_catalog_digest()
    );
    Ok(())
}

async fn is_adopted(pool: &sqlx::PgPool, expected: &[MigrationIdentity]) -> anyhow::Result<bool> {
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    if !exists {
        return Ok(false);
    }
    let ledger = read_public_ledger(pool).await?;
    Ok(migrations::validate_complete_ledger(&ledger).is_ok() && ledger == expected)
}

async fn read_public_ledger(pool: &sqlx::PgPool) -> anyhow::Result<Vec<MigrationIdentity>> {
    let rows = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT version, description, encode(checksum, 'hex') FROM public._sqlx_migrations ORDER BY version")
        .fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(
            |(version, description, checksum_sha384)| MigrationIdentity {
                version,
                description,
                checksum_sha384,
            },
        )
        .collect())
}

async fn verify_existing_ledger(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let ledgers: (bool, bool) = sqlx::query_as(
        "SELECT to_regclass('public._sqlx_migrations') IS NOT NULL, to_regclass('panda._sqlx_migrations') IS NOT NULL")
        .fetch_one(pool).await?;
    let query = match ledgers {
        (true, false) => "SELECT version, success FROM public._sqlx_migrations ORDER BY version",
        (false, true) => "SELECT version, success FROM panda._sqlx_migrations ORDER BY version",
        (false, false) => bail!("the current SQLx migration ledger is missing"),
        (true, true) => {
            bail!("both public and historical SQLx ledgers exist; adoption is ambiguous")
        }
    };
    let rows: Vec<(i64, bool)> = sqlx::query_as(query).fetch_all(pool).await?;
    let Some(&(first, _)) = rows.first() else {
        bail!("the existing SQLx migration ledger is empty");
    };
    if rows.iter().any(|(_, success)| !success) {
        bail!("the existing SQLx migration ledger contains a failed migration");
    }
    let mut expected = first;
    for (version, _) in rows {
        if first < 1 || version != expected {
            bail!("the existing SQLx migration ledger is not contiguous and ordered");
        }
        expected = version.saturating_add(1);
    }
    Ok(())
}

async fn refuse_active_clients(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let active_clients: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity
         WHERE datname = current_database()
           AND pid <> pg_backend_pid()
           AND backend_type = 'client backend'",
    )
    .fetch_one(pool)
    .await?;
    if active_clients > 0 {
        bail!(
            "baseline adoption requires stopped API and ingestion writers; found {active_clients} other database client connection(s)"
        );
    }
    Ok(())
}

async fn verify_backup_evidence(path: &str) -> anyhow::Result<()> {
    let path = Path::new(path);
    let metadata = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("backup evidence does not exist: {}", path.display()))?;
    if metadata.len() == 0 || !metadata.is_file() {
        bail!(
            "backup evidence must be a non-empty file: {}",
            path.display()
        );
    }
    let recovery_package = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".rma.tar.gz"));
    if recovery_package {
        return backups::validate_recovery_package(path)
            .await
            .context("validate recovery package supplied as backup evidence")
            .map(|_| ());
    }
    let output = Command::new("pg_restore").args([std::ffi::OsStr::new("--list"), path.as_os_str()])
        .output().await.context("verify raw dump with pg_restore; install PostgreSQL client tools or provide a validated .rma.tar.gz package")?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "pg_restore could not verify backup evidence: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn contract_fingerprint(contract: &serde_json::Value) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_vec(&serde_json::json!({
        "version": restore_compatibility::SCHEMA_CONTRACT_VERSION,
        "items": contract,
    }))?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn contract_difference(live: &serde_json::Value, baseline: &serde_json::Value) -> String {
    let mut membership = std::collections::BTreeMap::<String, u8>::new();
    for (side, value) in [(1, live), (2, baseline)] {
        if let Some(items) = value.as_array() {
            for item in items {
                if let Ok(serialized) = serde_json::to_string(item) {
                    *membership.entry(serialized).or_default() |= side;
                }
            }
        }
    }
    let mut live_only = Vec::new();
    let mut baseline_only = Vec::new();
    for (item, sides) in membership {
        let destination = match sides {
            1 => &mut live_only,
            2 => &mut baseline_only,
            _ => continue,
        };
        if destination.len() < 8 {
            destination.push(item);
        }
    }
    format!("live-only={live_only:?}; baseline-only={baseline_only:?}")
}

async fn scratch_baseline_contract(
    database_url: &str,
) -> anyhow::Result<(String, serde_json::Value)> {
    let mut scratch_url = Url::parse(database_url).context("parse DATABASE_URL")?;
    let mut admin_url = scratch_url.clone();
    admin_url.set_path("/postgres");
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(admin_url.as_str())
        .await
        .context("connect to postgres maintenance database")?;
    let database_name = format!("panda_baseline_verify_{}", Uuid::new_v4().simple());
    let create = sqlx::query(AssertSqlSafe(format!(
        "CREATE DATABASE \"{database_name}\" TEMPLATE template0"
    )))
    .execute(&admin_pool)
    .await
    .context("create disposable baseline database");
    if let Err(error) = create {
        admin_pool.close().await;
        return Err(error);
    }
    scratch_url.set_path(&format!("/{database_name}"));
    let outcome = async {
        sqlx::query(AssertSqlSafe(format!(
            "ALTER DATABASE \"{database_name}\" SET timescaledb.telemetry_level = 'off'"
        )))
        .execute(&admin_pool)
        .await
        .context("disable telemetry in disposable baseline database")?;
        match PgPoolOptions::new()
            .max_connections(2)
            .connect(scratch_url.as_str())
            .await
            .context("connect to disposable baseline database")
        {
            Err(error) => Err(error),
            Ok(scratch_pool) => {
                let describe = async {
                    migrations::apply_baseline_schema(&scratch_pool)
                        .await
                        .context("apply immutable baseline to disposable database")?;
                    let contract =
                        restore_compatibility::schema_contract_description(&scratch_pool)
                            .await
                            .context("describe disposable baseline")?;
                    Ok((contract_fingerprint(&contract)?, contract))
                }
                .await;
                scratch_pool.close().await;
                describe
            }
        }
    }
    .await;
    let cleanup = async {
        sqlx::query(AssertSqlSafe(format!(
            "ALTER DATABASE \"{database_name}\" ALLOW_CONNECTIONS false"
        )))
        .execute(&admin_pool)
        .await
        .context("close disposable baseline database to new connections")?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)"
        )))
        .execute(&admin_pool)
        .await
        .context("drop disposable baseline database")
    }
    .await;
    admin_pool.close().await;
    cleanup?;
    outcome
}

fn parse_args() -> anyhow::Result<(bool, Option<String>)> {
    use panda_api::db::maintenance::{command_options, OptionKind};
    let options = command_options(&[OptionKind::Yes, OptionKind::Backup],
        "Usage: cargo run --bin rebaseline_db -- [--yes --backup <path>]\n\
         Without --yes, creates a disposable database and proves the live schema matches the compiled baseline without writing to the live database.", "unknown argument")?;
    Ok((options.yes, options.backup))
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    use sqlx::Executor;

    #[tokio::test]
    #[ignore = "requires explicit isolated synthetic TimescaleDB with disposable role/database creation"]
    async fn scratch_verification_cleans_success_and_baseline_failure() {
        let configured = env::var("MAINTENANCE_DATABASE_URL").expect("explicit synthetic fixture");
        let mut url = Url::parse(&configured).unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "maintenance_bootstrap");
        url.set_path("/postgres");
        let admin = sqlx::PgPool::connect(url.as_str()).await.unwrap();
        let inventory = "SELECT datname::text FROM pg_database WHERE datname LIKE 'panda_baseline_verify_%' ORDER BY datname";
        let before: Vec<String> = sqlx::query_scalar(inventory)
            .fetch_all(&admin)
            .await
            .unwrap();
        for _ in 0..3 {
            let (fingerprint, contract) = scratch_baseline_contract(&configured).await.unwrap();
            assert_eq!(fingerprint.len(), 64);
            assert!(!contract.as_array().unwrap().is_empty());
        }
        let role = format!("maintenance_scratch_{}", Uuid::new_v4().simple());
        admin.execute(AssertSqlSafe(format!("CREATE ROLE {role} LOGIN CREATEDB NOSUPERUSER NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_scratch_only'"))).await.unwrap();
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_scratch_only")).unwrap();
        let error = scratch_baseline_contract(url.as_str()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("apply immutable baseline"),
            "{error:#}"
        );
        let after: Vec<String> = sqlx::query_scalar(inventory)
            .fetch_all(&admin)
            .await
            .unwrap();
        assert_eq!(
            after, before,
            "success and failure must leave no scratch database"
        );
        admin
            .execute(AssertSqlSafe(format!("DROP ROLE {role}")))
            .await
            .unwrap();
        admin.close().await;
    }
}

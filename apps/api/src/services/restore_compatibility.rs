use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, AssertSqlSafe, Executor, PgPool, Postgres};
use url::Url;
use uuid::Uuid;

use crate::{
    db::migrations::{self, LedgerValidationKind, MigrationIdentity, MIGRATION_CHAIN_ID, MIGRATOR},
    errors::AppError,
};

#[path = "restore_compatibility_recovery_stages.rs"]
mod recovery_stages;

pub const RECOVERY_FORMAT_V1: &str = "panda-recovery-v1";
pub const RECOVERY_FORMAT_V2: &str = "panda-recovery-v2";
pub const RECOVERY_FORMAT_V3: &str = "panda-recovery-v3";
pub const RESTORE_ENGINE_VERSION: u32 = 3;
pub const SCHEMA_CONTRACT_VERSION: &str = "panda-schema-contract-v1";
const REQUIRED_SCHEMA_RELATIONS: &[&str] = &[
    "panda.users",
    "panda.vehicles",
    "panda.dashboards",
    "panda.charts",
    "panda.trips",
    "panda.charge_sessions",
    "panda.backup_runs",
    "panda.backup_artifacts",
    "panda.backup_restore_requests",
    "timeseries.telemetry",
    "timeseries.telemetry_1min",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseProfile {
    pub postgres_major: Option<i32>,
    pub timescale_version: Option<String>,
    pub migration_version: i64,
    #[serde(default)]
    pub migration_ledger: Vec<MigrationIdentity>,
    #[serde(default)]
    pub migration_ledger_successful: bool,
    pub migration_chain_id: Option<String>,
    pub migration_catalog_digest: Option<String>,
    pub schema_contract_version: Option<String>,
    pub schema_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreBlockingCode {
    UnsupportedPackageFormat,
    UnsupportedMigrationChain,
    SourceLedgerInvalid,
    TargetMigrationDrift,
    MigrationChecksumMismatch,
    NewerSourceSchema,
    UnsupportedPostgresVersion,
    UnsupportedTimescaleVersion,
    SchemaContractMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreBlockingError {
    pub code: RestoreBlockingCode,
    pub message: String,
}

/// Reserved for future data-shape transitions that cannot be represented as a
/// normal forward SQL migration. The discarded pre-release chain has no
/// registered transforms.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreTransform {
    pub id: String,
    pub from_migration: i64,
    pub to_migration: i64,
    pub transactional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestorePlan {
    pub plan_id: String,
    pub engine_version: u32,
    pub package_checksum_sha256: String,
    pub package_format: String,
    pub compatible: bool,
    pub source: DatabaseProfile,
    pub target: DatabaseProfile,
    pub pending_migrations: Vec<i64>,
    pub transforms: Vec<RestoreTransform>,
    pub validation_checks: Vec<String>,
    pub warnings: Vec<String>,
    pub blocking_errors: Vec<RestoreBlockingError>,
    pub planned_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaContractReport {
    pub contract_version: String,
    pub schema_fingerprint: String,
    pub required_relations_present: bool,
    pub missing_relations: Vec<String>,
    pub telemetry_hypertable: bool,
    pub foreign_keys_validated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidatePreparationReport {
    pub source_schema: SchemaContractReport,
    pub target_schema: SchemaContractReport,
    pub applied_transforms: Vec<RestoreTransform>,
    pub migrations_applied: Vec<i64>,
    pub foreign_keys_validated: bool,
}

pub use super::restore_dump_inspection::{inspect_recovery_dump, DumpInspection};
pub use migrations::{compiled_migration_ledger, latest_migration_version};

pub async fn runtime_database_profile(pool: &PgPool) -> Result<DatabaseProfile, AppError> {
    let (postgres_major, timescale_version): (i32, Option<String>) = sqlx::query_as(
        "SELECT current_setting('server_version_num')::integer / 10000, (SELECT extversion FROM pg_extension WHERE extname = 'timescaledb')",
    ).fetch_one(pool).await?;
    let rows = sqlx::query_as::<_, (i64, String, bool, String)>(
        "SELECT version, description, success, encode(checksum, 'hex') FROM public._sqlx_migrations ORDER BY version",
    ).fetch_all(pool).await?;
    let (migration_ledger, migration_version, migration_ledger_successful) = rows.into_iter().fold(
        (Vec::new(), 0, true),
        |(mut ledger, _, successful), (version, description, success, checksum_sha384)| {
            ledger.push(MigrationIdentity {
                version,
                description,
                checksum_sha384,
            });
            (ledger, version, successful && success)
        },
    );
    Ok(DatabaseProfile {
        postgres_major: Some(postgres_major),
        timescale_version,
        migration_version,
        migration_ledger,
        migration_ledger_successful,
        migration_chain_id: Some(MIGRATION_CHAIN_ID.into()),
        migration_catalog_digest: Some(migrations::migration_catalog_digest()),
        schema_contract_version: Some(SCHEMA_CONTRACT_VERSION.into()),
        schema_fingerprint: Some(schema_fingerprint(pool).await?),
    })
}

/// Produce a canonical, data-free database contract. The contract intentionally
/// excludes SQLx bookkeeping, ownership, statistics, sequence positions, and
/// application rows.
pub async fn schema_contract_description(pool: &PgPool) -> Result<Value, AppError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SET LOCAL search_path = pg_catalog")
        .execute(&mut *transaction)
        .await?;
    let items: Vec<Value> = sqlx::query_scalar(
        r#"
        SELECT item
        FROM (
            SELECT 'column' AS category,
                   format('%I.%I.%s', namespace.nspname, relation.relname, attribute.attnum) AS identity,
                   jsonb_build_object(
                       'schema', namespace.nspname,
                       'relation', relation.relname,
                       'kind', relation.relkind::text,
                       'ordinal', attribute.attnum,
                       'name', attribute.attname,
                       'type', pg_catalog.format_type(attribute.atttypid, attribute.atttypmod),
                       'not_null', attribute.attnotnull,
                       'generated', attribute.attgenerated,
                       'identity', attribute.attidentity,
                       'default', pg_catalog.pg_get_expr(default_value.adbin, default_value.adrelid)
                   ) AS item
            FROM pg_catalog.pg_class relation
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
            JOIN pg_catalog.pg_attribute attribute ON attribute.attrelid = relation.oid
            LEFT JOIN pg_catalog.pg_attrdef default_value
              ON default_value.adrelid = relation.oid AND default_value.adnum = attribute.attnum
            WHERE namespace.nspname IN ('panda', 'timeseries')
              AND relation.relkind IN ('r', 'p', 'v', 'm')
              AND attribute.attnum > 0
              AND NOT attribute.attisdropped

            UNION ALL
            SELECT 'constraint',
                   format('%I.%I.%I', namespace.nspname, relation.relname, constraint_record.conname),
                   jsonb_build_object(
                       'schema', namespace.nspname,
                       'relation', relation.relname,
                       'name', constraint_record.conname,
                       'type', constraint_record.contype,
                       'validated', constraint_record.convalidated,
                       'definition', pg_catalog.pg_get_constraintdef(constraint_record.oid, true)
                   )
            FROM pg_catalog.pg_constraint constraint_record
            JOIN pg_catalog.pg_class relation ON relation.oid = constraint_record.conrelid
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname IN ('panda', 'timeseries')

            UNION ALL
            SELECT 'index',
                   format('%I.%I.%I', namespace.nspname, relation.relname, index_relation.relname),
                   jsonb_build_object(
                       'schema', namespace.nspname,
                       'relation', relation.relname,
                       'name', index_relation.relname,
                       'unique', index_record.indisunique,
                       'valid', index_record.indisvalid,
                       'definition', pg_catalog.pg_get_indexdef(index_record.indexrelid)
                   )
            FROM pg_catalog.pg_index index_record
            JOIN pg_catalog.pg_class relation ON relation.oid = index_record.indrelid
            JOIN pg_catalog.pg_class index_relation ON index_relation.oid = index_record.indexrelid
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname IN ('panda', 'timeseries')

            UNION ALL
            SELECT 'view',
                   format('%I.%I', namespace.nspname, relation.relname),
                   jsonb_build_object(
                       'schema', namespace.nspname,
                       'name', relation.relname,
                       'kind', relation.relkind::text,
                       'definition', regexp_replace(
                           regexp_replace(
                               pg_catalog.pg_get_viewdef(relation.oid, true),
                               '_materialized_hypertable_[0-9]+',
                               '_materialized_hypertable_ID',
                               'g'
                           ),
                           'cagg_watermark\([0-9]+\)',
                           'cagg_watermark(ID)',
                           'g'
                       )
                   )
            FROM pg_catalog.pg_class relation
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname IN ('panda', 'timeseries')
              AND relation.relkind IN ('v', 'm')

            UNION ALL
            SELECT 'function',
                   format('%I.%I(%s)', namespace.nspname, procedure.proname, pg_catalog.pg_get_function_identity_arguments(procedure.oid)),
                   jsonb_build_object(
                       'schema', namespace.nspname,
                       'name', procedure.proname,
                       'arguments', pg_catalog.pg_get_function_identity_arguments(procedure.oid),
                       'definition', pg_catalog.pg_get_functiondef(procedure.oid)
                   )
            FROM pg_catalog.pg_proc procedure
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = procedure.pronamespace
            WHERE namespace.nspname IN ('panda', 'timeseries')
              AND procedure.prokind IN ('f', 'p')
              AND NOT EXISTS (
                  SELECT 1
                  FROM pg_catalog.pg_depend dependency
                  WHERE dependency.classid = 'pg_proc'::regclass
                    AND dependency.objid = procedure.oid
                    AND dependency.deptype = 'e'
              )

            UNION ALL
            SELECT 'trigger',
                   format('%I.%I.%I', namespace.nspname, relation.relname, trigger_record.tgname),
                   jsonb_build_object(
                       'schema', namespace.nspname,
                       'relation', relation.relname,
                       'name', trigger_record.tgname,
                       'definition', pg_catalog.pg_get_triggerdef(trigger_record.oid, true)
                   )
            FROM pg_catalog.pg_trigger trigger_record
            JOIN pg_catalog.pg_class relation ON relation.oid = trigger_record.tgrelid
            JOIN pg_catalog.pg_namespace namespace ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname IN ('panda', 'timeseries')
              AND NOT trigger_record.tgisinternal

            UNION ALL
            SELECT 'extension', extension_record.extname,
                   jsonb_build_object('name', extension_record.extname)
            FROM pg_catalog.pg_extension extension_record
            WHERE extension_record.extname IN ('timescaledb', 'pgcrypto')

            UNION ALL
            SELECT 'hypertable',
                   format('%I.%I', hypertable_schema, hypertable_name),
                   jsonb_build_object(
                       'schema', hypertable_schema,
                       'name', hypertable_name,
                       'dimensions', num_dimensions,
                       'compression_enabled', compression_enabled
                   )
            FROM timescaledb_information.hypertables
            WHERE hypertable_schema IN ('panda', 'timeseries')

            UNION ALL
            SELECT 'continuous_aggregate',
                   format('%I.%I', view_schema, view_name),
                   jsonb_build_object(
                       'schema', view_schema,
                       'name', view_name,
                       'materialized_only', materialized_only,
                       'compression_enabled', compression_enabled
                   )
            FROM timescaledb_information.continuous_aggregates
            WHERE view_schema IN ('panda', 'timeseries')

            UNION ALL
            SELECT 'timescale_job',
                   format('%I.%I:%I.%I', proc_schema, proc_name, hypertable_schema, hypertable_name),
                   jsonb_build_object(
                       'proc_schema', proc_schema,
                       'proc_name', proc_name,
                       'schedule_interval', schedule_interval::text,
                       'scheduled', scheduled,
                       'hypertable_schema', hypertable_schema,
                       'hypertable_name', hypertable_name,
                       'config', config - 'mat_hypertable_id' - 'hypertable_id'
                   )
            FROM timescaledb_information.jobs
            WHERE hypertable_schema IN ('panda', 'timeseries')
        ) contract_items ORDER BY category, identity
        "#,
    )
    .fetch_all(&mut *transaction)
    .await?;
    transaction.rollback().await?;
    Ok(Value::Array(items))
}

pub async fn schema_fingerprint(pool: &PgPool) -> Result<String, AppError> {
    struct DigestOutput(Sha256);
    impl std::io::Write for DigestOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = DigestOutput(Sha256::new());
    let contract = serde_json::json!({"version":SCHEMA_CONTRACT_VERSION,"items":schema_contract_description(pool).await?});
    serde_json::to_writer(&mut output, &contract)
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
    Ok(hex::encode(output.0.finalize()))
}

pub async fn validate_schema_contract(pool: &PgPool) -> Result<SchemaContractReport, AppError> {
    validate_schema_contract_for_source(pool, true).await
}

async fn validate_schema_contract_for_source(
    pool: &PgPool,
    charts_required: bool,
) -> Result<SchemaContractReport, AppError> {
    let required: Vec<&str> = REQUIRED_SCHEMA_RELATIONS
        .iter()
        .copied()
        .filter(|relation| charts_required || *relation != "panda.charts")
        .collect();
    let missing_relations: Vec<String> = sqlx::query_scalar(
        "SELECT relation FROM unnest($1::text[]) WITH ORDINALITY AS required(relation, position) WHERE to_regclass(relation) IS NULL ORDER BY position",
    ).bind(required).fetch_all(pool).await?;
    let (telemetry_hypertable, foreign_keys_validated): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM timescaledb_information.hypertables WHERE hypertable_schema = 'timeseries' AND hypertable_name = 'telemetry'), (SELECT COALESCE(bool_and(convalidated), TRUE) FROM pg_constraint WHERE contype = 'f' AND connamespace IN ('panda'::regnamespace, 'timeseries'::regnamespace))",
    ).fetch_one(pool).await?;
    Ok(SchemaContractReport {
        contract_version: SCHEMA_CONTRACT_VERSION.into(),
        schema_fingerprint: schema_fingerprint(pool).await?,
        required_relations_present: missing_relations.is_empty(),
        missing_relations,
        telemetry_hypertable,
        foreign_keys_validated,
    })
}

/// Validate and migrate an isolated restore candidate. This function never
/// targets the live database.
pub async fn prepare_candidate_schema(
    pool: &PgPool,
    administrative_database_url: &str,
    manifest: &Value,
) -> anyhow::Result<CandidatePreparationReport> {
    recovery_stages::CandidateSource::admit(manifest)?
        .verify(pool)
        .await?
        .migrate(pool, administrative_database_url)
        .await?
        .verify(pool)
        .await
}

/// Recreate the redacted authentication singleton after a package restore.
///
/// Provider credentials and OIDC settings are intentionally excluded from
/// backups. A restored candidate must nevertheless have the migration's safe
/// defaults so startup and break-glass password recovery remain available.
/// Restored rows are never allowed to select the authentication provider.
async fn ensure_sanitized_authentication_settings<'c, E>(executor: E) -> Result<(), sqlx::Error>
where
    E: Executor<'c, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO panda.authentication_settings (id) VALUES (TRUE) ON CONFLICT (id) DO UPDATE SET oidc_enabled = FALSE, password_login_enabled = TRUE, issuer_url = NULL, public_base_url = NULL, client_id = NULL, client_secret_encrypted = NULL, button_label = DEFAULT, scopes = DEFAULT, token_auth_method = DEFAULT, auto_signup = FALSE, auto_link_verified_email = FALSE, allowed_email_domains = DEFAULT, required_claim_name = NULL, required_claim_value = NULL, last_validation_at = NULL, last_validation_fingerprint = NULL, updated_by = NULL, updated_at = now(), oidc_auto_login = FALSE",
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Create an empty sibling database and fingerprint only the immutable
/// baseline. This gives legacy-ledger restores a concrete structural proof
/// without ever applying or probing against the live target database.
async fn baseline_schema_fingerprint(candidate_database_url: &str) -> anyhow::Result<String> {
    baseline_schema_fingerprint_after_create(candidate_database_url, |_| async {}).await
}

async fn baseline_schema_fingerprint_after_create<F, W>(
    candidate_database_url: &str,
    after_create: F,
) -> anyhow::Result<String>
where
    F: FnOnce(String) -> W + Send + 'static,
    W: std::future::Future<Output = ()> + Send + 'static,
{
    let candidate_database_url = candidate_database_url.to_owned();
    let (waiter, mut cancelled) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let budget = std::time::Duration::from_secs(
            crate::config::RecoveryConfig::default().restore_deadline_seconds,
        );
        let cleanup_budget = std::time::Duration::from_secs(30);
        let mut admin_url = Url::parse(&candidate_database_url)?;
        admin_url.set_path("/postgres");
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(admin_url.as_str())
            .await?;
        let database_name = format!("panda_restore_baseline_{}", Uuid::new_v4().simple());
        let mut scratch_url = Url::parse(&candidate_database_url)?;
        scratch_url.set_path(&format!("/{database_name}"));
        let verification = tokio::time::timeout(budget, async {
            sqlx::query(AssertSqlSafe(format!(
                "CREATE DATABASE \"{database_name}\" TEMPLATE template0"
            )))
            .execute(&admin_pool)
            .await?;
            // A temporary schema proof must not start outbound telemetry jobs.
            sqlx::query(AssertSqlSafe(format!(
                "ALTER DATABASE \"{database_name}\" SET timescaledb.telemetry_level = 'off'"
            )))
            .execute(&admin_pool)
            .await?;
            after_create(database_name.clone()).await;
            let scratch_pool = PgPoolOptions::new()
                .max_connections(2)
                .connect(scratch_url.as_str())
                .await?;
            let result = async {
                migrations::apply_baseline_schema(&scratch_pool).await?;
                Ok::<_, anyhow::Error>(schema_fingerprint(&scratch_pool).await?)
            }
            .await;
            scratch_pool.close().await;
            result
        });
        let result = tokio::select! {
            biased;
            _ = &mut cancelled => Err(anyhow::anyhow!("recovery baseline verification cancelled")),
            result = verification => result.map_err(|_| anyhow::anyhow!("recovery baseline verification deadline exceeded")).and_then(|result| result),
        };
        let cleanup_deadline = tokio::time::Instant::now() + cleanup_budget;
        let cleanup = tokio::time::timeout_at(
            cleanup_deadline,
            drop_baseline_scratch_database(&admin_pool, &database_name),
        )
        .await
        .map_err(|_| anyhow::anyhow!("recovery baseline cleanup deadline exceeded"))
        .and_then(|result| result.map_err(anyhow::Error::from));
        let _ = tokio::time::timeout_at(cleanup_deadline, admin_pool.close()).await;
        if let Err(error) = &cleanup {
            tracing::error!(database = %database_name, error = %error, "recovery.baseline.cleanup_failed");
        }
        cleanup?;
        result
    });
    let result = task
        .await
        .map_err(|error| anyhow::anyhow!("recovery baseline task failed: {error}"))?;
    drop(waiter);
    result
}

async fn drop_baseline_scratch_database(
    admin_pool: &PgPool,
    database_name: &str,
) -> Result<(), sqlx::Error> {
    // Stop Timescale background workers reconnecting during forced removal.
    if let Err(error) = sqlx::query(AssertSqlSafe(format!(
        "ALTER DATABASE \"{database_name}\" ALLOW_CONNECTIONS false"
    )))
    .execute(admin_pool)
    .await
    {
        if error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref()
            == Some("3D000")
        {
            return Ok(());
        }
        return Err(error);
    }
    for attempt in 0..10 {
        match sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)"
        )))
        .execute(admin_pool)
        .await
        {
            Ok(_) => return Ok(()),
            Err(error) => {
                if attempt == 9
                    || error
                        .as_database_error()
                        .and_then(|error| error.code())
                        .as_deref()
                        != Some("55006")
                {
                    return Err(error);
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    unreachable!("the final drop attempt returns its result")
}

fn inject_candidate_fault(phase: &str) -> anyhow::Result<()> {
    if std::env::var("PANDA_RESTORE_FAULT_PHASE").as_deref() == Ok(phase) {
        anyhow::bail!("restore fault injection triggered at {phase}");
    }
    Ok(())
}

fn blocking_from_ledger_error(
    target: bool,
    error: migrations::LedgerValidationError,
) -> RestoreBlockingError {
    let (fallback, prefix) = match target {
        true => (
            RestoreBlockingCode::TargetMigrationDrift,
            "Target migration ledger is not compatible with this release",
        ),
        false => (
            RestoreBlockingCode::SourceLedgerInvalid,
            "Recovery package migration ledger is invalid",
        ),
    };
    RestoreBlockingError {
        code: match error.kind {
            LedgerValidationKind::Checksum => RestoreBlockingCode::MigrationChecksumMismatch,
            _ => fallback,
        },
        message: format!("{prefix}: {error}"),
    }
}

fn validate_source_schema_metadata(source: &DatabaseProfile) -> Result<(), RestoreBlockingError> {
    match (
        source.schema_contract_version.as_deref(),
        source.schema_fingerprint.as_ref(),
    ) {
        (Some(SCHEMA_CONTRACT_VERSION), Some(_)) => Ok(()),
        _ => Err(RestoreBlockingError {
            code: RestoreBlockingCode::SchemaContractMismatch,
            message: "The package is missing the supported versioned schema contract.".into(),
        }),
    }
}

pub async fn plan_restore(
    manifest: &Value,
    package_checksum_sha256: &str,
    target_pool: &PgPool,
    dump: Option<&DumpInspection>,
) -> Result<RestorePlan, AppError> {
    let format = manifest
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let source = source_profile_from_manifest(manifest)?;
    let target = runtime_database_profile(target_pool).await?;
    recovery_stages::PlanInput {
        format,
        source,
        target,
    }
    .build(package_checksum_sha256, dump)
}

fn source_profile_from_manifest(manifest: &Value) -> Result<DatabaseProfile, AppError> {
    let source = &manifest["source"];
    let string = |field: &str| source.get(field).and_then(Value::as_str).map(str::to_owned);
    let ledger = match source.get("migration_ledger") {
        None => Vec::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| AppError::Validation(format!("Invalid migration ledger: {error}")))?,
    };
    let postgres_major = source
        .get("postgres_major")
        .and_then(Value::as_i64)
        .and_then(|major| i32::try_from(major).ok());
    Ok(DatabaseProfile {
        postgres_major,
        timescale_version: string("timescale_version"),
        migration_version: source
            .get("migration_version")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        migration_ledger: ledger,
        migration_ledger_successful: true,
        migration_chain_id: string("migration_chain_id"),
        migration_catalog_digest: string("migration_catalog_digest"),
        schema_contract_version: string("schema_contract_version"),
        schema_fingerprint: string("schema_fingerprint"),
    })
}

fn timescale_versions_compatible(source: Option<&str>, target: Option<&str>) -> bool {
    match (
        source.and_then(parse_version_triplet),
        target.and_then(parse_version_triplet),
    ) {
        (Some(from), Some(to)) => from.0 == to.0 && from <= to,
        _ => false,
    }
}

fn parse_version_triplet(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0");
    let end = patch
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(patch.len());
    Some((major, minor, patch[..end].parse().ok()?))
}

#[cfg(test)]
fn validate_source_identity(source: &DatabaseProfile) -> Result<(), RestoreBlockingError> {
    validate_source_schema_metadata(source)?;
    migrations::validate_ledger_prefix(&source.migration_ledger)
        .map_err(|error| blocking_from_ledger_error(false, error))?;
    let declared_head = source.migration_version;
    let ledger_head = source
        .migration_ledger
        .last()
        .map(|entry| entry.version)
        .unwrap_or(0);
    if declared_head != ledger_head {
        return Err(RestoreBlockingError {
            code: RestoreBlockingCode::SourceLedgerInvalid,
            message: format!(
                "Recovery package declares migration {declared_head}, but its ledger stops at {ledger_head}."
            ),
        });
    }
    let source_catalog_digest = migrations::migration_ledger_digest(&source.migration_ledger);
    if source.migration_catalog_digest.as_deref() != Some(source_catalog_digest.as_str()) {
        return Err(RestoreBlockingError {
            code: RestoreBlockingCode::MigrationChecksumMismatch,
            message: "Recovery package migration catalog digest differs from this release catalog."
                .into(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "restore_compatibility_recovery_operations_tests.rs"]
pub(crate) mod recovery_operations_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[test]
    fn schema_contract_requires_chart_relation() {
        assert!(REQUIRED_SCHEMA_RELATIONS.contains(&"panda.charts"));
    }

    fn v3_manifest() -> Value {
        serde_json::json!({
            "format": RECOVERY_FORMAT_V3,
            "source": {
                "postgres_major": 18,
                "timescale_version": "2.28.3",
                "migration_version": latest_migration_version(),
                "migration_chain_id": MIGRATION_CHAIN_ID,
                "migration_ledger": compiled_migration_ledger(),
                "migration_catalog_digest": migrations::migration_catalog_digest(),
                "schema_contract_version": SCHEMA_CONTRACT_VERSION,
                "schema_fingerprint": "fingerprint"
            }
        })
    }

    #[test]
    fn timescale_compatibility_is_forward_only_within_a_major() {
        assert!(timescale_versions_compatible(
            Some("2.20.0"),
            Some("2.28.3")
        ));
        assert!(!timescale_versions_compatible(
            Some("2.29.0"),
            Some("2.28.3")
        ));
        assert!(!timescale_versions_compatible(None, Some("2.28.3")));
    }

    #[test]
    fn v3_source_profile_reads_chain_catalog_and_contract() {
        let profile = source_profile_from_manifest(&v3_manifest()).expect("source profile");
        assert_eq!(
            profile.migration_chain_id.as_deref(),
            Some(MIGRATION_CHAIN_ID)
        );
        assert_eq!(
            profile.migration_ledger.len(),
            compiled_migration_ledger().len()
        );
        assert_eq!(
            profile.schema_contract_version.as_deref(),
            Some(SCHEMA_CONTRACT_VERSION)
        );
        validate_source_identity(&profile).expect("v3 identity validates");
    }

    #[test]
    fn source_identity_rejects_checksum_drift_but_not_historical_chain_name() {
        let mut profile = source_profile_from_manifest(&v3_manifest()).expect("source profile");
        profile.migration_ledger[0]
            .checksum_sha384
            .replace_range(0..2, "00");
        assert_eq!(
            validate_source_identity(&profile)
                .expect_err("checksum drift")
                .code,
            RestoreBlockingCode::MigrationChecksumMismatch
        );

        let mut profile = source_profile_from_manifest(&v3_manifest()).expect("source profile");
        profile.migration_chain_id = Some("unknown-chain".into());
        validate_source_identity(&profile)
            .expect("chain name is verified by candidate schema proof");
    }

    #[test]
    fn v3_schema_metadata_allows_legacy_ledger_for_candidate_verification() {
        let mut profile = source_profile_from_manifest(&v3_manifest()).expect("source profile");
        profile.migration_chain_id = Some("pre-release-chain".into());
        profile.migration_ledger = vec![MigrationIdentity {
            version: 3,
            description: "historical state periods".into(),
            checksum_sha384: "00".repeat(48),
        }];
        profile.migration_version = 3;

        validate_source_schema_metadata(&profile).expect("schema metadata remains valid");
        assert!(validate_source_identity(&profile).is_err());
    }

    #[tokio::test]
    #[ignore = "requires a migrated PostgreSQL database"]
    async fn sanitized_authentication_settings_restore_defaults_without_overwriting() {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("database connection");

        let mut transaction = pool.begin().await.expect("transaction");
        sqlx::query("DELETE FROM panda.authentication_settings")
            .execute(&mut *transaction)
            .await
            .expect("delete singleton");
        ensure_sanitized_authentication_settings(&mut *transaction)
            .await
            .expect("recreate safe defaults");
        let defaults = sqlx::query(
            "SELECT oidc_enabled,password_login_enabled,client_secret_encrypted,scopes FROM panda.authentication_settings WHERE id=TRUE",
        )
        .fetch_one(&mut *transaction)
        .await
        .expect("safe defaults");
        assert!(!defaults.get::<bool, _>("oidc_enabled"));
        assert!(defaults.get::<bool, _>("password_login_enabled"));
        assert!(defaults
            .get::<Option<Vec<u8>>, _>("client_secret_encrypted")
            .is_none());
        assert_eq!(defaults.get::<String, _>("scopes"), "openid email profile");

        sqlx::query(
            "UPDATE panda.authentication_settings SET oidc_enabled=TRUE,password_login_enabled=FALSE,client_secret_encrypted=decode('c2VjcmV0','base64') WHERE id=TRUE",
        )
        .execute(&mut *transaction)
        .await
        .expect("seed existing settings");
        ensure_sanitized_authentication_settings(&mut *transaction)
            .await
            .expect("sanitize existing settings");
        let preserved = sqlx::query(
            "SELECT oidc_enabled,password_login_enabled,client_secret_encrypted FROM panda.authentication_settings WHERE id=TRUE",
        )
        .fetch_one(&mut *transaction)
        .await
        .expect("preserved settings");
        assert!(!preserved.get::<bool, _>("oidc_enabled"));
        assert!(preserved.get::<bool, _>("password_login_enabled"));
        assert_eq!(
            preserved.get::<Option<Vec<u8>>, _>("client_secret_encrypted"),
            None
        );
        transaction.rollback().await.expect("rollback test data");
    }
}

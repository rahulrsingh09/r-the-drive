use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration as StdDuration,
};

use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{pool::PoolConnection, FromRow, PgPool, Postgres};
use tar::{Archive, Builder};
use tokio::{fs, process::Command, time::MissedTickBehavior};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    config::{Config, RecoveryConfig},
    db::migrations,
    errors::AppError,
    ingestion::session_store::decrypt_json,
    services::{
        app_settings,
        restore_compatibility::{self, RECOVERY_FORMAT_V1, RECOVERY_FORMAT_V2, RECOVERY_FORMAT_V3},
        s3_backups::{self, S3Settings},
    },
};

#[path = "backups_recovery_admission.rs"]
mod recovery_admission;
use recovery_admission::RecoveryInventory;

#[path = "backups_recovery_stream.rs"]
mod recovery_stream;
use recovery_stream::{check_deadline, stream_error, CompressedInput, TarEnvelope};

#[path = "backups/calendar.rs"]
pub(crate) mod calendar;
#[path = "backups/publication.rs"]
mod publication;

const BACKUP_ADVISORY_LOCK_ID: i64 = 2_042_051_101;
const RECOVERY_MUTATION_ADVISORY_LOCK_ID: i64 = 2_042_051_102;
pub const RESTORE_CONFIRMATION_PHRASE: &str = "RESTORE";
static PG_DUMP_UNAVAILABLE_LOGGED: AtomicBool = AtomicBool::new(false);
const PG_DUMP_UNAVAILABLE_MESSAGE: &str = "pg_dump is not installed or not on PATH; install PostgreSQL client tools before creating a full recovery package";
const BACKUP_DRIVER_UNSUPPORTED_MESSAGE: &str =
    "manifest-only JSON backups are not valid recovery packages; use BACKUP_DRIVER=pg_dump";
const RECOVERY_PACKAGE_FORMAT: &str = RECOVERY_FORMAT_V3;

/// Hard safety ceilings for untrusted recovery packages. These are deliberately
/// independent of the source manifest: a manifest is part of the untrusted
/// archive and cannot be allowed to choose its own resource envelope.
pub const MAX_RECOVERY_PACKAGE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_RECOVERY_EXPANDED_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_RECOVERY_MEMBER_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_RECOVERY_MEMBERS: usize = 10_000;
pub const MAX_RECOVERY_COMPRESSION_RATIO: u64 = 200;

#[derive(Debug, Clone, Copy)]
pub enum BackupRunTrigger {
    Manual,
    Scheduled,
    PreRestore,
}

impl BackupRunTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::PreRestore => "pre_restore",
        }
    }
}

#[derive(Debug)]
pub struct BackupExecutionResult {
    pub run_id: Uuid,
    pub artifact_ids: Vec<Uuid>,
}

struct AdvisorySession {
    connection: Option<PoolConnection<Postgres>>,
    key: i64,
}

impl AdvisorySession {
    async fn try_acquire(pool: &PgPool, key: i64) -> Result<Option<Self>, AppError> {
        let mut session = Self {
            connection: Some(pool.acquire().await?),
            key,
        };
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(session.connection.as_deref_mut().expect("owned session"))
            .await?;
        if acquired {
            Ok(Some(session))
        } else {
            session.connection.take();
            Ok(None)
        }
    }

    async fn unlock(&mut self) -> Result<bool, sqlx::Error> {
        let Some(connection) = self.connection.as_deref_mut() else {
            return Ok(true);
        };
        let released: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
            .bind(self.key)
            .fetch_one(connection)
            .await?;
        if released {
            self.connection.take();
        }
        Ok(released)
    }
}

impl Drop for AdvisorySession {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            drop(connection.detach());
        }
    }
}

struct BackupLock(AdvisorySession);

impl BackupLock {
    async fn acquire(pool: &PgPool) -> Result<Self, AppError> {
        AdvisorySession::try_acquire(pool, BACKUP_ADVISORY_LOCK_ID)
            .await?
            .map(Self)
            .ok_or_else(|| AppError::Conflict("A backup job is already running.".into()))
    }

    async fn release(mut self) {
        match self.0.unlock().await {
            Ok(true) => {}
            Ok(false) => tracing::error!("backup.lock.release_not_owned"),
            Err(error) => tracing::error!(error = %error, "backup.lock.release_failed"),
        }
    }
}

/// Holds a physical PostgreSQL session until unlock is confirmed or it closes.
pub struct RecoveryMutationLock(AdvisorySession);

impl RecoveryMutationLock {
    pub async fn release(mut self) {
        let _ = self.0.unlock().await;
    }

    async fn run<T>(
        self,
        operation: impl std::future::Future<Output = Result<T, AppError>>,
    ) -> Result<T, AppError> {
        let result = operation.await;
        self.release().await;
        result
    }
}

pub async fn acquire_recovery_mutation_lock(
    pool: &PgPool,
) -> Result<RecoveryMutationLock, AppError> {
    AdvisorySession::try_acquire(pool, RECOVERY_MUTATION_ADVISORY_LOCK_ID)
        .await?
        .map(RecoveryMutationLock)
        .ok_or_else(|| {
            AppError::RecoveryConflict(
                "Another recovery upload or restore is already running.".into(),
            )
        })
}

/// Once admitted, bounded recovery work owns its lifetime independently of
/// the HTTP waiter. Dropping that waiter must not release admission while an
/// uncancellable spawn_blocking validator/extractor is still running.
pub async fn run_recovery_operation<F, T>(pool: PgPool, operation: F) -> Result<T, AppError>
where
    F: std::future::Future<Output = Result<T, AppError>> + Send + 'static,
    T: Send + 'static,
{
    let worker = tokio::spawn(async move {
        acquire_recovery_mutation_lock(&pool)
            .await?
            .run(operation)
            .await
    });
    worker.await.map_err(|error| {
        AppError::Internal(anyhow::anyhow!("recovery operation failed: {error}"))
    })?
}

#[derive(Debug, Clone)]
pub struct ValidatedRecoveryPackage {
    pub manifest: serde_json::Value,
    pub checksum_sha256: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupRuntimeReadiness {
    pub pg_dump_available: bool,
    pub run_now_allowed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, FromRow)]
struct BackupSettingsRow {
    enabled: bool,
    frequency: String,
    run_at: NaiveTime,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    retention_count: i32,
    prefix: String,
    local_enabled: bool,
    s3_enabled: bool,
    endpoint: String,
    region: Option<String>,
    bucket: String,
    access_key: Option<String>,
    secret_key_encrypted: Option<Vec<u8>>,
}

#[derive(Debug)]
struct BackupSettings {
    enabled: bool,
    frequency: BackupFrequency,
    run_at: NaiveTime,
    timezone: Tz,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    retention_count: i32,
    prefix: String,
    local_enabled: bool,
    s3_enabled: bool,
    s3: Option<S3Settings>,
}

#[derive(Debug, Clone, Copy)]
enum BackupFrequency {
    Daily,
    Weekly,
    Monthly,
}

impl TryFrom<&str> for BackupFrequency {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            "monthly" => Ok(Self::Monthly),
            _ => Err(AppError::Validation(
                "frequency must be daily, weekly, or monthly".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupDriver {
    PgDump,
}

impl BackupDriver {
    fn from_config(value: &str) -> Self {
        let _ = value;
        Self::PgDump
    }

    fn label(self) -> &'static str {
        "recovery_package"
    }
}

#[derive(Debug, FromRow)]
struct PrunableArtifactRow {
    id: Uuid,
    run_id: Uuid,
    storage_path: String,
}

pub fn start_backup_scheduler(pool: PgPool, config: Config) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let cadence = StdDuration::from_secs(config.backup_poll_interval_seconds.max(30));
        let mut ticks = tokio::time::interval(cadence);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut failure = None;
        loop {
            ticks.tick().await;
            match maybe_run_scheduled_backup(&pool, &config).await {
                Ok(_) => failure = None,
                Err(error) => match should_log_scheduler_failure(&mut failure, &error) {
                    true => tracing::error!(error = ?error, "backup.scheduler.tick_failed"),
                    false => tracing::debug!(error = ?error, "backup.scheduler.tick_failed_repeat"),
                },
            }
        }
    })
}

pub async fn run_backup_now(
    pool: &PgPool,
    config: &Config,
    requested_by: Option<Uuid>,
    trigger: BackupRunTrigger,
) -> Result<BackupExecutionResult, AppError> {
    ensure_backup_runtime_available(config).await?;
    BackupLock::acquire(pool)
        .await?
        .run(run_backup_inner(pool, config, requested_by, trigger))
        .await
}

/// Starts a backup without holding the HTTP request open. The advisory lock is
/// owned by the detached task so a second request still receives a fast 409
/// while the durable run record exposes the actual progress through the
/// overview endpoint.
pub async fn start_backup_now(
    pool: &PgPool,
    config: &Config,
    requested_by: Option<Uuid>,
    trigger: BackupRunTrigger,
) -> Result<Uuid, AppError> {
    ensure_backup_runtime_available(config).await?;
    let lock = BackupLock::acquire(pool).await?;
    let run_id = insert_running_backup(pool, requested_by, trigger).await?;
    let owned = (pool.clone(), config.clone());
    tokio::spawn(lock.run(async move {
        let result = run_backup_inner_for_run(&owned.0, &owned.1, run_id, trigger).await;
        if let Err(error) = &result {
            tracing::error!(run_id = %run_id, error = ?error, "backup.async_job_failed");
        }
        result
    }));
    Ok(run_id)
}

pub async fn create_restore_request(
    pool: &PgPool,
    artifact_id: Uuid,
    requested_by: Uuid,
    confirmation_phrase: &str,
    notes: Option<String>,
) -> Result<Uuid, AppError> {
    let confirmation = confirmation_phrase.trim();
    if confirmation != RESTORE_CONFIRMATION_PHRASE {
        return Err(AppError::Validation(format!(
            "Type {RESTORE_CONFIRMATION_PHRASE} to request a restore"
        )));
    }
    let (exists, pending): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM panda.backup_artifacts WHERE id = $1), EXISTS(SELECT 1 FROM panda.backup_restore_requests WHERE artifact_id = $1 AND status IN ('pending', 'approved', 'running'))",
    ).bind(artifact_id).fetch_one(pool).await?;
    match (exists, pending) {
        (false, _) => return Err(AppError::NotFound),
        (true, true) => {
            return Err(AppError::Conflict(
                "A restore request for this artifact is already pending.".into(),
            ))
        }
        _ => {}
    }
    let notes = notes
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Ok(sqlx::query_scalar(
        "INSERT INTO panda.backup_restore_requests (artifact_id, requested_by, status, confirmation_phrase, notes, updated_at) VALUES ($1, $2, 'pending', $3, $4, now()) RETURNING id",
    ).bind(artifact_id).bind(requested_by).bind(confirmation).bind(notes).fetch_one(pool).await?)
}

async fn run_backup_inner(
    pool: &PgPool,
    config: &Config,
    requested_by: Option<Uuid>,
    trigger: BackupRunTrigger,
) -> Result<BackupExecutionResult, AppError> {
    let run_id = insert_running_backup(pool, requested_by, trigger).await?;
    run_backup_inner_for_run(pool, config, run_id, trigger).await
}

async fn insert_running_backup(
    pool: &PgPool,
    requested_by: Option<Uuid>,
    trigger: BackupRunTrigger,
) -> Result<Uuid, AppError> {
    sqlx::query_scalar(
        r#"
        INSERT INTO panda.backup_runs (
            trigger, status, phase, progress_percent, requested_by, started_at, updated_at
        )
        VALUES ($1, 'running', 'queued', 0, $2, now(), now())
        RETURNING id
        "#,
    )
    .bind(trigger.as_str())
    .bind(requested_by)
    .fetch_one(pool)
    .await
    .map_err(AppError::from)
}

async fn update_backup_progress(
    pool: &PgPool,
    run_id: Uuid,
    phase: &str,
    progress_percent: i16,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE panda.backup_runs
        SET phase = $2, progress_percent = $3, updated_at = now()
        WHERE id = $1 AND status = 'running'
        "#,
    )
    .bind(run_id)
    .bind(phase)
    .bind(progress_percent)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AppError::from)
}

async fn run_backup_inner_for_run(
    pool: &PgPool,
    config: &Config,
    run_id: Uuid,
    trigger: BackupRunTrigger,
) -> Result<BackupExecutionResult, AppError> {
    let settings = match load_settings(pool, config).await {
        Ok(settings) => settings,
        Err(error) => {
            let _ = mark_backup_run_failed(pool, run_id, &error).await;
            return Err(error);
        }
    };
    let created_at = Utc::now();
    let path = build_artifact_path(config, &settings.prefix, created_at, run_id)?;
    let result =
        publication::execute(pool, config, &settings, run_id, trigger, created_at, &path).await;
    if let Err(error) = &result {
        publication::fail(pool, run_id, &path, error).await?;
    }
    result
}

async fn mark_backup_run_failed(
    pool: &PgPool,
    run_id: Uuid,
    error: &AppError,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE panda.backup_runs
        SET status = 'failed', phase = 'failed', completed_at = now(), updated_at = now(), error_message = $2
        WHERE id = $1 AND status = 'running'
        "#,
    )
    .bind(run_id)
    .bind(error.to_string())
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(AppError::from)
}

async fn maybe_run_scheduled_backup(
    pool: &PgPool,
    config: &Config,
) -> Result<Option<Uuid>, AppError> {
    let readiness = runtime_readiness(config).await;
    if config.backup_driver.trim().eq_ignore_ascii_case("json") {
        return Ok(None);
    }
    if !readiness.pg_dump_available {
        if !PG_DUMP_UNAVAILABLE_LOGGED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "backup.scheduler.skipped_pg_dump_unavailable: {}",
                PG_DUMP_UNAVAILABLE_MESSAGE
            );
        }
        return Ok(None);
    }
    PG_DUMP_UNAVAILABLE_LOGGED.store(false, Ordering::Relaxed);
    let settings = load_settings(pool, config).await?;
    if !settings.enabled {
        return Ok(None);
    }
    let due = compute_due_run_at(&settings, Utc::now())?;
    let covered: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.backup_runs WHERE trigger IN ('manual', 'scheduled') AND status IN ('pending', 'running', 'succeeded', 'failed') AND created_at >= $1)",
    ).bind(due).fetch_one(pool).await?;
    match covered {
        true => Ok(None),
        false => start_backup_now(pool, config, None, BackupRunTrigger::Scheduled)
            .await
            .map(Some),
    }
}

async fn load_settings(pool: &PgPool, config: &Config) -> Result<BackupSettings, AppError> {
    let timezone = app_settings::load_app_timezone(pool).await?;
    let row = sqlx::query_as::<_, BackupSettingsRow>(
        "SELECT enabled, frequency, run_at, day_of_week, day_of_month, retention_count, prefix, local_enabled, s3_enabled, endpoint, region, bucket, access_key, secret_key_encrypted FROM panda.backup_settings WHERE id = TRUE",
    ).fetch_optional(pool).await?.unwrap_or_else(|| BackupSettingsRow {
        enabled: false, frequency: "weekly".into(), run_at: NaiveTime::from_hms_opt(3, 0, 0).expect("valid default time"),
        day_of_week: Some(0), day_of_month: Some(1), retention_count: 8, prefix: "panda".into(),
        local_enabled: true, s3_enabled: false, endpoint: String::new(), region: None, bucket: String::new(),
        access_key: None, secret_key_encrypted: None,
    });
    let s3 = resolve_s3_settings(&row, config)?;
    let frequency = BackupFrequency::try_from(row.frequency.as_str())?;
    let prefix = normalize_prefix(&row.prefix)?;
    Ok(BackupSettings {
        enabled: row.enabled,
        frequency,
        run_at: row.run_at,
        timezone,
        day_of_week: row.day_of_week,
        day_of_month: row.day_of_month,
        retention_count: row.retention_count.max(1),
        prefix,
        local_enabled: row.local_enabled,
        s3_enabled: row.s3_enabled,
        s3,
    })
}

fn resolve_s3_settings(
    row: &BackupSettingsRow,
    config: &Config,
) -> Result<Option<S3Settings>, AppError> {
    if !row.s3_enabled {
        return Ok(None);
    }
    let saved_secret = row
        .secret_key_encrypted
        .as_deref()
        .map(|ciphertext| {
            let key = config.age_encryption_key.as_deref().ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("age encryption key is unavailable"))
            })?;
            let identity = key
                .parse::<age::x25519::Identity>()
                .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid age encryption key")))?;
            decrypt_json::<String>(ciphertext, &identity).map_err(AppError::Internal)
        })
        .transpose()?;
    let credentials = [
        row.access_key.clone().zip(saved_secret),
        config
            .s3_access_key
            .clone()
            .zip(config.s3_secret_key.clone()),
    ]
    .into_iter()
    .flatten()
    .next()
    .ok_or_else(|| AppError::Validation("S3 access key and secret key are required".into()))?;
    if row.bucket.trim().is_empty() {
        return Err(AppError::Validation("S3 bucket is required".into()));
    }
    let endpoint = match row.endpoint.trim() {
        "" => config.s3_endpoint.clone().unwrap_or_default(),
        _ => row.endpoint.clone(),
    };
    Ok(Some(S3Settings {
        endpoint,
        region: row
            .region
            .as_deref()
            .filter(|region| !region.trim().is_empty())
            .unwrap_or("us-east-1")
            .to_owned(),
        bucket: row.bucket.clone(),
        prefix: row.prefix.clone(),
        access_key: credentials.0,
        secret_key: credentials.1,
        policy: crate::services::s3_transport::S3Policy::from_config(config)?,
    }))
}

pub async fn configured_s3_settings(
    pool: &PgPool,
    config: &Config,
) -> Result<Option<S3Settings>, AppError> {
    Ok(load_settings(pool, config).await?.s3)
}

fn build_artifact_path(
    config: &Config,
    prefix: &str,
    created_at: DateTime<Utc>,
    run_id: Uuid,
) -> Result<PathBuf, AppError> {
    let prefix = crate::services::artifact_files::validate_prefix(prefix)?;
    let suffix = format!(
        "{}/{}/backup-{}-{}.rma.tar.gz",
        created_at.format("%Y"),
        created_at.format("%m"),
        created_at.format("%Y%m%dT%H%M%SZ"),
        run_id.simple()
    );
    Ok(Path::new(&config.backup_artifact_dir)
        .join(prefix)
        .join(suffix))
}

async fn execute_recovery_package(
    pool: &PgPool,
    config: &Config,
    run_id: Uuid,
    artifact_path: &Path,
    trigger: BackupRunTrigger,
    created_at: DateTime<Utc>,
) -> Result<serde_json::Value, AppError> {
    enum Stage {
        Dump,
        Snapshot,
        Package,
        Validate,
    }
    let workspace = crate::services::recovery_workspace::RecoveryWorkspace::new(
        Path::new(&config.backup_artifact_dir),
        &config.recovery,
    )?;
    let [dump, settings, history] = [
        "database.dump",
        "backup-settings.json",
        "operational-history.json",
    ]
    .map(|name| workspace.path().join(name));
    let cache = Path::new(&config.vehicle_image_cache_dir);
    let mut manifest = serde_json::Value::Null;
    for (stage, phase, percent) in [
        (Stage::Dump, "dumping", 10),
        (Stage::Snapshot, "snapshotting", 45),
        (Stage::Package, "packaging", 65),
        (Stage::Validate, "validating", 90),
    ] {
        update_backup_progress(pool, run_id, phase, percent).await?;
        match stage {
            Stage::Dump => execute_pg_dump(config, &dump).await?,
            Stage::Snapshot => {
                write_sanitized_backup_settings(pool, &settings).await?;
                write_operational_history(pool, &history).await?;
            }
            Stage::Package => {
                manifest = build_recovery_manifest(
                    pool, &dump, &settings, &history, cache, trigger, created_at,
                )
                .await?;
                let bytes = serde_json::to_vec_pretty(&manifest)
                    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
                let output = crate::services::artifact_files::create(
                    Path::new(&config.backup_artifact_dir),
                    artifact_path,
                )?;
                write_recovery_archive_to_file(output, &dump, &settings, &history, cache, &bytes)
                    .await?;
            }
            Stage::Validate => {
                validate_recovery_package_with_limits(artifact_path, &config.recovery).await?;
            }
        }
    }
    Ok(manifest)
}

pub async fn validate_recovery_package(
    package_path: &Path,
) -> Result<ValidatedRecoveryPackage, AppError> {
    validate_recovery_package_with_limits(package_path, &RecoveryConfig::default()).await
}

pub async fn validate_recovery_package_with_limits(
    package_path: &Path,
    limits: &RecoveryConfig,
) -> Result<ValidatedRecoveryPackage, AppError> {
    let package_path = package_path.to_owned();
    let limits = limits.clone();
    tokio::task::spawn_blocking(move || {
        validate_recovery_package_sync_with_limits(&package_path, &limits)
    })
    .await
    .map_err(|error| {
        AppError::Internal(anyhow::anyhow!("package validation task failed: {error}"))
    })?
    .map_err(classify_recovery_error)
}

pub async fn validate_open_recovery_package(
    file: File,
    limits: &RecoveryConfig,
) -> Result<ValidatedRecoveryPackage, AppError> {
    let limits = limits.clone();
    tokio::task::spawn_blocking(move || validate_recovery_file_sync(file, &limits))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?
        .map_err(classify_recovery_error)
}

pub async fn extract_recovery_package(
    package_path: &Path,
    destination: &Path,
) -> Result<(), AppError> {
    extract_recovery_package_with_limits(package_path, destination, &RecoveryConfig::default())
        .await
}

pub async fn extract_recovery_package_with_limits(
    package_path: &Path,
    destination: &Path,
    limits: &RecoveryConfig,
) -> Result<(), AppError> {
    let package_path = package_path.to_owned();
    let destination = destination.to_owned();
    let limits = limits.clone();
    tokio::task::spawn_blocking(move || {
        let root = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let workspace = crate::services::recovery_workspace::RecoveryWorkspace::new(root, &limits)?;
        let frozen = workspace.snapshot(&package_path, &limits)?;
        crate::services::recovery_extract::extract(frozen.path(), &destination, &limits)
    })
    .await
    .map_err(|error| {
        AppError::Internal(anyhow::anyhow!("package extraction task failed: {error}"))
    })?
    .map_err(classify_recovery_error)
}

pub(crate) fn metadata_limit(name: &str, limits: &RecoveryConfig) -> u64 {
    let ceiling = [
        ("manifest.json", limits.max_manifest_bytes),
        ("backup-settings.json", limits.max_settings_bytes),
        ("operational-history.json", limits.max_history_bytes),
    ]
    .into_iter()
    .find_map(|(member, maximum)| (name == member).then_some(maximum));
    ceiling
        .unwrap_or(limits.max_member_bytes)
        .min(limits.max_member_bytes)
}

pub(crate) fn classify_recovery_error(error: AppError) -> AppError {
    match error {
        AppError::Validation(message) => match [
            "exceeds",
            "too many archive members",
            "expanded size overflowed",
            "size overflowed",
        ]
        .iter()
        .any(|pattern| message.contains(pattern))
        {
            true => AppError::RecoveryTooLarge(message),
            false => AppError::RecoveryInvalid(message),
        },
        AppError::Io(error) => AppError::RecoveryInvalid(format!(
            "Recovery package could not be read safely: {error}"
        )),
        AppError::Conflict(message) => AppError::RecoveryConflict(message),
        other => other,
    }
}

/// Mark jobs from a previous API process as failed before new work is allowed
/// to start. Backup workers are intentionally detached from HTTP requests, so
/// a process restart cannot resume an in-memory task safely.
pub async fn reconcile_running_runs(pool: &PgPool) -> Result<u64, AppError> {
    let result = sqlx::query(
        r#"
        UPDATE panda.backup_runs
        SET status = 'failed',
            phase = 'failed',
            completed_at = COALESCE(completed_at, now()),
            updated_at = now(),
            error_message = COALESCE(NULLIF(error_message, ''), 'Backup worker stopped before completion.')
        WHERE status = 'running'
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Rebuild missing local catalog rows from the persistent backup directory.
pub use crate::services::backup_catalog::reconcile_local_catalog;

pub(crate) fn validate_recovery_package_sync_with_limits(
    package_path: &Path,
    limits: &RecoveryConfig,
) -> Result<ValidatedRecoveryPackage, AppError> {
    validate_recovery_file_sync(File::open(package_path)?, limits)
}

fn validate_recovery_file_sync(
    mut file: File,
    limits: &RecoveryConfig,
) -> Result<ValidatedRecoveryPackage, AppError> {
    use std::io::{Seek, SeekFrom};
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(AppError::Validation(
            "Recovery package is not a regular file.".into(),
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    validate_recovery_reader(file, metadata.len(), limits)
}

fn validate_recovery_reader<R: Read>(
    source: R,
    compressed_bytes: u64,
    limits: &RecoveryConfig,
) -> Result<ValidatedRecoveryPackage, AppError> {
    let started = std::time::Instant::now();
    check_deadline(started, limits)?;
    if compressed_bytes > limits.max_upload_bytes {
        return Err(AppError::Validation(format!(
            "Recovery package exceeds the {} GiB compressed size limit.",
            limits.max_upload_bytes / 1024 / 1024 / 1024
        )));
    }
    let source = CompressedInput::new(source, limits, started);
    let decoder = GzDecoder::new(source);
    let mut archive = Archive::new(TarEnvelope::new(decoder, limits, started));
    let inventory = RecoveryInventory::read(&mut archive, limits, compressed_bytes, started)?;
    let source = archive.into_inner().finish()?.into_inner();
    let manifest = inventory.manifest()?;
    validate_manifest_contract(&manifest, &inventory)?;
    let validated = source.finish(manifest)?;
    if validated.size_bytes as u64 != compressed_bytes {
        return Err(AppError::Validation(
            "Recovery package size changed during validation.".into(),
        ));
    }
    Ok(validated)
}

fn validate_manifest_contract(
    manifest: &serde_json::Value,
    inventory: &RecoveryInventory,
) -> Result<(), AppError> {
    let format = manifest
        .get("format")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let format_version = manifest
        .get("format_version")
        .and_then(|value| value.as_u64());
    let supported_format = matches!(
        (format, format_version),
        (RECOVERY_FORMAT_V1, Some(1))
            | (RECOVERY_FORMAT_V2, Some(2))
            | (RECOVERY_FORMAT_V3, Some(3))
    );
    if !supported_format {
        return Err(AppError::Validation(
            "Unsupported recovery package format.".into(),
        ));
    }
    if matches!(format, RECOVERY_FORMAT_V2 | RECOVERY_FORMAT_V3) {
        for pointer in [
            "/source/postgres_major",
            "/source/migration_version",
            "/source/migration_ledger",
            "/source/schema_fingerprint",
            "/restore/engine_version",
        ] {
            if manifest.pointer(pointer).is_none() {
                return Err(AppError::Validation(format!(
                    "Recovery manifest is missing {pointer}."
                )));
            }
        }
        let engine_version = manifest
            .pointer("/restore/engine_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        if engine_version == 0
            || engine_version > u64::from(restore_compatibility::RESTORE_ENGINE_VERSION)
        {
            return Err(AppError::Validation(format!(
                "Recovery package requires unsupported restore engine version {engine_version}."
            )));
        }
        validate_migration_ledger_shape(manifest)?;
        if format == RECOVERY_FORMAT_V3 {
            for pointer in [
                "/source/migration_chain_id",
                "/source/migration_catalog_digest",
                "/source/schema_contract_version",
                "/source/app_version",
            ] {
                if manifest
                    .pointer(pointer)
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(AppError::Validation(format!(
                        "Recovery v3 manifest is missing {pointer}."
                    )));
                }
            }
        }
        for component in [
            "database",
            "backup_settings",
            "operational_history",
            "vehicle_image_cache",
        ] {
            let value = manifest
                .pointer(&format!("/components/{component}"))
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "Recovery manifest is missing component {component}."
                    ))
                })?;
            if value.get("version").and_then(serde_json::Value::as_u64) != Some(1)
                || value
                    .get("restore_policy")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(str::is_empty)
                || !value
                    .get("redactions")
                    .is_some_and(serde_json::Value::is_array)
            {
                return Err(AppError::Validation(format!(
                    "Recovery v2 component {component} has an invalid contract."
                )));
            }
        }
    }

    verify_manifest_component(manifest, inventory, "database", "database.dump")?;
    if !inventory.custom_database_dump() {
        return Err(AppError::Validation(
            "Recovery package database.dump is not a PostgreSQL custom-format dump.".into(),
        ));
    }
    verify_manifest_component(
        manifest,
        inventory,
        "backup_settings",
        "backup-settings.json",
    )?;
    if matches!(format, RECOVERY_FORMAT_V2 | RECOVERY_FORMAT_V3) {
        verify_manifest_component(
            manifest,
            inventory,
            "operational_history",
            "operational-history.json",
        )?;
    }
    if let Some(files) = manifest
        .pointer("/components/vehicle_image_cache/files")
        .and_then(|value| value.as_array())
    {
        for file in files {
            let path = file
                .get("path")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    AppError::Validation("Artwork manifest entry is missing its path.".into())
                })?;
            let expected = file
                .get("sha256")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    AppError::Validation("Artwork manifest entry is missing its checksum.".into())
                })?;
            if inventory.checksum(path) != Some(expected) {
                return Err(AppError::Validation(format!(
                    "Recovery package checksum mismatch for {path}."
                )));
            }
            let expected_size = file
                .get("size_bytes")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    AppError::Validation("Artwork manifest entry is missing its size.".into())
                })?;
            if inventory.size(path) != Some(expected_size) {
                return Err(AppError::Validation(format!(
                    "Recovery package size mismatch for {path}."
                )));
            }
        }
        if matches!(format, RECOVERY_FORMAT_V2 | RECOVERY_FORMAT_V3) {
            validate_v2_artwork_component(manifest, files)?;
        }
    }

    Ok(())
}

fn verify_manifest_component(
    manifest: &serde_json::Value,
    inventory: &RecoveryInventory,
    component: &str,
    expected_path: &str,
) -> Result<(), AppError> {
    let declaration = manifest
        .pointer(&format!("/components/{component}"))
        .ok_or_else(|| {
            AppError::Validation(format!("Recovery manifest is missing {component}."))
        })?;
    let path = declaration
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(expected_path);
    let checksum = declaration
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "Recovery manifest is missing the checksum for {declaration}."
            ))
        })?;
    match (path == expected_path, inventory.checksum(path)) {
        (true, Some(actual)) if actual == checksum => {}
        _ => {
            return Err(AppError::Validation(format!(
                "Recovery package checksum mismatch for {expected_path}."
            )))
        }
    }
    match declaration
        .get("size_bytes")
        .and_then(serde_json::Value::as_u64)
    {
        None => Err(AppError::Validation(format!(
            "Recovery manifest is missing the size for {expected_path}."
        ))),
        Some(size) if inventory.size(path) == Some(size) => Ok(()),
        _ => Err(AppError::Validation(format!(
            "Recovery package size mismatch for {expected_path}."
        ))),
    }
}

fn validate_migration_ledger_shape(manifest: &serde_json::Value) -> Result<(), AppError> {
    let declared = manifest
        .pointer("/source/migration_version")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    let ledger = manifest
        .pointer("/source/migration_ledger")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AppError::Validation("Recovery migration ledger is invalid.".into()))?;
    if declared < 1 || ledger.is_empty() {
        return Err(AppError::Validation(
            "Recovery migration ledger is incomplete.".into(),
        ));
    }
    let last = ledger.iter().try_fold(0, |previous, row| {
        let version = row
            .get("version")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default();
        match (
            version > previous,
            row.get("checksum_sha384")
                .and_then(serde_json::Value::as_str),
        ) {
            (true, Some(checksum)) if !checksum.is_empty() => Ok(version),
            _ => Err(AppError::Validation(format!(
                "Recovery migration ledger entry {version} is invalid."
            ))),
        }
    })?;
    if last != declared {
        return Err(AppError::Validation(
            "Recovery migration ledger does not end at the declared migration version.".into(),
        ));
    }
    Ok(())
}

fn validate_v2_artwork_component(
    manifest: &serde_json::Value,
    files: &[serde_json::Value],
) -> Result<(), AppError> {
    let component = manifest
        .pointer("/components/vehicle_image_cache")
        .ok_or_else(|| AppError::Validation("Recovery v2 artwork component is missing.".into()))?;
    let expected = component
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AppError::Validation("Recovery v2 artwork component checksum is missing.".into())
        })?;
    let bytes =
        serde_json::to_vec(files).map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
    let size = files
        .iter()
        .filter_map(|file| file.get("size_bytes").and_then(serde_json::Value::as_u64))
        .sum::<u64>();
    let numbers_match = [("size_bytes", size), ("file_count", files.len() as u64)]
        .iter()
        .all(|(field, actual)| {
            component.get(*field).and_then(serde_json::Value::as_u64) == Some(*actual)
        });
    if numbers_match && expected == hex::encode(Sha256::digest(bytes)) {
        return Ok(());
    }
    Err(AppError::Validation(
        "Recovery v2 artwork component summary is inconsistent.".into(),
    ))
}

async fn execute_pg_dump(config: &Config, dump_path: &Path) -> Result<(), AppError> {
    let address = url::Url::parse(&config.database_url)
        .map_err(|error| AppError::Internal(anyhow::anyhow!("invalid DATABASE_URL: {error}")))?;
    let executable = resolve_pg_dump_executable()
        .await
        .ok_or_else(|| AppError::DependencyUnavailable(PG_DUMP_UNAVAILABLE_MESSAGE.into()))?;
    let mut command = Command::new(executable);
    command.kill_on_drop(true);
    let port = address.port().map(|port| port.to_string());
    let database = address.path().trim_start_matches('/');
    for (flag, value) in [
        ("host", address.host_str()),
        ("port", port.as_deref()),
        (
            "username",
            (!address.username().is_empty()).then_some(address.username()),
        ),
        ("dbname", (!database.is_empty()).then_some(database)),
    ] {
        if let Some(value) = value {
            command.arg(format!("--{flag}={value}"));
        }
    }
    if let Some(password) = address.password() {
        command.env("PGPASSWORD", password);
    }
    command.args(["--format=custom", "--no-owner", "--no-privileges"]);
    for table in crate::services::restore_policy::PROTECTED_TABLES {
        command.arg(format!("--exclude-table-data=panda.{table}"));
    }
    let output = command
        .arg(format!("--file={}", dump_path.display()))
        .output()
        .await
        .map_err(|error| AppError::Internal(anyhow::anyhow!("failed to spawn pg_dump: {error}")))?;
    match (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).trim(),
    ) {
        (true, _) => Ok(()),
        (false, "") => Err(AppError::Internal(anyhow::anyhow!(
            "pg_dump failed with exit code {:?}",
            output.status.code()
        ))),
        (false, message) => Err(AppError::Internal(anyhow::anyhow!(message.to_owned()))),
    }
}

async fn is_pg_dump_available() -> bool {
    resolve_pg_dump_executable().await.is_some()
}

async fn resolve_pg_dump_executable() -> Option<PathBuf> {
    match Command::new("pg_dump").arg("--version").output().await {
        Ok(output) if output.status.success() => Some(PathBuf::from("pg_dump")),
        _ => find_windows_pg_dump_executable(),
    }
}

#[cfg(windows)]
fn find_windows_pg_dump_executable() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter_map(|root| std::fs::read_dir(PathBuf::from(root).join("PostgreSQL")).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("bin/pg_dump.exe"))
        .find(|candidate| candidate.is_file())
}

#[cfg(not(windows))]
fn find_windows_pg_dump_executable() -> Option<PathBuf> {
    None
}

async fn ensure_backup_runtime_available(config: &Config) -> Result<(), AppError> {
    let status = runtime_readiness(config).await;
    if config.backup_driver.trim().eq_ignore_ascii_case("json") {
        return Err(AppError::DependencyUnavailable(
            BACKUP_DRIVER_UNSUPPORTED_MESSAGE.into(),
        ));
    }
    match runtime_dependency_error_if_unavailable(
        BackupDriver::from_config(&config.backup_driver),
        status.pg_dump_available,
    ) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn runtime_dependency_error_if_unavailable(
    driver: BackupDriver,
    pg_dump_available: bool,
) -> Option<AppError> {
    if driver == BackupDriver::PgDump && !pg_dump_available {
        return Some(AppError::DependencyUnavailable(
            PG_DUMP_UNAVAILABLE_MESSAGE.to_string(),
        ));
    }
    None
}

pub async fn runtime_readiness(config: &Config) -> BackupRuntimeReadiness {
    let unsupported = config.backup_driver.trim().eq_ignore_ascii_case("json");
    let available = !unsupported && is_pg_dump_available().await;
    let reason = match (unsupported, available) {
        (true, _) => Some(BACKUP_DRIVER_UNSUPPORTED_MESSAGE),
        (false, false) => Some(PG_DUMP_UNAVAILABLE_MESSAGE),
        (false, true) => None,
    };
    BackupRuntimeReadiness {
        pg_dump_available: available,
        run_now_allowed: reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}

/// Builds the manifest for the portable recovery package.
async fn build_recovery_manifest(
    pool: &PgPool,
    dump_path: &Path,
    settings_path: &Path,
    history_path: &Path,
    cache_root: &Path,
    trigger: BackupRunTrigger,
    created_at: DateTime<Utc>,
) -> Result<serde_json::Value, AppError> {
    let current_database = current_database_name(pool).await?;
    let database_profile = restore_compatibility::runtime_database_profile(pool).await?;
    if !database_profile.migration_ledger_successful {
        return Err(AppError::Validation(
            "Backup creation refused because the live migration ledger contains a failed migration."
                .into(),
        ));
    }
    migrations::validate_complete_ledger(&database_profile.migration_ledger).map_err(|error| {
        AppError::Validation(format!(
            "Backup creation refused because the live migration ledger differs from this release: {error}"
        ))
    })?;
    let mut file_components = serde_json::Map::new();
    for (name, path, archived, policy, redactions) in [
        (
            "database",
            dump_path,
            "database.dump",
            "replace_isolated_candidate",
            json!(crate::services::restore_policy::PROTECTED_TABLES),
        ),
        (
            "backup_settings",
            settings_path,
            "backup-settings.json",
            "replace_sanitized_settings",
            json!(["secret_key_encrypted"]),
        ),
        (
            "operational_history",
            history_path,
            "operational-history.json",
            "merge_target_wins",
            json!(["requested_by"]),
        ),
    ] {
        let checksum = compute_sha256(path).await?;
        file_components.insert(
            name.into(),
            json!({"version":1,"path":archived,"sha256":checksum,
            "size_bytes":std::fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0),
            "restore_policy":policy,"redactions":redactions}),
        );
    }
    let cache_files = collect_cache_files(cache_root).await?;
    let cache_size_bytes = cache_files
        .iter()
        .filter_map(|file| file.get("size_bytes").and_then(serde_json::Value::as_u64))
        .sum::<u64>();
    let cache_checksum = hex::encode(Sha256::digest(
        serde_json::to_vec(&cache_files)
            .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?,
    ));

    Ok(json!({
        "format": RECOVERY_PACKAGE_FORMAT,
        "format_version": 3,
        "created_at": created_at,
        "trigger": trigger.as_str(),
        "source": {
            "app_version": std::env::var("PANDA_BUILD_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into()),
            "database": current_database,
            "postgres_major": database_profile.postgres_major,
            "timescale_version": database_profile.timescale_version,
            "migration_version": database_profile.migration_version,
            "migration_ledger": database_profile.migration_ledger,
            "migration_chain_id": database_profile.migration_chain_id,
            "migration_catalog_digest": database_profile.migration_catalog_digest,
            "schema_contract_version": database_profile.schema_contract_version,
            "schema_fingerprint": database_profile.schema_fingerprint,
        },
        "scope": {
            "included": [
                "PostgreSQL durable application data",
                "TimescaleDB telemetry and enrichment data",
                "vehicle artwork cache files",
                "sanitized backup schedule and target configuration",
                "backup execution, package catalog, and restore request history"
            ],
            "redacted": [
                "vehicle_credentials table data (provider credential tokens)",
                "external_connection_settings table data (provider bearer tokens and target secrets)",
                "system_config table data (installation cryptographic keys)",
                "authentication_settings table data (OIDC provider configuration and client secret)",
                "session families, refresh tokens, API keys, and invitation proofs",
                "backup_settings.secret_key_encrypted"
            ],
            "excluded": [
                "Redis live state, pub/sub messages, and OTP challenges",
                "browser localStorage and sessionStorage",
                "external provider connection cache state and activity history"
            ]
        },
        "components": {
            "database": file_components["database"],
            "backup_settings": file_components["backup_settings"],
            "operational_history": file_components["operational_history"],
            "vehicle_image_cache": {
                "version": 1,
                "path": "vehicle-image-cache",
                "sha256": cache_checksum,
                "size_bytes": cache_size_bytes,
                "file_count": cache_files.len(),
                "restore_policy": "replace_atomically",
                "redactions": [],
                "files": cache_files
            }
        },
        "restore": {
            "engine_version": restore_compatibility::RESTORE_ENGINE_VERSION,
            "requires": "same or newer Panda release",
            "provider_credentials": "re-authenticate after restore",
            "operator_command": "node scripts/restore-backup.mjs <package>"
        }
    }))
}

async fn write_sanitized_backup_settings(pool: &PgPool, path: &Path) -> Result<(), AppError> {
    let settings: serde_json::Value = sqlx::query_scalar(
        r#"SELECT COALESCE((
            SELECT jsonb_build_object(
                'present', TRUE,
                'enabled', enabled,
                'frequency', frequency,
                'run_at', run_at,
                'timezone', timezone,
                'day_of_week', day_of_week,
                'day_of_month', day_of_month,
                'retention_count', retention_count,
                'local_enabled', local_enabled,
                's3_enabled', s3_enabled,
                'target_type', target_type,
                'endpoint', endpoint,
                'region', region,
                'bucket', bucket,
                'prefix', prefix,
                'access_key', access_key
            )
            FROM panda.backup_settings
            WHERE id = TRUE
        ), '{"present": false}'::jsonb)"#,
    )
    .fetch_one(pool)
    .await
    .map_err(AppError::from)?;
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
    fs::write(path, bytes).await.map_err(AppError::from)
}

async fn write_operational_history(pool: &PgPool, path: &Path) -> Result<(), AppError> {
    let snapshot = crate::services::restore_jobs::snapshot_catalog(pool).await?;
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
    fs::write(path, bytes).await.map_err(AppError::from)
}

async fn collect_cache_files(cache_root: &Path) -> Result<Vec<serde_json::Value>, AppError> {
    let root = cache_root.to_owned();
    tokio::task::spawn_blocking(move || {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut descriptions = std::collections::BTreeMap::<String, Vec<serde_json::Value>>::new();
        for entry in WalkDir::new(&root) {
            let entry = entry.map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
                let path = format!(
                    "vehicle-image-cache/{}",
                    relative.to_string_lossy().replace('\\', "/")
                );
                let checksum = sha256_file(entry.path())?;
                let size = entry
                    .metadata()
                    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
                    .len();
                descriptions
                    .entry(path.clone())
                    .or_default()
                    .push(json!({"path":path,"sha256":checksum,"size_bytes":size}));
            }
        }
        Ok(descriptions.into_values().flatten().collect())
    })
    .await
    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
}

async fn write_recovery_archive_to_file(
    file: File,
    dump_path: &Path,
    settings_path: &Path,
    history_path: &Path,
    cache_root: &Path,
    manifest_bytes: &[u8],
) -> Result<(), AppError> {
    let sources = [dump_path, settings_path, history_path].map(Path::to_owned);
    let cache = cache_root.to_owned();
    let manifest = manifest_bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        enum Content {
            Bytes(Vec<u8>),
            File(PathBuf),
            Directory,
        }
        let mut entries = vec![("manifest.json".to_owned(), Content::Bytes(manifest))];
        entries.extend(
            [
                "database.dump",
                "backup-settings.json",
                "operational-history.json",
            ]
            .into_iter()
            .zip(sources)
            .map(|(name, path)| (name.to_owned(), Content::File(path))),
        );
        entries.push(("vehicle-image-cache".into(), Content::Directory));
        let mut archive = Builder::new(GzEncoder::new(file, Compression::default()));
        let mut entries = std::collections::VecDeque::from(entries);
        while let Some((name, content)) = entries.pop_front() {
            let directory = matches!(content, Content::Directory);
            let mut header = tar::Header::new_gnu();
            let mut input: Box<dyn Read> = match content {
                Content::Bytes(bytes) => {
                    header.set_size(bytes.len() as u64);
                    header.set_mode(0o600);
                    Box::new(Cursor::new(bytes))
                }
                Content::File(path) => {
                    let input = File::open(path)?;
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(input.metadata()?.len());
                    header.set_mode(0o600);
                    Box::new(input)
                }
                Content::Directory => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_mode(0o700);
                    Box::new(std::io::empty())
                }
            };
            header.set_cksum();
            archive.append_data(&mut header, name, &mut input)?;
            if directory && cache.is_dir() {
                let mut paths = Vec::new();
                for entry in WalkDir::new(&cache).follow_links(false) {
                    let entry = entry?;
                    if entry.file_type().is_file() {
                        paths.push(entry.into_path());
                    }
                }
                paths.sort();
                for path in paths {
                    let name = format!(
                        "vehicle-image-cache/{}",
                        path.strip_prefix(&cache)?
                            .to_string_lossy()
                            .replace('\\', "/")
                    );
                    entries.push_back((name, Content::File(path)));
                }
            }
        }
        archive.into_inner()?.finish()?.sync_all()?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
    .map_err(AppError::from)
}

fn sha256_file(path: &Path) -> Result<String, anyhow::Error> {
    struct DigestWriter(Sha256);
    impl std::io::Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut digest = DigestWriter(Sha256::new());
    std::io::copy(&mut File::open(path)?, &mut digest)?;
    Ok(hex::encode(digest.0.finalize()))
}

async fn current_database_name(pool: &PgPool) -> Result<String, AppError> {
    sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await
        .map_err(AppError::from)
}

async fn current_timescale_version(pool: &PgPool) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT extversion FROM pg_extension WHERE extname = 'timescaledb'")
        .fetch_optional(pool)
        .await
        .map_err(AppError::from)
}

async fn compute_sha256(path: &Path) -> Result<String, AppError> {
    let source = path.to_owned();
    let digest = tokio::task::spawn_blocking(move || sha256_file(&source));
    digest
        .await
        .map_err(|error| AppError::Internal(anyhow::anyhow!("checksum task failed: {error}")))?
        .map_err(AppError::from)
}

fn with_storage(
    base: &serde_json::Value,
    storage_type: &str,
    emergency_fallback: bool,
) -> serde_json::Value {
    let mut manifest = base.clone();
    if let serde_json::Value::Object(fields) = &mut manifest {
        fields.extend(
            std::iter::once(("storage_type".into(), json!(storage_type)))
                .chain(emergency_fallback.then(|| ("emergency_fallback".into(), json!(true)))),
        );
    }
    manifest
}

struct InsertArtifactParams<'a> {
    run_id: Option<Uuid>,
    storage_type: &'a str,
    file_name: &'a str,
    storage_path: &'a str,
    size_bytes: i64,
    checksum_sha256: &'a str,
    manifest: serde_json::Value,
}

async fn insert_artifact(
    pool: &PgPool,
    artifact: InsertArtifactParams<'_>,
) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar(
        r#"INSERT INTO panda.backup_artifacts
           (run_id, storage_type, file_name, storage_path, size_bytes, checksum_sha256, manifest)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (storage_path) WHERE storage_type = 's3'
           DO UPDATE SET file_name = EXCLUDED.file_name, size_bytes = EXCLUDED.size_bytes,
                         checksum_sha256 = EXCLUDED.checksum_sha256, manifest = EXCLUDED.manifest
           RETURNING id"#,
    )
    .bind(artifact.run_id)
    .bind(artifact.storage_type)
    .bind(artifact.file_name)
    .bind(artifact.storage_path)
    .bind(artifact.size_bytes)
    .bind(artifact.checksum_sha256)
    .bind(artifact.manifest)
    .fetch_one(pool)
    .await?)
}

async fn prune_retained_artifacts(
    pool: &PgPool,
    config: &Config,
    retention_count: i32,
) -> Result<(), AppError> {
    let selected = sqlx::query_as::<_, PrunableArtifactRow>(
        "SELECT a.id, a.run_id, a.storage_path FROM panda.backup_artifacts a WHERE a.storage_type = 'local' ORDER BY a.created_at DESC OFFSET $1",
    ).bind(i64::from(retention_count.max(1))).fetch_all(pool).await?;
    for artifact in selected {
        let actions = artifact.retire(Path::new(&config.backup_artifact_dir));
        for action in actions {
            action.apply(pool).await?;
        }
    }
    Ok(())
}

async fn prune_remote_artifacts(
    pool: &PgPool,
    settings: &S3Settings,
    retention_count: i32,
) -> Result<(), AppError> {
    let selected = s3_backups::list(settings)
        .await
        .map_err(remote_retention_error)?;
    let mut remaining = selected.into_iter().skip(retention_count.max(1) as usize);
    for artifact in &mut remaining {
        s3_backups::delete(settings, &artifact.key)
            .await
            .map_err(remote_retention_error)?;
        Retirement::Remote(s3_backups::locator(&settings.bucket, &artifact.key))
            .apply(pool)
            .await?;
    }
    Ok(())
}

fn normalize_prefix(value: &str) -> Result<String, AppError> {
    let prefix = crate::services::artifact_files::validate_prefix(value)?;
    Ok(if prefix.is_empty() {
        "panda".into()
    } else {
        prefix
    })
}

fn compute_due_run_at(
    settings: &BackupSettings,
    now_utc: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    let now_local = now_utc.with_timezone(&settings.timezone);
    let due_local = match settings.frequency {
        BackupFrequency::Daily => most_recent_daily_run(now_local, settings.run_at),
        BackupFrequency::Weekly => most_recent_weekly_run(
            now_local,
            settings.run_at,
            settings.day_of_week.unwrap_or(0),
        ),
        BackupFrequency::Monthly => most_recent_monthly_run(
            now_local,
            settings.run_at,
            settings.day_of_month.unwrap_or(1),
        ),
    };

    Ok(due_local.with_timezone(&Utc))
}

fn most_recent_daily_run(
    now_local: chrono::DateTime<Tz>,
    run_at: NaiveTime,
) -> chrono::DateTime<Tz> {
    calendar::occurrence(now_local, run_at, calendar::Cadence::Daily, false)
}
fn most_recent_weekly_run(
    now_local: chrono::DateTime<Tz>,
    run_at: NaiveTime,
    day_of_week: i16,
) -> chrono::DateTime<Tz> {
    calendar::occurrence(
        now_local,
        run_at,
        calendar::Cadence::Weekly(day_of_week),
        false,
    )
}
fn most_recent_monthly_run(
    now_local: chrono::DateTime<Tz>,
    run_at: NaiveTime,
    day_of_month: i16,
) -> chrono::DateTime<Tz> {
    calendar::occurrence(
        now_local,
        run_at,
        calendar::Cadence::Monthly(day_of_month),
        false,
    )
}

fn should_log_scheduler_failure(
    last_failure_signature: &mut Option<String>,
    error: &AppError,
) -> bool {
    let current = format!("{error:?}");
    last_failure_signature.replace(current.clone()).as_ref() != Some(&current)
}

impl PrunableArtifactRow {
    fn retire(self, root: &Path) -> Vec<Retirement> {
        match crate::services::artifact_files::remove(root, Path::new(&self.storage_path)) {
            Ok(()) => vec![
                Retirement::ClearRun(self.run_id),
                Retirement::Local(self.id),
            ],
            Err(error) => {
                tracing::warn!(artifact_id = %self.id, error = %error, "backup.retention.unsafe_or_unavailable_artifact");
                vec![Retirement::Unavailable(self.id)]
            }
        }
    }
}

enum Retirement {
    ClearRun(Uuid),
    Local(Uuid),
    Unavailable(Uuid),
    Remote(String),
}
impl Retirement {
    async fn apply(self, pool: &PgPool) -> Result<(), AppError> {
        match self {
            Self::Remote(locator) => {
                sqlx::query("DELETE FROM panda.backup_artifacts WHERE storage_type = 's3' AND storage_path = $1")
                    .bind(locator).execute(pool).await?;
            }
            local => {
                let (statement, id) = match local {
                    Self::ClearRun(id) => ("UPDATE panda.backup_runs SET artifact_key = NULL, updated_at = now() WHERE id = $1", id),
                    Self::Local(id) => ("DELETE FROM panda.backup_artifacts WHERE id = $1", id),
                    Self::Unavailable(id) => ("UPDATE panda.backup_artifacts SET manifest = CASE WHEN jsonb_typeof(manifest) = 'object' THEN manifest ELSE '{}'::jsonb END || '{\"restore_availability\":\"unavailable\"}'::jsonb WHERE id = $1", id),
                    Self::Remote(_) => unreachable!(),
                };
                sqlx::query(statement).bind(id).execute(pool).await?;
            }
        }
        Ok(())
    }
}

fn remote_retention_error(error: anyhow::Error) -> AppError {
    AppError::DependencyUnavailable(error.to_string())
}

impl BackupLock {
    async fn run<T>(
        self,
        operation: impl std::future::Future<Output = Result<T, AppError>>,
    ) -> Result<T, AppError> {
        let result = operation.await;
        self.release().await;
        result
    }
}

#[cfg(test)]
use chrono::Duration;

#[cfg(test)]
fn validate_recovery_package_sync(
    package_path: &Path,
) -> Result<ValidatedRecoveryPackage, AppError> {
    validate_recovery_package_sync_with_limits(package_path, &RecoveryConfig::default())
}

#[cfg(test)]
async fn write_recovery_archive(
    artifact_path: &Path,
    dump_path: &Path,
    settings_path: &Path,
    history_path: &Path,
    cache_root: &Path,
    manifest_bytes: &[u8],
) -> Result<(), AppError> {
    write_recovery_archive_to_file(
        File::create(artifact_path)?,
        dump_path,
        settings_path,
        history_path,
        cache_root,
        manifest_bytes,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        classify_recovery_error, runtime_dependency_error_if_unavailable, sha256_file,
        should_log_scheduler_failure, validate_recovery_package, validate_recovery_package_sync,
        write_recovery_archive, BackupDriver, MAX_RECOVERY_MEMBERS,
    };
    use crate::{errors::AppError, services::restore_compatibility};
    use flate2::{read::GzDecoder, write::GzEncoder, Compression};
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use tar::{Archive, Builder, Header};
    use uuid::Uuid;

    #[test]
    fn scheduler_failure_logging_is_deduplicated() {
        let mut last_failure_signature = None;
        let error = AppError::Conflict("backup already running".into());

        assert!(should_log_scheduler_failure(
            &mut last_failure_signature,
            &error
        ));
        assert!(!should_log_scheduler_failure(
            &mut last_failure_signature,
            &error
        ));
    }

    #[test]
    fn scheduler_failure_logging_resets_for_new_errors() {
        let mut last_failure_signature = None;
        let first = AppError::Conflict("first".into());
        let second = AppError::Conflict("second".into());

        assert!(should_log_scheduler_failure(
            &mut last_failure_signature,
            &first
        ));
        assert!(should_log_scheduler_failure(
            &mut last_failure_signature,
            &second
        ));
    }

    #[test]
    fn runtime_dependency_error_is_returned_when_pg_dump_is_missing() {
        let err = runtime_dependency_error_if_unavailable(BackupDriver::PgDump, false)
            .expect("expected dependency error");
        assert!(matches!(err, AppError::DependencyUnavailable(_)));
    }

    #[test]
    fn classifies_recovery_validation_failures_into_stable_errors() {
        assert!(matches!(
            classify_recovery_error(AppError::Validation(
                "Recovery package exceeds the limit".into()
            )),
            AppError::RecoveryTooLarge(_)
        ));
        assert!(matches!(
            classify_recovery_error(AppError::Validation("Unsafe recovery package path".into())),
            AppError::RecoveryInvalid(_)
        ));
    }

    #[test]
    fn recovery_validation_rejects_duplicate_and_traversal_members_before_manifest_parsing() {
        let root = std::env::temp_dir().join(format!("panda-unsafe-tar-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        let duplicate = root.join("duplicate.rma.tar.gz");
        write_test_archive(
            &duplicate,
            &[("manifest.json", b"{}"), ("manifest.json", b"{}")],
        );
        assert!(
            matches!(validate_recovery_package_sync(&duplicate), Err(AppError::Validation(message)) if message.contains("duplicate"))
        );

        let traversal = root.join("traversal.rma.tar.gz");
        write_unsafe_test_archive(&traversal, "../outside", b"no");
        assert!(
            matches!(validate_recovery_package_sync(&traversal), Err(AppError::Validation(message)) if message.contains("Unsafe"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_validation_enforces_member_and_expansion_ratio_limits() {
        let root = std::env::temp_dir().join(format!("panda-bounds-tar-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");

        let member_flood = root.join("member-flood.rma.tar.gz");
        let file = std::fs::File::create(&member_flood).expect("archive file");
        let encoder = GzEncoder::new(file, Compression::default());
        let mut archive = Builder::new(encoder);
        for index in 0..=MAX_RECOVERY_MEMBERS {
            let mut header = Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o600);
            header.set_cksum();
            archive
                .append_data(
                    &mut header,
                    format!("vehicle-image-cache/{index}.webp"),
                    std::io::empty(),
                )
                .expect("append member");
        }
        archive
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip");
        assert!(
            matches!(validate_recovery_package_sync(&member_flood), Err(AppError::Validation(message)) if message.contains("member limit"))
        );

        let expansion_bomb = root.join("expansion-ratio.rma.tar.gz");
        let repeated_bytes = vec![0_u8; 256 * 1024];
        write_test_archive(
            &expansion_bomb,
            &[("database.dump", repeated_bytes.as_slice())],
        );
        assert!(
            matches!(validate_recovery_package_sync(&expansion_bomb), Err(AppError::Validation(message)) if message.contains("expansion ratio"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_metadata_uses_independent_configured_limits_before_decoding() {
        let root = tempfile::tempdir().unwrap();
        let limits = super::RecoveryConfig {
            max_manifest_bytes: 8,
            max_settings_bytes: 8,
            max_history_bytes: 8,
            ..Default::default()
        };
        for name in [
            "manifest.json",
            "backup-settings.json",
            "operational-history.json",
        ] {
            let archive = root.path().join(format!("{name}.rma.tar.gz"));
            write_test_archive(&archive, &[(name, b"123456789")]);
            assert!(
                matches!(super::validate_recovery_package_sync_with_limits(&archive, &limits), Err(AppError::Validation(message)) if message.contains("configured metadata")),
                "{name}"
            );
        }
    }

    fn write_test_archive(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("archive file");
        let encoder = GzEncoder::new(file, Compression::default());
        let mut archive = Builder::new(encoder);
        for (path, contents) in entries {
            let mut header = Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            archive
                .append_data(&mut header, path, *contents)
                .expect("append member");
        }
        let encoder = archive.into_inner().expect("finish tar");
        let mut file = encoder.finish().expect("finish gzip");
        file.flush().expect("flush archive");
    }

    fn write_unsafe_test_archive(path: &std::path::Path, member_path: &str, contents: &[u8]) {
        let file = std::fs::File::create(path).expect("archive file");
        let encoder = GzEncoder::new(file, Compression::default());
        let mut archive = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o600);
        let path_bytes = member_path.as_bytes();
        assert!(path_bytes.len() < 100, "test path must fit in a tar header");
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..path_bytes.len()].copy_from_slice(path_bytes);
        header.set_cksum();
        archive
            .append(&header, contents)
            .expect("append unsafe member");
        archive
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip");
    }

    #[tokio::test]
    async fn recovery_archive_contains_manifest_database_and_artwork() {
        let root = std::env::temp_dir().join(format!("panda-backup-test-{}", Uuid::new_v4()));
        let cache = root.join("cache");
        let dump = root.join("database.dump");
        let settings = root.join("backup-settings.json");
        let history = root.join("operational-history.json");
        let archive = root.join("backup.rma.tar.gz");
        std::fs::create_dir_all(cache.join("vehicle-1")).expect("cache directory");
        std::fs::write(cache.join("vehicle-1/artwork.webp"), b"artwork").expect("artwork");
        std::fs::write(&dump, b"PGDMPdatabase").expect("dump");
        std::fs::write(&settings, br#"{"present":false}"#).expect("settings");
        std::fs::write(
            &history,
            br#"{"runs":[],"artifacts":[],"restore_requests":[]}"#,
        )
        .expect("history");

        let manifest = serde_json::json!({
            "format": "panda-recovery-v1",
            "format_version": 1,
            "components": {
                "database": { "path": "database.dump", "sha256": sha256_file(&dump).expect("dump checksum"), "size_bytes": std::fs::metadata(&dump).expect("dump metadata").len() },
                "backup_settings": { "path": "backup-settings.json", "sha256": sha256_file(&settings).expect("settings checksum"), "size_bytes": std::fs::metadata(&settings).expect("settings metadata").len() },
                "vehicle_image_cache": {
                    "files": [{
                        "path": "vehicle-image-cache/vehicle-1/artwork.webp",
                        "sha256": sha256_file(&cache.join("vehicle-1/artwork.webp")).expect("artwork checksum"),
                        "size_bytes": std::fs::metadata(cache.join("vehicle-1/artwork.webp")).expect("artwork metadata").len()
                    }]
                }
            }
        });
        write_recovery_archive(
            &archive,
            &dump,
            &settings,
            &history,
            &cache,
            &serde_json::to_vec(&manifest).expect("manifest"),
        )
        .await
        .expect("archive");

        let validated = validate_recovery_package(&archive)
            .await
            .expect("valid package");
        assert_eq!(validated.manifest["format"], "panda-recovery-v1");

        let file = std::fs::File::open(&archive).expect("open archive");
        let decoder = GzDecoder::new(file);
        let mut tar = Archive::new(decoder);
        let mut names = Vec::new();
        for entry in tar.entries().expect("entries") {
            let mut entry = entry.expect("entry");
            names.push(
                entry
                    .path()
                    .expect("path")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).expect("read entry");
        }

        assert!(names.iter().any(|name| name == "manifest.json"));
        assert!(names.iter().any(|name| name == "database.dump"));
        assert!(names.iter().any(|name| name == "backup-settings.json"));
        assert!(names.iter().any(|name| name == "operational-history.json"));
        assert!(names
            .iter()
            .any(|name| name.ends_with("vehicle-1/artwork.webp")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn recovery_extraction_preserves_an_empty_artwork_cache() {
        let root = std::env::temp_dir().join(format!("panda-empty-cache-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let dump = root.join("database.dump");
        let settings = root.join("backup-settings.json");
        let history = root.join("operational-history.json");
        std::fs::write(&dump, b"PGDMPsynthetic").unwrap();
        std::fs::write(&settings, b"{}").unwrap();
        std::fs::write(&history, b"{}").unwrap();
        let manifest = serde_json::json!({
            "format": "panda-recovery-v1",
            "format_version": 1,
            "components": {
                "database": {
                    "path": "database.dump",
                    "sha256": sha256_file(&dump).unwrap(),
                    "size_bytes": std::fs::metadata(&dump).unwrap().len()
                },
                "backup_settings": {
                    "path": "backup-settings.json",
                    "sha256": sha256_file(&settings).unwrap(),
                    "size_bytes": std::fs::metadata(&settings).unwrap().len()
                },
                "vehicle_image_cache": { "files": [] }
            }
        });
        let archive = root.join("empty-cache.rma.tar.gz");
        write_recovery_archive(
            &archive,
            &dump,
            &settings,
            &history,
            &root.join("absent-cache"),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .await
        .unwrap();
        let extracted = root.join("extracted");
        super::extract_recovery_package(&archive, &extracted)
            .await
            .unwrap();
        let cache = extracted.join("vehicle-image-cache");
        assert!(cache.is_dir());
        assert_eq!(std::fs::read_dir(cache).unwrap().count(), 0);
        assert_eq!(
            std::fs::read(extracted.join("database.dump")).unwrap(),
            b"PGDMPsynthetic"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn recovery_v3_archive_enforces_the_versioned_component_contract() {
        let root = std::env::temp_dir().join(format!("panda-backup-v2-{}", Uuid::new_v4()));
        let cache = root.join("cache");
        let artwork = cache.join("vehicle-1/artwork.webp");
        let dump = root.join("database.dump");
        let settings = root.join("backup-settings.json");
        let history = root.join("operational-history.json");
        let archive = root.join("backup-v2.rma.tar.gz");
        std::fs::create_dir_all(artwork.parent().expect("artwork parent")).expect("cache");
        std::fs::write(&artwork, b"artwork").expect("artwork");
        std::fs::write(&dump, b"PGDMPdatabase").expect("dump");
        std::fs::write(&settings, br#"{"present":false}"#).expect("settings");
        std::fs::write(
            &history,
            br#"{"runs":[],"artifacts":[],"restore_requests":[]}"#,
        )
        .expect("history");
        let cache_files = vec![serde_json::json!({
            "path": "vehicle-image-cache/vehicle-1/artwork.webp",
            "sha256": sha256_file(&artwork).expect("artwork checksum"),
            "size_bytes": std::fs::metadata(&artwork).expect("artwork metadata").len()
        })];
        let cache_checksum = hex::encode(Sha256::digest(
            serde_json::to_vec(&cache_files).expect("cache manifest"),
        ));
        let contract = |path: &str, checksum: String, size: u64, policy: &str| {
            serde_json::json!({
                "version": 1,
                "path": path,
                "sha256": checksum,
                "size_bytes": size,
                "restore_policy": policy,
                "redactions": []
            })
        };
        let manifest = serde_json::json!({
            "format": "panda-recovery-v3",
            "format_version": 3,
            "source": {
                "app_version": "test",
                "postgres_major": 18,
                "timescale_version": "2.28.3",
                "migration_version": restore_compatibility::latest_migration_version(),
                "migration_ledger": restore_compatibility::compiled_migration_ledger(),
                "migration_chain_id": crate::db::migrations::MIGRATION_CHAIN_ID,
                "migration_catalog_digest": crate::db::migrations::migration_catalog_digest(),
                "schema_contract_version": restore_compatibility::SCHEMA_CONTRACT_VERSION,
                "schema_fingerprint": "test-fingerprint"
            },
            "components": {
                "database": contract("database.dump", sha256_file(&dump).expect("dump checksum"), std::fs::metadata(&dump).expect("dump metadata").len(), "replace_isolated_candidate"),
                "backup_settings": contract("backup-settings.json", sha256_file(&settings).expect("settings checksum"), std::fs::metadata(&settings).expect("settings metadata").len(), "replace_sanitized_settings"),
                "operational_history": contract("operational-history.json", sha256_file(&history).expect("history checksum"), std::fs::metadata(&history).expect("history metadata").len(), "merge_target_wins"),
                "vehicle_image_cache": {
                    "version": 1,
                    "path": "vehicle-image-cache",
                    "sha256": cache_checksum,
                    "size_bytes": std::fs::metadata(&artwork).expect("artwork metadata").len(),
                    "file_count": 1,
                    "restore_policy": "replace_atomically",
                    "redactions": [],
                    "files": cache_files
                }
            },
            "restore": { "engine_version": restore_compatibility::RESTORE_ENGINE_VERSION }
        });
        write_recovery_archive(
            &archive,
            &dump,
            &settings,
            &history,
            &cache,
            &serde_json::to_vec(&manifest).expect("manifest"),
        )
        .await
        .expect("archive");

        let validated = validate_recovery_package(&archive)
            .await
            .expect("valid v3 package");
        assert_eq!(validated.manifest["format"], "panda-recovery-v3");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "backups_recovery_operations_tests.rs"]
mod recovery_operations_tests;

#[cfg(test)]
#[path = "__tests__/backup_workflows/service.rs"]
mod workflow_contracts;

#[cfg(test)]
#[path = "__tests__/backup_workflows/support.rs"]
pub(crate) mod workflow_test_support;

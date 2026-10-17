use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde_json::json;
use tokio::{fs, net::TcpListener, process::Command, time::sleep};
use uuid::Uuid;

use panda_api::{
    config::Config,
    logging,
    services::{
        backups, restore_compatibility, restore_history,
        restore_jobs::{self, RestorePhase},
        restore_policy,
    },
};

const MAX_AUTH_HEADER_BYTES: usize = 512;

#[path = "restore_execution/activation.rs"]
mod activation;
#[path = "restore_execution/rollback.rs"]
mod rollback;

#[derive(Clone)]
struct AgentState {
    config: Config,
    agent_key: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init();
    let config = Config::from_env()?;
    cleanup_restore_roles(&config).await?;
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.first().map(String::as_str) {
        Some("host-restore") => {
            let package = arguments
                .get(1)
                .context("host-restore requires a package path")?;
            host_restore(
                &config,
                Path::new(package),
                arguments.iter().any(|value| value == "--force"),
            )
            .await
        }
        Some("host-finalize") => finalize_host_restore(&config).await,
        Some("host-activate") => activate_host_restore(&config).await,
        Some("host-rollback") => rollback_host_restore(&config).await,
        _ => {
            let key = fs::read_to_string(restore_jobs::agent_key_path(&config)).await?;
            let state = AgentState {
                config,
                agent_key: key.trim().into(),
            };
            let port = std::env::var("RESTORE_AGENT_PORT").unwrap_or_else(|_| "3002".into());
            let address: SocketAddr = format!("127.0.0.1:{port}").parse()?;
            let listener = TcpListener::bind(address).await?;
            let recovery = state.clone();
            let router = Router::new()
                .route("/health", get(|| async { StatusCode::OK }))
                .route("/v1/restore-runtime/jobs/{job_id}", get(get_job))
                .route("/internal/jobs/{job_id}/execute", post(execute_job))
                .with_state(state);
            tokio::spawn(async move {
                if let Err(error) = retry_interrupted_restore(&recovery).await {
                    tracing::error!(error = ?error, "interrupted restore retry failed");
                }
            });
            Ok(axum::serve(listener, router).await?)
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HostRestoreState {
    job_id: Uuid,
    artifact_id: Uuid,
    request_id: Uuid,
    candidate_database: String,
    previous_database: String,
    #[serde(default)]
    artwork_destination: Option<PathBuf>,
    #[serde(default)]
    artwork_previous: Option<PathBuf>,
    #[serde(default)]
    activated: bool,
    #[serde(default)]
    activation_started: bool,
    staging: PathBuf,
    plan: restore_compatibility::RestorePlan,
    validation_report: restore_compatibility::CandidatePreparationReport,
}

fn host_restore_state_path(config: &Config) -> PathBuf {
    Path::new(&config.backup_artifact_dir).join(".host-restore-state.json")
}

async fn host_restore(config: &Config, package: &Path, force: bool) -> anyhow::Result<()> {
    let lock_pool = panda_api::db::pool::create_pool(&config.database_url).await?;
    let lock = backups::acquire_recovery_mutation_lock(&lock_pool).await?;
    let job_id = Uuid::new_v4();
    let result = with_restore_deadline(
        config,
        job_id,
        host_restore_inner(config, package, force, job_id),
    )
    .await;
    lock.release().await;
    lock_pool.close().await;
    result
}

async fn host_restore_inner(
    config: &Config,
    package: &Path,
    force: bool,
    job_id: Uuid,
) -> anyhow::Result<()> {
    let checkpoint_path = host_restore_state_path(config);
    anyhow::ensure!(
        !fs::try_exists(&checkpoint_path).await.unwrap_or(false),
        "a host restore is already awaiting finalize or rollback"
    );
    let package_info =
        backups::validate_recovery_package_with_limits(package, &config.recovery).await?;
    let inspection = restore_compatibility::inspect_recovery_dump(
        package,
        Path::new(&config.backup_artifact_dir),
        &config.recovery,
    )
    .await?;
    let pool = panda_api::db::pool::create_pool(&config.database_url).await?;
    initialize_host_target(&pool, force).await?;
    let active = config_with_active_keys(config, &pool).await?;
    backups::reconcile_local_catalog(&pool, &active).await?;
    let plan = restore_compatibility::plan_restore(
        &package_info.manifest,
        &package_info.checksum_sha256,
        &pool,
        Some(&inspection),
    )
    .await?;
    if !plan.compatible {
        let messages = plan
            .blocking_errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        anyhow::bail!("host restore compatibility preflight failed: {messages}");
    }
    let history = restore_jobs::snapshot_catalog(&pool).await.ok();
    let staging = Path::new(&active.backup_artifact_dir)
        .join(".restore-staging")
        .join(format!("host-{job_id}"));
    fs::create_dir_all(&staging).await?;
    let staged = stage_host_package(
        &active,
        package,
        job_id,
        &staging,
        &package_info.checksum_sha256,
    )
    .await?;
    extract_package(&active, &staged, &staging).await?;
    ensure_recovery_free_space(&staging, active.recovery.min_free_bytes)?;
    let prepared = prepare_restore_database(
        &active,
        job_id,
        &staging.join("database.dump"),
        &package_info.manifest,
        &staging.join("backup-settings.json"),
        Some(&staging.join("operational-history.json")),
        history.as_ref(),
    )
    .await?;
    let approval = async {
        verify_candidate_plan(Some(&plan), &prepared)?;
        capture_host_safety(&active, &pool, &prepared.candidate_database).await
    }
    .await;
    if let Err(error) = approval {
        let _ = drop_database(&active, &prepared.candidate_database).await;
        pool.close().await;
        return Err(error);
    }
    pool.close().await;
    let state = HostRestoreState {
        job_id,
        artifact_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        candidate_database: prepared.candidate_database,
        previous_database: prepared.previous_database,
        artwork_destination: None,
        artwork_previous: None,
        activated: false,
        activation_started: false,
        staging,
        plan,
        validation_report: prepared.validation_report,
    };
    let published = match serde_json::to_vec_pretty(&state) {
        Ok(bytes) => fs::write(&checkpoint_path, bytes)
            .await
            .map_err(anyhow::Error::from),
        Err(error) => Err(error.into()),
    };
    if let Err(error) = published {
        let _ = drop_database(&active, &state.candidate_database).await;
        let _ = fs::remove_dir_all(&state.staging).await;
        return Err(error);
    }
    println!(
        "host restore candidate prepared and validated; state={}",
        checkpoint_path.display()
    );
    Ok(())
}

async fn read_host_restore_state(config: &Config) -> anyhow::Result<HostRestoreState> {
    let input = fs::read(host_restore_state_path(config)).await?;
    serde_json::from_slice(&input).map_err(Into::into)
}

async fn activate_host_restore(config: &Config) -> anyhow::Result<()> {
    activation::host(config).await
}

fn artwork_activation_from_state(state: &HostRestoreState) -> anyhow::Result<ArtworkActivation> {
    match (&state.artwork_destination, &state.artwork_previous) {
        (None, _) => anyhow::bail!("host restore artwork destination is unavailable"),
        (_, None) => anyhow::bail!("host restore previous artwork path is unavailable"),
        (Some(destination), Some(previous)) => Ok(ArtworkActivation {
            destination: destination.clone(),
            previous: previous.clone(),
        }),
    }
}

async fn finalize_host_restore(config: &Config) -> anyhow::Result<()> {
    let state = read_host_restore_state(config).await?;
    match state.activated {
        false => anyhow::bail!("host restore candidate has not been activated"),
        true => {
            record_host_restore_history(config, &state, "completed", None).await?;
            drop_database(config, &state.previous_database).await?;
            let artwork = artwork_activation_from_state(&state)?;
            finalize_artwork(&artwork).await;
            let _ = fs::remove_dir_all(&state.staging).await;
            write_host_restore_report(config, &state, "completed", None).await?;
            fs::remove_file(host_restore_state_path(config)).await?;
            Ok(())
        }
    }
}

async fn rollback_host_restore(config: &Config) -> anyhow::Result<()> {
    let state = read_host_restore_state(config).await?;
    let swapped = match (state.activated, state.activation_started) {
        (true, _) => true,
        (false, false) => false,
        (false, true) => database_exists(config, &state.previous_database)
            .await
            .unwrap_or(false),
    };
    match swapped {
        false => drop_database(config, &state.candidate_database).await?,
        true => {
            rollback_databases(config, &state.previous_database, &state.candidate_database).await?;
            if let (Some(destination), Some(previous)) =
                (&state.artwork_destination, &state.artwork_previous)
            {
                rollback_artwork(&ArtworkActivation {
                    destination: destination.clone(),
                    previous: previous.clone(),
                })
                .await?;
            }
        }
    }
    record_host_restore_history(
        config,
        &state,
        "failed",
        Some("Application verification failed after candidate activation; rollback succeeded"),
    )
    .await?;
    let _ = fs::remove_dir_all(&state.staging).await;
    write_host_restore_report(
        config,
        &state,
        "rolled_back",
        Some("Application verification failed after candidate activation"),
    )
    .await?;
    Ok(fs::remove_file(host_restore_state_path(config)).await?)
}

async fn record_host_restore_history(
    config: &Config,
    state: &HostRestoreState,
    status: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let completed = status == "completed";
    let run = json!({
        "id":state.job_id,"status":if completed {"succeeded"} else {"failed"},
        "phase":if completed {"completed"} else {"failed"},"progress":if completed {100_i16} else {0_i16},
        "artifact_key":format!("restore:{}",state.plan.package_checksum_sha256),"error":error,
    });
    let artifact = json!({
        "id":state.artifact_id,"run_id":state.job_id,
        "path":format!("host-restore:{}",state.plan.package_checksum_sha256),"checksum":state.plan.package_checksum_sha256,
        "manifest":{"artifact_kind":"recovery_package","format":state.plan.package_format,"restore_availability":"unavailable","plan_id":state.plan.plan_id},
    });
    let request = json!({"id":state.request_id,"artifact_id":state.artifact_id,"status":status,"error":error});
    let pool = panda_api::db::pool::create_pool(&config.database_url).await?;
    for (statement, record) in [
        (
            r#"INSERT INTO panda.backup_runs
            (id, trigger, status, phase, progress_percent, requested_by, artifact_key, started_at, completed_at, error_message, created_at, updated_at)
            SELECT id, 'restore', status, phase, progress, NULL, artifact_key, now(), now(), error, now(), now()
            FROM jsonb_to_record($1) AS row(id uuid, status text, phase text, progress smallint, artifact_key text, error text)
            ON CONFLICT (id) DO UPDATE SET status=EXCLUDED.status, phase=EXCLUDED.phase, progress_percent=EXCLUDED.progress_percent,
                completed_at=EXCLUDED.completed_at, error_message=EXCLUDED.error_message, updated_at=now()"#,
            run,
        ),
        (
            r#"INSERT INTO panda.backup_artifacts
            (id, run_id, storage_type, file_name, storage_path, size_bytes, checksum_sha256, manifest, created_at)
            SELECT id, run_id, 'uploaded', 'host-restore-package.rma.tar.gz', path, 0, checksum, manifest, now()
            FROM jsonb_to_record($1) AS row(id uuid, run_id uuid, path text, checksum text, manifest jsonb)
            ON CONFLICT (id) DO NOTHING"#,
            artifact,
        ),
        (
            r#"INSERT INTO panda.backup_restore_requests
            (id, artifact_id, requested_by, status, confirmation_phrase, notes, error_message, requested_at, updated_at)
            SELECT id, artifact_id, NULL, status, 'RESTORE', 'Host restore command', error, now(), now()
            FROM jsonb_to_record($1) AS row(id uuid, artifact_id uuid, status text, error text)
            ON CONFLICT (id) DO UPDATE SET status=EXCLUDED.status, error_message=EXCLUDED.error_message, updated_at=now()"#,
            request,
        ),
    ] {
        sqlx::query(statement).bind(record).execute(&pool).await?;
    }
    pool.close().await;
    Ok(())
}

async fn write_host_restore_report(
    config: &Config,
    state: &HostRestoreState,
    status: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    #[derive(serde::Serialize)]
    struct Report<'a> {
        job_id: Uuid,
        status: &'a str,
        completed_at: chrono::DateTime<Utc>,
        plan: &'a restore_compatibility::RestorePlan,
        validation_report: &'a restore_compatibility::CandidatePreparationReport,
        error: Option<&'a str>,
    }
    let report = Report {
        job_id: state.job_id,
        status,
        completed_at: Utc::now(),
        plan: &state.plan,
        validation_report: &state.validation_report,
        error,
    };
    let directory = Path::new(&config.backup_artifact_dir).join(".restore-reports");
    fs::create_dir_all(&directory).await?;
    // Value retains the historical lexicographic JSON object ordering.
    let bytes = serde_json::to_vec_pretty(&serde_json::to_value(report)?)?;
    Ok(fs::write(directory.join(format!("{}.json", state.job_id)), bytes).await?)
}

async fn retry_interrupted_restore(state: &AgentState) -> anyhow::Result<()> {
    let directory = restore_jobs::jobs_dir(&state.config);
    if !fs::try_exists(&directory).await.unwrap_or(false) {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|name| name.parse::<Uuid>().ok())
        else {
            continue;
        };
        let Ok(mut job) = restore_jobs::read(&state.config, id).await else {
            continue;
        };
        if !retry_admitted(&state.config, &job).await {
            continue;
        }
        if let Some(previous) = job.previous_database.as_deref() {
            if database_exists(&state.config, previous)
                .await
                .unwrap_or(false)
            {
                let candidate = job
                    .candidate_database
                    .clone()
                    .unwrap_or_else(|| format!("panda_restore_{}", job.id.simple()));
                let _ = fs::write(restore_marker_path(), job.id.to_string()).await;
                let _ = stop_api_process().await;
                rollback_databases(&state.config, previous, &candidate).await?;
                let _ = drop_database(&state.config, &candidate).await;
                let _ = fs::remove_file(restore_marker_path()).await;
            }
        }
        job.automatic_retry_count = job.automatic_retry_count.saturating_add(1);
        job.phase = RestorePhase::PreparingCandidate;
        job.progress_percent = 25;
        job.message = "Retrying interrupted restore with an isolated database".into();
        job.error_message = None;
        job.updated_at = Utc::now();
        restore_jobs::write(&state.config, &job).await?;
        if let Err(error) = perform_restore(&state.config, job.id).await {
            let _ = fs::remove_file(restore_marker_path()).await;
            let _ = restore_jobs::fail(&state.config, job.id, error).await;
        }
        break;
    }
    Ok(())
}

async fn get_job(
    State(state): State<AgentState>,
    AxumPath(job_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<restore_jobs::RestoreJobPublic>, StatusCode> {
    let token = credential_header(&headers, "x-panda-restore-token")?;
    let job = restore_jobs::read(&state.config, job_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    match restore_jobs::token_matches(&job, token) {
        true => Ok(Json(job.public())),
        false => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn execute_job(
    State(state): State<AgentState>,
    AxumPath(job_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let key = credential_header(&headers, "x-panda-agent-key")?;
    if !restore_jobs::constant_time_equal(key.as_bytes(), state.agent_key.as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    match restore_jobs::read(&state.config, job_id).await {
        Err(_) => Err(StatusCode::NOT_FOUND),
        Ok(job) if !matches!(job.phase, RestorePhase::PreparingCandidate) => {
            Err(StatusCode::CONFLICT)
        }
        Ok(_) => {
            tokio::spawn(async move {
                let Err(error) = perform_restore(&state.config, job_id).await else {
                    return;
                };
                tracing::error!(job_id = %job_id, error = ?error, "restore agent job failed");
                let _ = restore_jobs::fail(&state.config, job_id, &error).await;
                let _ = fs::remove_file(restore_marker_path()).await;
                if let Ok(job) = restore_jobs::read(&state.config, job_id).await {
                    cleanup_remote_staging(&job.artifact_path).await;
                }
            });
            Ok((StatusCode::ACCEPTED, Json(json!({"accepted": true}))))
        }
    }
}

async fn perform_restore(config: &Config, job_id: Uuid) -> anyhow::Result<()> {
    let lock_pool = panda_api::db::pool::create_pool(&config.database_url).await?;
    let config = config_with_active_keys(config, &lock_pool).await?;
    let lock = backups::acquire_recovery_mutation_lock(&lock_pool).await?;
    let result =
        with_restore_deadline(&config, job_id, perform_restore_inner(&config, job_id)).await;
    lock.release().await;
    lock_pool.close().await;
    result
}

async fn with_restore_deadline<T>(
    config: &Config,
    job_id: Uuid,
    restore: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(config.recovery.restore_deadline_seconds);
    let result = tokio::time::timeout_at(deadline, restore).await;
    if let Ok(completed) = result {
        return completed;
    }
    cleanup_failed_candidate(config, job_id)
        .await
        .context("restore deadline exceeded; candidate cleanup failed")?;
    anyhow::bail!("restore exceeded the configured deadline; candidate and credentials cleaned up")
}

async fn cleanup_failed_candidate(config: &Config, job_id: Uuid) -> anyhow::Result<()> {
    let maintenance = config_with_database(config, "postgres")?;
    let owner = format!("panda_restore_owner_{}", job_id.simple());
    let pool = panda_api::db::pool::create_pool(&maintenance.database_url).await?;
    let present: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=$1)")
            .bind(&owner)
            .fetch_one(&pool)
            .await?;
    if present {
        for action in [
            RoleCleanup::Terminate,
            RoleCleanup::Revoke,
            RoleCleanup::Terminate,
        ] {
            action.execute(&pool, &owner).await?;
        }
    }
    pool.close().await;
    let database_result =
        drop_database(&maintenance, &format!("panda_restore_{}", job_id.simple())).await;
    let roles_result = cleanup_restore_roles(&maintenance).await;
    database_result.and(roles_result)
}

async fn config_with_active_keys(config: &Config, pool: &sqlx::PgPool) -> anyhow::Result<Config> {
    let active_keys = panda_api::keys::bootstrap_keys_for_config(pool, config)
        .await
        .context("bootstrap restore cryptographic keys")?;
    let mut config = config.clone();
    config.age_encryption_key = Some(active_keys.age_key);
    Ok(config)
}

async fn perform_restore_inner(config: &Config, job_id: Uuid) -> anyhow::Result<()> {
    let job = restore_jobs::read(config, job_id).await?;
    let staging = Path::new(&config.backup_artifact_dir)
        .join(".restore-staging")
        .join(job_id.to_string());
    if fs::try_exists(&staging).await.unwrap_or(false) {
        fs::remove_dir_all(&staging).await?;
    }
    fs::create_dir_all(&staging).await?;
    ensure_recovery_free_space(&staging, config.recovery.min_free_bytes)?;
    let staged_package =
        stage_recovery_package(config, job_id, false, Path::new(&job.artifact_path)).await?;
    ensure_recovery_free_space(&staging, config.recovery.min_free_bytes)?;
    let validated =
        backups::validate_recovery_package_with_limits(&staged_package, &config.recovery).await?;
    if job
        .plan
        .as_ref()
        .map(|plan| plan.package_checksum_sha256.as_str())
        != Some(validated.checksum_sha256.as_str())
    {
        let mut terminal = job;
        terminal.retryable = false;
        terminal.updated_at = Utc::now();
        restore_jobs::write(config, &terminal).await?;
        anyhow::bail!("recovery package checksum changed after restore preflight");
    }
    inject_restore_fault("package_validated")?;
    extract_package(config, &staged_package, &staging).await?;
    ensure_recovery_free_space(&staging, config.recovery.min_free_bytes)?;

    restore_jobs::update(
        config,
        job_id,
        RestorePhase::PreparingCandidate,
        32,
        "Restoring and validating an isolated database candidate",
    )
    .await?;
    let prepared = match prepare_restore_database(
        config,
        job_id,
        &staging.join("database.dump"),
        &validated.manifest,
        &staging.join("backup-settings.json"),
        Some(&staging.join("operational-history.json")),
        job.catalog_snapshot.as_ref(),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            let mut terminal = restore_jobs::read(config, job_id).await?;
            terminal.retryable = false;
            terminal.updated_at = Utc::now();
            restore_jobs::write(config, &terminal).await?;
            return Err(error);
        }
    };
    if let Err(error) = verify_candidate_plan(job.plan.as_ref(), &prepared) {
        let _ = drop_database(config, &prepared.candidate_database).await;
        let mut terminal = restore_jobs::read(config, job_id).await?;
        terminal.retryable = false;
        terminal.updated_at = Utc::now();
        restore_jobs::write(config, &terminal).await?;
        return Err(error);
    }
    inject_restore_fault("candidate_validated")?;

    restore_jobs::update(
        config,
        job_id,
        RestorePhase::SafetyBackup,
        58,
        "Candidate validated; creating the required safety package",
    )
    .await?;
    let live_pool = panda_api::db::pool::create_pool(&config.database_url).await?;
    let safety = backups::run_backup_now(
        &live_pool,
        config,
        None,
        backups::BackupRunTrigger::PreRestore,
    )
    .await?;
    inject_restore_fault("safety_backup")?;
    backups::reconcile_local_catalog(&live_pool, config).await?;
    let (safety_artifact_id, safety_path): (Uuid, String) = sqlx::query_as(
        "SELECT id, storage_path FROM panda.backup_artifacts WHERE run_id = $1 AND storage_type = 'safety' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(safety.run_id)
    .fetch_one(&live_pool)
    .await?;
    let final_target_history = restore_jobs::snapshot_catalog(&live_pool).await?;
    live_pool.close().await;
    let candidate_config = config_with_database(config, &prepared.candidate_database)?;
    let candidate_pool = panda_api::db::pool::create_pool(&candidate_config.database_url).await?;
    restore_jobs::merge_catalog_snapshot(&candidate_pool, &final_target_history).await?;
    candidate_pool.close().await;
    inject_restore_fault("history_merged")?;

    let mut checkpoint = restore_jobs::read(config, job_id).await?;
    checkpoint.safety_artifact_path = Some(safety_path);
    checkpoint.safety_run_id = Some(safety.run_id);
    checkpoint.safety_artifact_id = Some(safety_artifact_id);
    checkpoint.catalog_snapshot = Some(final_target_history);
    checkpoint.validation_report = Some(prepared.validation_report.clone());
    checkpoint.candidate_database = Some(prepared.candidate_database.clone());
    checkpoint.previous_database = Some(prepared.previous_database.clone());
    checkpoint.rollback_state = restore_jobs::RollbackState::Available;
    checkpoint.updated_at = Utc::now();
    restore_jobs::write(config, &checkpoint).await?;

    let artwork = activation::runtime(config, job_id, &prepared, &staging).await?;
    finalize_artwork(&artwork).await;
    let _ = fs::remove_dir_all(&staging).await;
    if let Err(error) = drop_database(config, &prepared.previous_database).await {
        tracing::warn!(database = %prepared.previous_database, error = ?error, "could not remove the previous database after a successful restore");
    }
    restore_jobs::update(
        config,
        job_id,
        RestorePhase::Completed,
        100,
        "Restore completed successfully",
    )
    .await?;
    cleanup_remote_staging(&job.artifact_path).await;
    Ok(())
}

async fn rollback_activated_candidate(
    config: &Config,
    job_id: Uuid,
    prepared: &PreparedDatabase,
    artwork: Option<&ArtworkActivation>,
    cause: anyhow::Error,
) -> anyhow::Error {
    rollback::runtime(config, job_id, prepared, artwork, cause).await
}

fn inject_restore_fault(phase: &str) -> anyhow::Result<()> {
    if std::env::var("PANDA_RESTORE_FAULT_PHASE").as_deref() == Ok(phase) {
        anyhow::bail!("restore fault injection triggered at {phase}");
    }
    Ok(())
}

async fn cleanup_remote_staging(path: &str) {
    let file = Path::new(path);
    match file.parent().and_then(Path::file_name) {
        Some(parent) if parent == ".remote-staging" => {
            let _ = fs::remove_file(file).await;
        }
        _ => {}
    }
}

fn api_pid_path() -> PathBuf {
    std::env::var("PANDA_API_PID_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/panda-api.pid"))
}

fn restore_marker_path() -> PathBuf {
    std::env::var("PANDA_RESTORE_MARKER_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/panda-restore-in-progress"))
}

#[cfg(not(windows))]
async fn stop_api_process() -> anyhow::Result<()> {
    let pid = match fs::read_to_string(api_pid_path()).await {
        Ok(value) => value
            .trim()
            .parse::<u32>()
            .context("Panda API pid file is invalid")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let process = PathBuf::from(format!("/proc/{pid}"));
    if !process.exists() {
        return Ok(());
    }
    let status = Command::new("/bin/sh")
        .args(["-c", "kill -TERM \"$1\"", "panda-restore-agent"])
        .arg(pid.to_string())
        .status()
        .await?;
    anyhow::ensure!(status.success(), "could not stop the Panda API process");
    retry_operation(300, Duration::from_millis(100), || async {
        if process.exists() {
            Err(anyhow::anyhow!("Panda API did not stop before restore"))
        } else {
            Ok(())
        }
    })
    .await
}

#[cfg(windows)]
async fn stop_api_process() -> anyhow::Result<()> {
    let pid = fs::read_to_string(api_pid_path()).await?;
    let status = Command::new("taskkill")
        .args(["/PID", pid.trim(), "/T", "/F"])
        .status()
        .await?;
    anyhow::ensure!(status.success(), "could not stop the Panda API process");
    retry_operation(300, Duration::from_millis(100), || async {
        let listing = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid.trim()), "/NH"])
            .output()
            .await;
        match listing {
            Ok(output) if String::from_utf8_lossy(&output.stdout).contains(pid.trim()) => {
                Err(anyhow::anyhow!("Panda API did not stop before restore"))
            }
            _ => Ok(()),
        }
    })
    .await
}

async fn extract_package(config: &Config, package: &Path, staging: &Path) -> anyhow::Result<()> {
    backups::extract_recovery_package_with_limits(package, staging, &config.recovery).await?;
    Ok(())
}

async fn stage_recovery_package(
    config: &Config,
    job_id: Uuid,
    host: bool,
    package: &Path,
) -> anyhow::Result<PathBuf> {
    let mut source = fs::File::open(package).await?;
    Ok(panda_api::services::artifact_files::stage_restore_reader(
        Path::new(&config.backup_artifact_dir),
        job_id,
        host,
        &mut source,
        &config.recovery,
    )
    .await?)
}

fn ensure_recovery_free_space(path: &Path, minimum: u64) -> anyhow::Result<()> {
    match fs2::available_space(path)? >= minimum {
        true => Ok(()),
        false => anyhow::bail!(
            "recovery storage needs at least {} GiB free before continuing",
            minimum / 1024 / 1024 / 1024
        ),
    }
}

async fn terminate_database_sessions(config: &Config) -> anyhow::Result<()> {
    run_psql(
        config,
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid();",
    )
    .await
}

async fn build_restore_toc(dump: &Path, toc_path: &Path) -> anyhow::Result<()> {
    let output = Command::new("pg_restore")
        .arg("--list")
        .arg(dump)
        .output()
        .await?;
    let bytes = checked_output(output, "pg_restore could not inspect the recovery dump")?;
    let mut filtered = String::new();
    for line in String::from_utf8_lossy(&bytes).lines() {
        if !restore_policy::skip_toc_entry(line) {
            filtered.push_str(line);
            filtered.push('\n');
        }
    }
    if filtered.is_empty() {
        filtered.push('\n');
    }
    Ok(fs::write(toc_path, filtered).await?)
}

async fn run_pg_restore(config: &Config, dump: &Path, toc_path: &Path) -> anyhow::Result<()> {
    let output = postgres_command("pg_restore", config)?
        .args([
            "--no-owner",
            "--no-privileges",
            "--exit-on-error",
            "--single-transaction",
        ])
        .arg(format!("--use-list={}", toc_path.display()))
        .arg(dump)
        .output()
        .await?;
    checked_output(output, "pg_restore failed").map(|_| ())
}

struct PreparedDatabase {
    candidate_database: String,
    previous_database: String,
    validation_report: restore_compatibility::CandidatePreparationReport,
}

async fn prepare_restore_database(
    config: &Config,
    job_id: Uuid,
    dump: &Path,
    manifest: &serde_json::Value,
    settings: &Path,
    history: Option<&Path>,
    target_history: Option<&restore_jobs::BackupCatalogSnapshot>,
) -> anyhow::Result<PreparedDatabase> {
    let candidate_name = format!("panda_restore_{}", job_id.simple());
    let previous_database = format!("panda_previous_{}", job_id.simple());
    let candidate = config_with_database(config, &candidate_name)?;
    let toc = dump.with_extension("restore-toc");
    if database_exists(config, &candidate_name).await? {
        drop_database(config, &candidate_name).await?;
    }
    create_database(config, &candidate_name).await?;
    let credentials = CandidateCredentials::new(&candidate, job_id)?;
    let attempt = async {
        inject_restore_fault("timescale_pre_restore")?;
        run_psql_with_retry(&candidate,
            "CREATE SCHEMA IF NOT EXISTS panda; CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda; CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda; SELECT timescaledb_pre_restore();", 30).await?;
        credentials.provision(&candidate, &candidate_name, config.recovery.restore_deadline_seconds).await?;
        build_restore_toc(dump, &toc).await?;
        run_pg_restore(&credentials.archive, dump, &toc).await?;
        inject_restore_fault("dump_restored")?;
        let pool = panda_api::db::pool::create_pool(&credentials.archive.database_url).await?;
        let report = restore_compatibility::prepare_candidate_schema(&pool, &config.database_url, manifest).await?;
        for source in [history].into_iter().flatten() {
            if fs::try_exists(source).await.unwrap_or(false) {
                restore_history::merge(&pool, source, config.recovery.max_history_bytes).await?;
            }
        }
        if let Some(snapshot) = target_history { restore_jobs::merge_catalog_snapshot(&pool, snapshot).await?; }
        pool.close().await;
        run_psql(&candidate, "SELECT timescaledb_post_restore();").await?;
        restore_backup_settings(&credentials.archive, settings).await?;
        run_psql(config, &format!("ALTER ROLE {} NOLOGIN PASSWORD NULL;", quote_identifier(&credentials.role))).await?;
        Ok::<_, anyhow::Error>(report)
    }.await;
    if attempt.is_err() {
        if let Err(cleanup) = cleanup_failed_candidate(config, job_id).await {
            tracing::error!(job_id = %job_id, ?cleanup, "failed candidate cleanup failed");
        }
    }
    let _ = fs::remove_file(&toc).await;
    attempt.map(|validation_report| PreparedDatabase {
        candidate_database: candidate_name,
        previous_database,
        validation_report,
    })
}

fn config_with_database(config: &Config, database: &str) -> anyhow::Result<Config> {
    let mut address = url::Url::parse(&config.database_url)?;
    address.set_path(&format!("/{database}"));
    Ok(Config {
        database_url: address.into(),
        ..config.clone()
    })
}

/// Make interrupted credentials unusable at startup. Roles with live object
/// dependencies remain NOLOGIN owners; orphan roles are removed after DB cleanup.
async fn cleanup_restore_roles(config: &Config) -> anyhow::Result<()> {
    let maintenance = config_with_database(config, "postgres")?;
    let pool = panda_api::db::pool::create_pool(&maintenance.database_url).await?;
    let owners: Vec<String> = sqlx::query_scalar(
        "SELECT rolname FROM pg_roles WHERE rolname ~ '^panda_restore_owner_[0-9a-f]{32}$'",
    )
    .fetch_all(&pool)
    .await?;
    for owner in owners {
        RoleCleanup::Revoke.execute(&pool, &owner).await?;
        let dependent: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_shdepend WHERE refclassid = 'pg_authid'::regclass AND refobjid = (SELECT oid FROM pg_roles WHERE rolname = $1))").bind(&owner).fetch_one(&pool).await?;
        if !dependent {
            RoleCleanup::Drop.execute(&pool, &owner).await?;
        }
    }
    pool.close().await;
    Ok(())
}

async fn create_database(config: &Config, database: &str) -> anyhow::Result<()> {
    let sql = format!(
        "CREATE DATABASE {database} TEMPLATE template0;",
        database = quote_identifier(database)
    );
    run_psql_on_database(config, "postgres", &sql).await
}

async fn swap_databases(
    config: &Config,
    restore_database: &str,
    previous_database: &str,
) -> anyhow::Result<()> {
    exchange_databases(config, restore_database, previous_database).await
}

async fn rollback_databases(
    config: &Config,
    previous_database: &str,
    failed_database: &str,
) -> anyhow::Result<()> {
    exchange_databases(config, previous_database, failed_database).await
}

async fn drop_database(config: &Config, database: &str) -> anyhow::Result<()> {
    let sql = format!(
        "DROP DATABASE IF EXISTS {database} WITH (FORCE);",
        database = quote_identifier(database)
    );
    run_psql_on_database(config, "postgres", &sql).await?;
    cleanup_restore_roles(config).await
}

async fn database_exists(config: &Config, database: &str) -> anyhow::Result<bool> {
    let query = format!(
        "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = {});",
        quote_literal(database)
    );
    let output = postgres_command_for_database("psql", config, "postgres")?
        .args(["-At", "-v", "ON_ERROR_STOP=1", "-c", &query])
        .output()
        .await?;
    let bytes = checked_output(output, "could not inspect database state")?;
    Ok(String::from_utf8_lossy(&bytes).trim() == "t")
}

async fn run_psql_on_database(config: &Config, database: &str, sql: &str) -> anyhow::Result<()> {
    let output = postgres_command_for_database("psql", config, database)?
        .args(["-v", "ON_ERROR_STOP=1", "-c", sql])
        .output()
        .await?;
    checked_output(output, "psql failed").map(|_| ())
}

async fn run_psql(config: &Config, sql: &str) -> anyhow::Result<()> {
    let database = url::Url::parse(&config.database_url)?
        .path()
        .trim_start_matches('/')
        .to_string();
    run_psql_on_database(config, &database, sql).await
}

async fn run_psql_with_retry(config: &Config, sql: &str, attempts: u32) -> anyhow::Result<()> {
    retry_operation(attempts, Duration::from_secs(1), || run_psql(config, sql)).await
}

fn postgres_command(program: &str, config: &Config) -> anyhow::Result<Command> {
    let address = url::Url::parse(&config.database_url)?;
    Ok(postgres_invocation(
        program,
        &address,
        address.path().trim_start_matches('/'),
    ))
}

fn postgres_command_for_database(
    program: &str,
    config: &Config,
    database: &str,
) -> anyhow::Result<Command> {
    let address = url::Url::parse(&config.database_url)?;
    Ok(postgres_invocation(program, &address, database))
}

fn quote_identifier(value: &str) -> String {
    quote_sql(value, '"')
}

fn quote_literal(value: &str) -> String {
    quote_sql(value, '\'')
}

async fn restore_backup_settings(config: &Config, path: &Path) -> anyhow::Result<()> {
    let value: serde_json::Value =
        restore_history::read_bounded_json(path, config.recovery.max_settings_bytes)?;
    if value.get("present").and_then(|value| value.as_bool()) != Some(true) {
        return Ok(());
    }
    panda_api::services::artifact_files::validate_prefix(
        value
            .get("prefix")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )?;
    let json_literal = serde_json::to_string(&value)?.replace('\'', "''");
    let sql = format!(
        r#"
        ALTER TABLE panda.backup_settings
          ADD COLUMN IF NOT EXISTS local_enabled boolean NOT NULL DEFAULT true,
          ADD COLUMN IF NOT EXISTS s3_enabled boolean NOT NULL DEFAULT false;
        WITH settings AS MATERIALIZED (
          SELECT * FROM jsonb_to_record('{json_literal}'::jsonb) AS decoded(
            present boolean, enabled boolean, frequency text, run_at text, timezone text,
            day_of_week smallint, day_of_month smallint, retention_count integer, local_enabled boolean, s3_enabled boolean,
            target_type text, endpoint text, region text, bucket text, prefix text, access_key text
          )
        ), saved AS (
        INSERT INTO panda.backup_settings (
          id, enabled, frequency, run_at, timezone, day_of_week, day_of_month,
          retention_count, local_enabled, s3_enabled, target_type, endpoint, region, bucket, prefix, access_key,
          secret_key_encrypted, updated_at, updated_by
        )
        SELECT TRUE, enabled, frequency, run_at::time, timezone, day_of_week, day_of_month,
          retention_count, COALESCE(local_enabled, TRUE), COALESCE(s3_enabled, FALSE), target_type, endpoint, region, bucket, prefix, access_key,
          NULL, now(), NULL
        FROM settings
        ON CONFLICT (id) DO UPDATE SET
          enabled = EXCLUDED.enabled, frequency = EXCLUDED.frequency, run_at = EXCLUDED.run_at,
          timezone = EXCLUDED.timezone, day_of_week = EXCLUDED.day_of_week,
          day_of_month = EXCLUDED.day_of_month, retention_count = EXCLUDED.retention_count,
          local_enabled = EXCLUDED.local_enabled, s3_enabled = EXCLUDED.s3_enabled,
          target_type = EXCLUDED.target_type, endpoint = EXCLUDED.endpoint,
          region = EXCLUDED.region, bucket = EXCLUDED.bucket, prefix = EXCLUDED.prefix,
          access_key = EXCLUDED.access_key, secret_key_encrypted = NULL,
          updated_at = now(), updated_by = NULL
        RETURNING id)
        INSERT INTO panda.system_config (key, value)
        SELECT 'app_timezone', timezone
        FROM settings
        WHERE timezone IS NOT NULL AND btrim(timezone) <> ''
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value;
    "#
    );
    run_psql(config, &sql).await
}

struct ArtworkActivation {
    destination: PathBuf,
    previous: PathBuf,
}

async fn activate_artwork(
    config: &Config,
    source: &Path,
    job_id: Uuid,
) -> anyhow::Result<ArtworkActivation> {
    let activation = ArtworkActivation {
        destination: PathBuf::from(&config.vehicle_image_cache_dir),
        previous: Path::new(&config.vehicle_image_cache_dir)
            .parent()
            .context("invalid artwork path")?
            .join(format!(".vehicle-images-before-{job_id}")),
    };
    let parent = activation
        .destination
        .parent()
        .context("invalid artwork path")?;
    fs::create_dir_all(parent).await?;
    let incoming = parent.join(format!(".vehicle-images-restore-{job_id}"));
    ArtworkStep::Remove(&incoming).execute().await?;
    copy_directory(source, &incoming).await?;
    for operation in [
        ArtworkStep::MoveIfPresent(&activation.destination, &activation.previous),
        ArtworkStep::Move(&incoming, &activation.destination),
    ] {
        operation.execute().await?;
    }
    Ok(activation)
}

async fn rollback_artwork(activation: &ArtworkActivation) -> anyhow::Result<()> {
    for operation in [
        ArtworkStep::Remove(&activation.destination),
        ArtworkStep::MoveIfPresent(&activation.previous, &activation.destination),
    ] {
        operation.execute().await?;
    }
    Ok(())
}

async fn finalize_artwork(activation: &ArtworkActivation) {
    let _ = ArtworkStep::Remove(&activation.previous).execute().await;
}

async fn copy_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let source = source.to_owned();
    let destination = destination.to_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&destination)?;
        let mut pending = vec![(source, destination, true)];
        while let Some((input, output, root)) = pending.pop() {
            let metadata = if root {
                std::fs::metadata(&input)?
            } else {
                std::fs::symlink_metadata(&input)?
            };
            if metadata.is_dir() {
                std::fs::create_dir_all(&output)?;
                for entry in std::fs::read_dir(input)? {
                    let entry = entry?;
                    pending.push((entry.path(), output.join(entry.file_name()), false));
                }
            } else if metadata.is_file() {
                std::fs::copy(input, output)?;
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await?
}

async fn wait_for_api_health(config: &Config) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let endpoint = format!("http://127.0.0.1:{}/health", config.port);
    let result = retry_operation(180, Duration::from_secs(1), || async {
        match client.get(&endpoint).send().await {
            Ok(response) if response.status().is_success() => Ok(()),
            _ => Err(()),
        }
    })
    .await;
    result.map_err(|_| {
        anyhow::anyhow!("restored application did not become healthy within 180 seconds")
    })
}

async fn verify_api_state(config: &Config) -> anyhow::Result<()> {
    wait_for_api_health(config).await?;
    let response = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/v1/auth/setup", config.port))
        .send()
        .await?;
    let status = response.status();
    anyhow::ensure!(
        status.is_success(),
        "restored application setup-state check failed with {status}"
    );
    let payload = response.json::<serde_json::Value>().await?;
    match payload.get("setup_required") {
        Some(serde_json::Value::Bool(false)) => Ok(()),
        _ => anyhow::bail!("restored application unexpectedly requires owner setup"),
    }
}

fn credential_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, StatusCode> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= MAX_AUTH_HEADER_BYTES)
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn postgres_invocation(program: &str, url: &url::Url, database: &str) -> Command {
    let mut command = Command::new(program);
    command.kill_on_drop(true);
    let port = url.port().map(|value| value.to_string());
    for (flag, value) in [
        ("host", url.host_str()),
        ("port", port.as_deref()),
        (
            "username",
            (!url.username().is_empty()).then_some(url.username()),
        ),
        ("dbname", (!database.is_empty()).then_some(database)),
    ] {
        if let Some(value) = value {
            command.arg(format!("--{flag}={value}"));
        }
    }
    if let Some(password) = url.password() {
        command.env("PGPASSWORD", password);
    }
    command
}

fn checked_output(output: std::process::Output, failure: &str) -> anyhow::Result<Vec<u8>> {
    match output.status.success() {
        true => Ok(output.stdout),
        false => anyhow::bail!(
            "{failure}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

fn quote_sql(value: &str, delimiter: char) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push(delimiter);
    for character in value.chars() {
        quoted.push(character);
        if character == delimiter {
            quoted.push(character);
        }
    }
    quoted.push(delimiter);
    quoted
}

async fn exchange_databases(
    config: &Config,
    incoming: &str,
    displaced: &str,
) -> anyhow::Result<()> {
    let statements = [
        format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname IN ('panda', {}) AND pid <> pg_backend_pid()", quote_literal(incoming)),
        format!("ALTER DATABASE panda RENAME TO {}", quote_identifier(displaced)),
        format!("ALTER DATABASE {} RENAME TO panda", quote_identifier(incoming)),
    ];
    run_psql_on_database(config, "postgres", &format!("{};", statements.join(";\n"))).await
}

async fn initialize_host_target(pool: &sqlx::PgPool, force: bool) -> anyhow::Result<()> {
    let initialized: bool = sqlx::query_scalar("SELECT to_regclass('panda.users') IS NOT NULL")
        .fetch_one(pool)
        .await?;
    if !initialized {
        let mut connection = pool.acquire().await?;
        sqlx::query("SET search_path = public")
            .execute(&mut *connection)
            .await?;
        panda_api::db::migrations::MIGRATOR
            .run_direct(None, &mut *connection, false)
            .await?;
    }
    let populated: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM panda.users)")
        .fetch_one(pool)
        .await?;
    anyhow::ensure!(
        !populated || force,
        "target database contains users; pass --force to replace this installation"
    );
    Ok(())
}

async fn stage_host_package(
    config: &Config,
    package: &Path,
    job_id: Uuid,
    staging: &Path,
    expected: &str,
) -> anyhow::Result<PathBuf> {
    ensure_recovery_free_space(staging, config.recovery.min_free_bytes)?;
    let staged = stage_recovery_package(config, job_id, true, package).await?;
    ensure_recovery_free_space(staging, config.recovery.min_free_bytes)?;
    let validated =
        backups::validate_recovery_package_with_limits(&staged, &config.recovery).await?;
    match validated.checksum_sha256 == expected {
        true => Ok(staged),
        false => {
            let _ = fs::remove_dir_all(staging).await;
            anyhow::bail!("recovery package changed while it was being staged");
        }
    }
}

fn verify_candidate_plan(
    plan: Option<&restore_compatibility::RestorePlan>,
    prepared: &PreparedDatabase,
) -> anyhow::Result<()> {
    let planned_transforms: Vec<_> = plan
        .into_iter()
        .flat_map(|plan| &plan.transforms)
        .map(|transform| transform.id.as_str())
        .collect();
    let applied_transforms: Vec<_> = prepared
        .validation_report
        .applied_transforms
        .iter()
        .map(|transform| transform.id.as_str())
        .collect();
    if planned_transforms != applied_transforms {
        anyhow::bail!("candidate compatibility transforms changed after preflight: planned={planned_transforms:?}, actual={applied_transforms:?}");
    }
    let expected = plan
        .and_then(|plan| plan.target.schema_fingerprint.as_deref())
        .unwrap_or_default();
    anyhow::ensure!(
        prepared.validation_report.target_schema.schema_fingerprint == expected,
        "candidate final schema contract differs from the preflight target contract"
    );
    Ok(())
}

async fn capture_host_safety(
    config: &Config,
    current: &sqlx::PgPool,
    candidate_database: &str,
) -> anyhow::Result<()> {
    backups::run_backup_now(current, config, None, backups::BackupRunTrigger::PreRestore).await?;
    backups::reconcile_local_catalog(current, config).await?;
    let history = restore_jobs::snapshot_catalog(current).await?;
    let candidate = config_with_database(config, candidate_database)?;
    let pool = panda_api::db::pool::create_pool(&candidate.database_url).await?;
    let result = restore_jobs::merge_catalog_snapshot(&pool, &history).await;
    if result.is_ok() {
        pool.close().await;
    }
    result.map_err(Into::into)
}

struct CandidateCredentials {
    archive: Config,
    role: String,
    password: String,
}

impl CandidateCredentials {
    fn new(candidate: &Config, job_id: Uuid) -> anyhow::Result<Self> {
        let role = format!("panda_restore_owner_{}", job_id.simple());
        let mut entropy = [0; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut entropy);
        let password = hex::encode(entropy);
        let mut url = url::Url::parse(&candidate.database_url)?;
        url.set_username(&role)
            .map_err(|_| anyhow::anyhow!("invalid restore role"))?;
        url.set_password(Some(&password))
            .map_err(|_| anyhow::anyhow!("invalid restore password"))?;
        Ok(Self {
            archive: Config {
                database_url: url.into(),
                ..candidate.clone()
            },
            role,
            password,
        })
    }

    async fn provision(
        &self,
        candidate: &Config,
        database: &str,
        deadline_seconds: u64,
    ) -> anyhow::Result<()> {
        let pool = panda_api::db::pool::create_pool(&candidate.database_url).await?;
        let role = quote_identifier(&self.role);
        let expiration = Utc::now() + chrono::Duration::seconds(deadline_seconds as i64);
        let creation = format!("CREATE ROLE {role} LOGIN PASSWORD {} VALID UNTIL {} NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 4", quote_literal(&self.password), quote_literal(&expiration.to_rfc3339()));
        sqlx::query(sqlx::AssertSqlSafe(creation))
            .execute(&pool)
            .await?;
        let database = quote_identifier(database);
        let grants = [
            format!("REVOKE CONNECT ON DATABASE {database} FROM PUBLIC"),
            format!("GRANT CONNECT, CREATE, TEMPORARY ON DATABASE {database} TO {role}"),
            format!("GRANT USAGE, CREATE ON SCHEMA public, panda TO {role}"),
            format!("ALTER SCHEMA panda OWNER TO {role}"),
        ];
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{};", grants.join(";"))))
            .execute(&pool)
            .await?;
        for (schema, create) in [
            ("_timescaledb_catalog", false),
            ("_timescaledb_config", false),
            ("_timescaledb_internal", true),
        ] {
            if create {
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                    "GRANT CREATE ON SCHEMA _timescaledb_internal TO {role};"
                )))
                .execute(&pool)
                .await?;
            }
            let schema = quote_identifier(schema);
            let privileges = [
                format!("GRANT USAGE ON SCHEMA {schema} TO {role}"),
                format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO {role}"),
                format!("GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA {schema} TO {role}"),
            ];
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{};", privileges.join(";"))))
                .execute(&pool)
                .await?;
        }
        pool.close().await;
        Ok(())
    }
}

enum RoleCleanup {
    Terminate,
    Revoke,
    Drop,
}
impl RoleCleanup {
    async fn execute(self, pool: &sqlx::PgPool, role: &str) -> anyhow::Result<()> {
        match self {
            Self::Terminate => {
                sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename=$1 AND pid <> pg_backend_pid()")
                    .bind(role).execute(pool).await?;
            }
            action => {
                let role = quote_identifier(role);
                let statement = match action {
                    Self::Revoke => format!("ALTER ROLE {role} NOLOGIN PASSWORD NULL"),
                    Self::Drop => format!("DROP ROLE {role}"),
                    Self::Terminate => unreachable!(),
                };
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }
}

async fn retry_operation<T, E, F, Future>(
    attempts: u32,
    pause: Duration,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Future,
    Future: std::future::Future<Output = Result<T, E>>,
{
    let mut remaining = attempts.max(1);
    loop {
        let result = operation().await;
        if result.is_ok() {
            return result;
        }
        sleep(pause).await;
        remaining -= 1;
        if remaining == 0 {
            return result;
        }
    }
}

enum ArtworkStep<'a> {
    Remove(&'a Path),
    MoveIfPresent(&'a Path, &'a Path),
    Move(&'a Path, &'a Path),
}
impl ArtworkStep<'_> {
    async fn execute(self) -> anyhow::Result<()> {
        match self {
            Self::Remove(path) if fs::try_exists(path).await.unwrap_or(false) => {
                fs::remove_dir_all(path).await?
            }
            Self::MoveIfPresent(from, to) if fs::try_exists(from).await.unwrap_or(false) => {
                fs::rename(from, to).await?
            }
            Self::Move(from, to) => fs::rename(from, to).await?,
            _ => {}
        }
        Ok(())
    }
}

async fn retry_admitted(config: &Config, job: &restore_jobs::RestoreJob) -> bool {
    match (&job.phase, job.retryable, job.automatic_retry_count) {
        (RestorePhase::Completed | RestorePhase::Queued, _, _) | (_, false, _) | (_, _, 2..) => {
            false
        }
        _ => {
            let staging = Path::new(&config.backup_artifact_dir)
                .join(".restore-staging")
                .join(job.id.to_string());
            for file in ["manifest.json", "database.dump"] {
                if !fs::try_exists(staging.join(file)).await.unwrap_or(false) {
                    return false;
                }
            }
            true
        }
    }
}

#[cfg(test)]
#[path = "__tests__/restore_execution_contracts.rs"]
mod execution_contracts;

#[cfg(test)]
mod security_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a disposable Timescale server and PostgreSQL client tools"]
    async fn restore_deadline_cleans_a_role_and_candidate_while_pg_restore_is_executing() {
        let mut config = Config::from_env().expect("isolated fixture configuration");
        config.recovery.restore_deadline_seconds = 10;
        let files = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let source = format!("panda_timeout_source_{}", id.simple());
        create_database(&config, &source).await.unwrap();
        let source_config = config_with_database(&config, &source).unwrap();
        // pg_dump records the slow replacement function and index definition;
        // pg_restore must execute the index expression against the copied row.
        run_psql(&source_config, "CREATE SCHEMA panda; CREATE FUNCTION public.slow_restore_index(integer) RETURNS integer LANGUAGE SQL IMMUTABLE AS 'SELECT $1'; CREATE TABLE panda.timeout_probe(id integer); INSERT INTO panda.timeout_probe VALUES (1); CREATE INDEX timeout_probe_idx ON panda.timeout_probe(public.slow_restore_index(id)); CREATE OR REPLACE FUNCTION public.slow_restore_index(integer) RETURNS integer LANGUAGE SQL IMMUTABLE AS 'SELECT $1 FROM pg_sleep(30)';").await.unwrap();
        let dump = files.path().join("database.dump");
        let dumped = postgres_command("pg_dump", &source_config)
            .unwrap()
            .arg("--format=custom")
            .arg(format!("--file={}", dump.display()))
            .output()
            .await
            .unwrap();
        assert!(dumped.status.success(), "fixture dump failed");
        let job_config = config.clone();
        let settings = files.path().join("backup-settings.json");
        fs::write(&settings, b"{}").await.unwrap();
        let worker = tokio::spawn(async move {
            let manifest = json!({});
            with_restore_deadline(
                &job_config,
                id,
                prepare_restore_database(&job_config, id, &dump, &manifest, &settings, None, None),
            )
            .await
        });
        let admin = panda_api::db::pool::create_pool(&config.database_url)
            .await
            .unwrap();
        let role = format!("panda_restore_owner_{}", id.simple());
        tokio::time::timeout(Duration::from_secs(9), async {
            loop {
                let executing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE usename=$1 AND state='active' AND wait_event='PgSleep' AND query LIKE '%CREATE INDEX%')")
                    .bind(&role).fetch_one(&admin).await.unwrap();
                if executing { break; }
                sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("actual pg_restore reached its sleeping archive-defined index");
        let login: bool = sqlx::query_scalar("SELECT rolcanlogin FROM pg_roles WHERE rolname=$1")
            .bind(&role)
            .fetch_one(&admin)
            .await
            .unwrap();
        assert!(login, "the temporary LOGIN exists before timeout");
        let error = worker
            .await
            .unwrap()
            .err()
            .expect("restore deadline must expire");
        assert!(error.to_string().contains("deadline"), "{error}");
        assert!(
            !database_exists(&config, &format!("panda_restore_{}", id.simple()))
                .await
                .unwrap()
        );
        let remaining: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=$1)")
                .bind(&role)
                .fetch_one(&admin)
                .await
                .unwrap();
        assert!(!remaining, "timed-out candidate owner and password removed");
        let sessions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE usename=$1")
                .bind(&role)
                .fetch_one(&admin)
                .await
                .unwrap();
        assert_eq!(sessions, 0, "archive execution backend terminated");
        admin.close().await;
        drop_database(&config, &source).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a disposable Timescale server and PANDA_TEST_RESTORE_PACKAGE"]
    async fn historical_restore_uses_an_unprivileged_nonlogin_object_owner() {
        let package =
            PathBuf::from(std::env::var("PANDA_TEST_RESTORE_PACKAGE").expect("fixture package"));
        let mut config = Config::from_env().expect("isolated fixture configuration");
        let temp = tempfile::tempdir().unwrap();
        config.backup_artifact_dir = temp.path().to_string_lossy().into_owned();
        let staging = temp.path().join("extracted");
        let package = backups::validate_recovery_package_with_limits(&package, &config.recovery)
            .await
            .unwrap();
        let path = PathBuf::from(std::env::var("PANDA_TEST_RESTORE_PACKAGE").unwrap());
        extract_package(&config, &path, &staging).await.unwrap();
        let id = Uuid::new_v4();
        let prepared = prepare_restore_database(
            &config,
            id,
            &staging.join("database.dump"),
            &package.manifest,
            &staging.join("backup-settings.json"),
            Some(&staging.join("operational-history.json")),
            None,
        )
        .await
        .expect("historical candidate restore");
        let admin = panda_api::db::pool::create_pool(&config.database_url)
            .await
            .unwrap();
        let role = format!("panda_restore_owner_{}", id.simple());
        let attributes: (bool, bool, bool, bool, bool, Option<String>) = sqlx::query_as("SELECT rolsuper, rolcreatedb, rolcreaterole, rolbypassrls, rolcanlogin, rolpassword FROM pg_authid WHERE rolname=$1").bind(&role).fetch_one(&admin).await.unwrap();
        assert_eq!(attributes, (false, false, false, false, false, None));
        let candidate = config_with_database(&config, &prepared.candidate_database).unwrap();
        let pool = panda_api::db::pool::create_pool(&candidate.database_url)
            .await
            .unwrap();
        for table in restore_policy::PROTECTED_TABLES {
            if *table == "authentication_settings"
                || *table == "backup_settings"
                || *table == "backup_runs"
                || *table == "backup_artifacts"
                || *table == "backup_restore_requests"
                || *table == "system_config"
            {
                continue;
            }
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM panda.\"{table}\""
            )))
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(count, 0, "protected table {table}");
        }
        let auth: (bool, bool, Option<Vec<u8>>) = sqlx::query_as("SELECT oidc_enabled, password_login_enabled, client_secret_encrypted FROM panda.authentication_settings").fetch_one(&pool).await.unwrap();
        assert_eq!(auth, (false, true, None));
        pool.close().await;
        // Re-enable only this fixture's owner to test actual server permission
        // denial, then remove the disposable candidate and role.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER ROLE {} LOGIN PASSWORD 'isolated-role-denial-fixture'",
            quote_identifier(&role)
        )))
        .execute(&admin)
        .await
        .unwrap();
        let mut restricted_url = url::Url::parse(&candidate.database_url).unwrap();
        restricted_url.set_username(&role).unwrap();
        restricted_url
            .set_password(Some("isolated-role-denial-fixture"))
            .unwrap();
        let restricted = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(restricted_url.as_str())
            .await
            .unwrap();
        for sql in [
            "SELECT pg_read_file('/etc/passwd')",
            "COPY (SELECT 1) TO PROGRAM 'echo fixture'",
            "CREATE DATABASE panda_security_denial_probe",
        ] {
            let error = sqlx::query(sql)
                .execute(&restricted)
                .await
                .expect_err("privileged operation must be denied");
            assert_eq!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("42501")
            );
        }
        restricted.close().await;
        admin.close().await;
        drop_database(&config, &prepared.candidate_database)
            .await
            .unwrap();
        let admin = panda_api::db::pool::create_pool(&config.database_url)
            .await
            .unwrap();
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=$1)")
                .bind(&role)
                .fetch_one(&admin)
                .await
                .unwrap();
        assert!(!exists, "orphan restore role removed");
        admin.close().await;
        let failed_id = Uuid::new_v4();
        std::env::set_var("PANDA_RESTORE_FAULT_PHASE", "dump_restored");
        let failed = prepare_restore_database(
            &config,
            failed_id,
            &staging.join("database.dump"),
            &package.manifest,
            &staging.join("backup-settings.json"),
            Some(&staging.join("operational-history.json")),
            None,
        )
        .await;
        std::env::remove_var("PANDA_RESTORE_FAULT_PHASE");
        assert!(failed.is_err(), "injected candidate failure");
        assert!(
            !database_exists(&config, &format!("panda_restore_{}", failed_id.simple()))
                .await
                .unwrap()
        );
        let admin = panda_api::db::pool::create_pool(&config.database_url)
            .await
            .unwrap();
        let orphan: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=$1)")
                .bind(format!("panda_restore_owner_{}", failed_id.simple()))
                .fetch_one(&admin)
                .await
                .unwrap();
        assert!(
            !orphan,
            "failed preparation removes candidate and credentials"
        );
        admin.close().await;
    }
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn retries_keep_the_last_error_and_the_final_failure_pause() {
        let started = tokio::time::Instant::now();
        let mut calls = 0;
        let result = retry_operation(3, Duration::from_secs(1), || {
            calls += 1;
            std::future::ready(Err::<(), _>(calls))
        })
        .await;
        assert_eq!(result, Err(3));
        assert_eq!(calls, 3);
        assert_eq!(started.elapsed(), Duration::from_secs(3));
        let result = retry_operation(0, Duration::from_secs(1), || {
            std::future::ready(Ok::<_, ()>(7))
        })
        .await;
        assert_eq!(result, Ok(7));
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn artwork_follows_a_root_link_and_skips_nested_links() {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("image"), b"synthetic artwork").unwrap();
        std::os::unix::fs::symlink("image", source.join("nested-link")).unwrap();
        let root_link = fixture.path().join("root-link");
        std::os::unix::fs::symlink(&source, &root_link).unwrap();
        let destination = fixture.path().join("destination");
        copy_directory(&root_link, &destination).await.unwrap();
        assert_eq!(
            std::fs::read(destination.join("image")).unwrap(),
            b"synthetic artwork"
        );
        assert!(!destination.join("nested-link").exists());
    }
}

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, Response, StatusCode},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use tokio::{
    fs,
    io::AsyncWriteExt,
    time::{Duration as TokioDuration, Instant},
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    errors::AppError,
    ingestion::session_store::encrypt_json,
    middleware::auth::{AppState, AuthUser},
    services::{
        app_settings, backups as backup_service, restore_compatibility, restore_jobs, s3_backups,
    },
};

use crate::services::backups::calendar;
#[path = "backups/import.rs"]
mod import;
#[path = "backups/read_runs.rs"]
mod read_runs;
#[path = "backups/settings.rs"]
mod settings;

pub fn router() -> Router<AppState> {
    super::route_table([
        ("/admin/backups", get(get_backup_overview)),
        ("/admin/backups/settings", put(update_backup_settings)),
        ("/admin/backups/run", post(run_backup_now)),
        ("/admin/backups/s3/test", post(test_s3_connection)),
        (
            "/admin/backups/imports/{artifact_id}",
            delete(delete_uploaded_artifact),
        ),
        ("/admin/backups/restores", post(start_restore)),
        ("/admin/backups/restores/preflight", post(preflight_restore)),
        (
            "/admin/backups/artifacts/{artifact_id}/download",
            get(download_backup_artifact),
        ),
        (
            "/admin/backups/restore-requests",
            post(create_restore_request),
        ),
    ])
}

pub fn upload_router() -> Router<AppState> {
    super::route_table([(
        "/admin/backups/imports",
        post(upload_backup_artifact).layer(DefaultBodyLimit::disable()),
    )])
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BackupFrequency {
    Daily,
    Weekly,
    Monthly,
}

impl BackupFrequency {
    fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BackupTargetType {
    S3,
}

impl BackupTargetType {
    fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
        }
    }
}

impl TryFrom<&str> for BackupTargetType {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "s3" => Ok(Self::S3),
            _ => Err(AppError::Validation("target_type must be s3".into())),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BackupRunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl TryFrom<&str> for BackupRunStatus {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            _ => Err(AppError::Validation("backup run status is invalid".into())),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum BackupRunTrigger {
    Manual,
    Scheduled,
    Restore,
    Upload,
    PreRestore,
}

impl TryFrom<&str> for BackupRunTrigger {
    type Error = AppError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "manual" => Ok(Self::Manual),
            "scheduled" => Ok(Self::Scheduled),
            "restore" => Ok(Self::Restore),
            "upload" => Ok(Self::Upload),
            "pre_restore" => Ok(Self::PreRestore),
            _ => Err(AppError::Validation("backup run trigger is invalid".into())),
        }
    }
}

#[derive(Debug, Serialize)]
struct BackupOverviewResponse {
    settings: BackupSettingsResponse,
    recent_runs: Vec<BackupRunResponse>,
    recent_runs_total: i64,
    recent_runs_page: i64,
    recent_runs_per_page: i64,
    artifacts: Vec<BackupArtifactResponse>,
    restore_requests: Vec<BackupRestoreRequestResponse>,
    latest_successful_run: Option<BackupRunResponse>,
    next_run_at: Option<DateTime<Utc>>,
    runtime_readiness: BackupRuntimeReadinessResponse,
    s3_catalog_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct BackupRuntimeReadinessResponse {
    pg_dump_available: bool,
    run_now_allowed: bool,
    restore_automation_available: bool,
    restore_automation_reason: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct BackupSettingsResponse {
    enabled: bool,
    frequency: BackupFrequency,
    run_at: String,
    timezone: String,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    retention_count: i32,
    local_enabled: bool,
    s3_enabled: bool,
    target_type: BackupTargetType,
    endpoint: String,
    region: Option<String>,
    bucket: String,
    prefix: String,
    access_key: Option<String>,
    has_secret_key: bool,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct BackupRunResponse {
    id: Uuid,
    trigger: BackupRunTrigger,
    status: BackupRunStatus,
    phase: String,
    progress_percent: i16,
    artifact_key: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BackupArtifactResponse {
    id: Uuid,
    run_id: Option<Uuid>,
    storage_type: String,
    file_name: String,
    storage_path: String,
    size_bytes: i64,
    checksum_sha256: String,
    manifest: Value,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BackupRestoreRequestResponse {
    id: Uuid,
    artifact_id: Uuid,
    requested_by: Option<Uuid>,
    status: String,
    confirmation_phrase: String,
    notes: Option<String>,
    error_message: Option<String>,
    requested_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BackupRunExecutionResponse {
    run: BackupRunResponse,
    artifacts: Vec<BackupArtifactResponse>,
}

#[derive(Debug, Deserialize)]
struct UpdateBackupSettingsBody {
    enabled: bool,
    frequency: BackupFrequency,
    run_at: String,
    timezone: String,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    retention_count: i32,
    local_enabled: bool,
    s3_enabled: bool,
    target_type: BackupTargetType,
    endpoint: String,
    region: Option<String>,
    bucket: String,
    prefix: String,
    access_key: Option<String>,
    secret_key: Option<String>,
    clear_secret_key: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateRestoreRequestBody {
    artifact_id: Uuid,
    confirmation_phrase: String,
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartRestoreBody {
    artifact_id: Uuid,
    confirmation_phrase: String,
    notes: Option<String>,
    plan_id: String,
    package_checksum_sha256: String,
}

#[derive(Debug, Deserialize)]
struct PreflightRestoreBody {
    artifact_id: Uuid,
}

#[derive(Debug, Serialize)]
struct RestorePreflightResponse {
    plan: restore_compatibility::RestorePlan,
}

#[derive(Debug, Serialize)]
struct UploadBackupResponse {
    artifact: BackupArtifactResponse,
}

#[derive(Debug, Serialize)]
struct StartRestoreResponse {
    job: restore_jobs::RestoreJobPublic,
    capability_token: String,
}

#[derive(Debug, FromRow)]
struct BackupSettingsRow {
    enabled: bool,
    frequency: String,
    run_at: NaiveTime,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    retention_count: i32,
    local_enabled: bool,
    s3_enabled: bool,
    target_type: String,
    endpoint: String,
    region: Option<String>,
    bucket: String,
    prefix: String,
    access_key: Option<String>,
    has_secret_key: bool,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct BackupRunRow {
    id: Uuid,
    trigger: String,
    status: String,
    phase: String,
    progress_percent: i16,
    artifact_key: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct BackupArtifactRow {
    id: Uuid,
    run_id: Option<Uuid>,
    storage_type: String,
    file_name: String,
    storage_path: String,
    size_bytes: i64,
    checksum_sha256: String,
    manifest: Value,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct BackupRestoreRequestRow {
    id: Uuid,
    artifact_id: Uuid,
    requested_by: Option<Uuid>,
    status: String,
    confirmation_phrase: String,
    notes: Option<String>,
    error_message: Option<String>,
    requested_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

async fn get_backup_overview(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<BackupOverviewQuery>,
) -> Result<Json<BackupOverviewResponse>, AppError> {
    require_admin(&state, auth.user_id).await?;
    backup_service::reconcile_local_catalog(&state.pool, &state.config).await?;
    let settings = load_settings(&state).await?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(10).clamp(1, 100);
    let total = count_recent_runs(&state).await?;
    let runs = load_recent_runs(&state, page, per_page).await?;
    let s3_catalog_error = reconcile_s3_catalog(&state)
        .await
        .err()
        .map(|error| error.to_string());
    let (artifacts, requests, latest) = tokio::join!(
        load_artifacts(&state),
        load_restore_requests(&state),
        load_latest_successful_run(&state)
    );
    let artifacts = artifacts?;
    let restore_requests = requests?;
    let latest_successful_run = latest?;
    let next_run_at = compute_next_run(&settings)?;
    let readiness = backup_service::runtime_readiness(&state.config).await;
    let restore = restore_jobs::agent_readiness(&state.config).await;
    let runtime_readiness = BackupRuntimeReadinessResponse {
        pg_dump_available: readiness.pg_dump_available,
        run_now_allowed: readiness.run_now_allowed,
        restore_automation_available: restore.is_ok(),
        restore_automation_reason: restore.err(),
        reason: readiness.reason,
    };
    Ok(Json(BackupOverviewResponse {
        settings,
        recent_runs: runs,
        recent_runs_total: total,
        recent_runs_page: i64::from(page),
        recent_runs_per_page: i64::from(per_page),
        artifacts,
        restore_requests,
        latest_successful_run,
        next_run_at,
        runtime_readiness,
        s3_catalog_error,
    }))
}

async fn reconcile_s3_catalog(state: &AppState) -> Result<(), AppError> {
    let Some(settings) = backup_service::configured_s3_settings(&state.pool, &state.config).await?
    else {
        return Ok(());
    };
    let listed = match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        s3_backups::list(&settings),
    )
    .await
    {
        Err(_) => {
            return Err(AppError::DependencyUnavailable(
                "S3 catalog listing timed out".into(),
            ))
        }
        Ok(result) => {
            result.map_err(|error| AppError::DependencyUnavailable(format!("{error:#}")))?
        }
    };
    let objects = listed.into_iter().map(|row| (serde_json::json!({
        "file_name":row.file_name, "storage_path":s3_backups::locator(&settings.bucket, &row.key),
        "size_bytes":row.size_bytes, "checksum_sha256":row.checksum_sha256.unwrap_or_default(),
        "manifest":{
            "artifact_kind":"recovery_package",
            "format":row.metadata.get("panda-format").cloned().unwrap_or_else(|| "panda-recovery-v1".into()),
            "storage_type":"s3", "object_key":row.key, "restore_availability":"available",
        }
    }), row.created_at)).collect::<Vec<_>>();
    sqlx::query("UPDATE panda.backup_artifacts SET manifest=jsonb_set(manifest,'{restore_availability}','\"unavailable\"'::jsonb,true) WHERE storage_type='s3'")
        .execute(&state.pool).await?;
    for (object, created_at) in objects {
        sqlx::query(
            "INSERT INTO panda.backup_artifacts (run_id,storage_type,file_name,storage_path,size_bytes,checksum_sha256,manifest,created_at)
             SELECT NULL,'s3',v.file_name,v.storage_path,v.size_bytes,v.checksum_sha256,v.manifest,$2
             FROM jsonb_populate_record(NULL::panda.backup_artifacts,$1) v
             ON CONFLICT (storage_path) WHERE storage_type='s3'
             DO UPDATE SET file_name=EXCLUDED.file_name,size_bytes=EXCLUDED.size_bytes,
             checksum_sha256=EXCLUDED.checksum_sha256,manifest=EXCLUDED.manifest")
            .bind(object).bind(created_at).execute(&state.pool).await?;
    }
    Ok(())
}

async fn upload_backup_artifact(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadBackupResponse>), AppError> {
    let user_id = auth.user_id;
    match (RecoveryCommand::Upload {
        auth,
        headers,
        body,
    })
    .execute(state, user_id)
    .await?
    {
        RecoveryResponse::Upload(response) => Ok(*response),
        _ => unreachable!("upload command returns an upload response"),
    }
}

async fn upload_backup_artifact_admitted(
    state: AppState,
    auth: AuthUser,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadBackupResponse>), AppError> {
    import::execute(state, auth, headers, body).await
}

fn ensure_recovery_free_space(path: &std::path::Path, minimum: u64) -> Result<(), AppError> {
    let available = fs2::available_space(path)?;
    if available < minimum {
        return Err(AppError::RecoveryInsufficientStorage(format!(
            "Recovery storage needs at least {} GiB free before continuing.",
            minimum / 1024 / 1024 / 1024
        )));
    }
    Ok(())
}

async fn delete_uploaded_artifact(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(artifact_id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, AppError> {
    require_admin(&state, auth.user_id).await?;
    #[derive(sqlx::FromRow)]
    struct Removal {
        run_id: Option<Uuid>,
        storage_type: String,
        storage_path: String,
        active: bool,
    }
    let removal = sqlx::query_as::<_, Removal>(
        "SELECT a.run_id,a.storage_type,a.storage_path,
         EXISTS(SELECT 1 FROM panda.backup_restore_requests r WHERE r.artifact_id=a.id AND r.status=ANY($2)) AS active
         FROM panda.backup_artifacts a WHERE a.id=$1")
        .bind(artifact_id).bind(&["pending","approved","running"][..])
        .fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    match (removal.storage_type.as_str(), removal.active) {
        ("uploaded", false) => {}
        ("uploaded", true) => {
            return Err(AppError::Conflict(
                "This package has an active restore job.".into(),
            ))
        }
        _ => {
            return Err(AppError::Validation(
                "Only imported recovery packages can be deleted with this action.".into(),
            ))
        }
    }
    crate::services::artifact_files::remove(
        std::path::Path::new(&state.config.backup_artifact_dir),
        std::path::Path::new(&removal.storage_path),
    )?;
    sqlx::query("DELETE FROM panda.backup_runs WHERE id=$1")
        .bind(removal.run_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_restore(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StartRestoreBody>,
) -> Result<(StatusCode, Json<StartRestoreResponse>), AppError> {
    let user_id = auth.user_id;
    match (RecoveryCommand::Restore { auth, body })
        .execute(state, user_id)
        .await?
    {
        RecoveryResponse::Restore(response) => Ok(*response),
        _ => unreachable!("restore command returns a restore response"),
    }
}

async fn start_restore_admitted(
    state: AppState,
    auth: AuthUser,
    body: StartRestoreBody,
) -> Result<(StatusCode, Json<StartRestoreResponse>), AppError> {
    if !restore_jobs::agent_is_ready(&state.config).await {
        return Err(AppError::DependencyUnavailable(
            "Automated restore is unavailable in this runtime; use scripts/restore-backup.mjs."
                .into(),
        ));
    }
    if body.confirmation_phrase.trim() != backup_service::RESTORE_CONFIRMATION_PHRASE {
        return Err(AppError::Validation(format!(
            "Type {} to request a restore",
            backup_service::RESTORE_CONFIRMATION_PHRASE
        )));
    }
    let prepared = PreparedRecovery::inspect(&state, body.artifact_id).await?;
    prepared.confirm(&body)?;
    prepared.submit(state, auth.user_id, body).await
}

async fn preflight_restore(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PreflightRestoreBody>,
) -> Result<Json<RestorePreflightResponse>, AppError> {
    match RecoveryCommand::Preflight(body)
        .execute(state, auth.user_id)
        .await?
    {
        RecoveryResponse::Preflight(response) => Ok(*response),
        _ => unreachable!("preflight command returns a preflight response"),
    }
}

async fn preflight_restore_admitted(
    state: AppState,
    body: PreflightRestoreBody,
) -> Result<Json<RestorePreflightResponse>, AppError> {
    let prepared = PreparedRecovery::inspect(&state, body.artifact_id).await?;
    Ok(Json(RestorePreflightResponse {
        plan: prepared.plan,
    }))
}

async fn cleanup_remote_staging_path(path: &str) {
    let path = std::path::Path::new(path);
    if path
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str())
        == Some(".remote-staging")
    {
        let _ = fs::remove_file(path).await;
    }
}

async fn prepare_and_handoff_restore(state: AppState, job_id: Uuid) -> Result<(), AppError> {
    let mut job = restore_jobs::read(&state.config, job_id).await?;
    enum Checkpoint {
        Preparing,
        Catalog,
    }
    for checkpoint in [Checkpoint::Preparing, Checkpoint::Catalog] {
        if matches!(checkpoint, Checkpoint::Catalog) {
            sqlx::query("UPDATE panda.backup_restore_requests SET status='running',updated_at=now() WHERE id=$1")
                .bind(job.restore_request_id).execute(&state.pool).await?;
            backup_service::reconcile_local_catalog(&state.pool, &state.config).await?;
            job.catalog_snapshot = Some(restore_jobs::snapshot_catalog(&state.pool).await?);
        } else {
            job.phase = restore_jobs::RestorePhase::PreparingCandidate;
            job.progress_percent = 15;
            job.message =
                "Preparing isolated restore candidate while Panda remains available".into();
        }
        job.updated_at = Utc::now();
        restore_jobs::write(&state.config, &job).await?;
    }
    let key = fs::read_to_string(restore_jobs::agent_key_path(&state.config)).await?;
    let endpoint = format!(
        "{}/internal/jobs/{job_id}/execute",
        state.config.restore_agent_url.trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let request = client
        .post(endpoint)
        .header("x-panda-agent-key", key.trim())
        .build()
        .map_err(|error| {
            AppError::DependencyUnavailable(format!("Restore supervisor is unavailable: {error}"))
        })?;
    let response = client.execute(request).await.map_err(|error| {
        AppError::DependencyUnavailable(format!("Restore supervisor is unavailable: {error}"))
    })?;
    match response.status() {
        status if status.is_success() => Ok(()),
        status => Err(AppError::DependencyUnavailable(format!(
            "Restore supervisor rejected the job with status {status}."
        ))),
    }
}

fn sanitize_upload_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(180)
        .collect()
}

async fn download_backup_artifact(
    State(state): State<AppState>,
    auth: AuthUser,
    axum::extract::Path(artifact_id): axum::extract::Path<Uuid>,
) -> Result<Response<Body>, AppError> {
    require_admin(&state, auth.user_id).await?;
    let artifact = load_artifact_by_id(&state, artifact_id).await?;
    let origin = ArtifactOrigin::resolve(&state, &artifact).await?;
    let content = origin.body().await?;
    let disposition = HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        artifact.file_name.replace('"', "_")
    ))
    .map_err(|error| AppError::Internal(anyhow::anyhow!("invalid download filename: {error}")))?;
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        ),
        (header::CONTENT_DISPOSITION, disposition),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
    ];
    let mut response = Response::new(content);
    response.headers_mut().extend(headers);
    Ok(response)
}

async fn update_backup_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateBackupSettingsBody>,
) -> Result<Json<BackupSettingsResponse>, AppError> {
    require_admin(&state, auth.user_id).await?;
    settings::update(&state, auth.user_id, body).await.map(Json)
}

struct MaterializedArtifact {
    root: std::path::PathBuf,
    staging_id: Uuid,
    path: String,
    retained: bool,
}

impl MaterializedArtifact {
    fn retain(mut self) {
        self.retained = true;
    }
}

impl Drop for MaterializedArtifact {
    fn drop(&mut self) {
        if !self.retained {
            let _ = crate::services::artifact_files::remove_staged(&self.root, self.staging_id);
        }
    }
}

async fn materialize_artifact(
    state: &AppState,
    artifact: &BackupArtifactResponse,
) -> Result<MaterializedArtifact, AppError> {
    let root = std::path::Path::new(&state.config.backup_artifact_dir);
    let staging_id = Uuid::new_v4();
    let mut staged = MaterializedArtifact {
        root: root.to_owned(),
        staging_id,
        path: String::new(),
        retained: false,
    };
    let origin = ArtifactOrigin::resolve(state, artifact).await?;
    let path = origin.stage(root, staging_id, state).await?;
    staged.path = path.to_string_lossy().into_owned();
    let package =
        backup_service::validate_recovery_package_with_limits(&path, &state.config.recovery)
            .await?;
    match artifact.checksum_sha256.as_str() {
        "" => Ok(staged),
        checksum if checksum == package.checksum_sha256 => Ok(staged),
        _ => Err(AppError::Validation(
            "Recovery package checksum does not match its catalog metadata".into(),
        )),
    }
}

async fn resolve_remote_artifact(
    state: &AppState,
    artifact: &BackupArtifactResponse,
) -> Result<(s3_backups::S3Settings, String), AppError> {
    let settings = backup_service::configured_s3_settings(&state.pool, &state.config)
        .await?
        .ok_or_else(|| {
            AppError::Validation(
                "S3 credentials must be configured before accessing this package".into(),
            )
        })?;
    let locator = s3_backups::key_from_locator(&settings.bucket, &artifact.storage_path);
    let key = match locator {
        None => {
            return Err(AppError::Validation(
                "This backup belongs to a different S3 bucket".into(),
            ))
        }
        Some(key) if !s3_backups::key_belongs_to_prefix(&settings.prefix, key) => {
            return Err(AppError::Validation(
                "This backup is outside the configured S3 prefix".into(),
            ))
        }
        Some(key) => key.to_owned(),
    };
    Ok((settings, key))
}

#[derive(Debug, Serialize)]
struct TestS3Response {
    ok: bool,
    message: String,
}

async fn test_s3_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateBackupSettingsBody>,
) -> Result<Json<TestS3Response>, AppError> {
    require_admin(&state, auth.user_id).await?;
    validate_target(
        body.target_type,
        body.local_enabled,
        true,
        &body.endpoint,
        &body.bucket,
    )?;
    crate::services::s3_transport::S3Policy::from_config(&state.config)?
        .validate_endpoint(&body.endpoint)
        .await?;
    let existing = backup_service::configured_s3_settings(&state.pool, &state.config)
        .await
        .ok()
        .flatten();
    let mut credentials = Vec::with_capacity(2);
    for (supplied, saved, name) in [
        (
            body.secret_key.as_deref(),
            existing
                .as_ref()
                .map(|settings| settings.secret_key.as_str()),
            "secret",
        ),
        (
            body.access_key.as_deref(),
            existing
                .as_ref()
                .map(|settings| settings.access_key.as_str()),
            "access",
        ),
    ] {
        let credential = supplied
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(saved)
            .ok_or_else(|| {
                AppError::Validation(format!("Enter or save an S3 {name} key before testing"))
            })?;
        credentials.push(credential.to_owned());
    }
    let secret_key = credentials.remove(0);
    let access_key = credentials.remove(0);
    let settings = s3_backups::S3Settings {
        endpoint: body.endpoint.trim().to_owned(),
        region: body
            .region
            .as_deref()
            .map(str::trim)
            .filter(|region| !region.is_empty())
            .unwrap_or("us-east-1")
            .to_owned(),
        bucket: body.bucket.trim().to_owned(),
        prefix: normalize_prefix(&body.prefix)?,
        access_key,
        secret_key,
        policy: crate::services::s3_transport::S3Policy::from_config(&state.config)?,
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        s3_backups::test_connection(&settings),
    )
    .await
    {
        Err(_) => Err(AppError::DependencyUnavailable(
            "S3 connection test timed out".into(),
        )),
        Ok(Err(error)) => Err(AppError::DependencyUnavailable(format!("{error:#}"))),
        Ok(Ok(())) => Ok(Json(TestS3Response {
            ok: true,
            message: "S3 list/write/read/delete checks passed".into(),
        })),
    }
}

async fn run_backup_now(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<(StatusCode, Json<BackupRunExecutionResponse>), AppError> {
    let id = BackupSubmission::Manual
        .submit(&state, auth.user_id)
        .await?;
    let response = BackupRunExecutionResponse {
        run: load_run_by_id(&state, id).await?,
        artifacts: Vec::new(),
    };
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn create_restore_request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRestoreRequestBody>,
) -> Result<(StatusCode, Json<BackupRestoreRequestResponse>), AppError> {
    let id = BackupSubmission::Request(body)
        .submit(&state, auth.user_id)
        .await?;
    load_restore_request_by_id(&state, id)
        .await
        .map(|request| (StatusCode::CREATED, Json(request)))
}

async fn load_settings(state: &AppState) -> Result<BackupSettingsResponse, AppError> {
    let app_timezone = app_settings::load_app_timezone_name(&state.pool).await?;
    let row = sqlx::query_as::<_, BackupSettingsRow>(
        r#"
        SELECT
            enabled,
            frequency,
            run_at,
            day_of_week,
            day_of_month,
            retention_count,
            local_enabled,
            s3_enabled,
            target_type,
            endpoint,
            region,
            bucket,
            prefix,
            access_key,
            secret_key_encrypted IS NOT NULL AS has_secret_key,
            updated_at
        FROM panda.backup_settings
        WHERE id = TRUE
        "#,
    )
    .fetch_optional(&state.pool)
    .await?;

    match row {
        Some(row) => map_settings_row(row, app_timezone),
        None => Ok(default_settings(app_timezone)),
    }
}

async fn count_recent_runs(state: &AppState) -> Result<i64, AppError> {
    let total = sqlx::query_scalar("SELECT COUNT(*) FROM panda.backup_runs")
        .fetch_one(&state.pool)
        .await?;
    Ok(total)
}

async fn load_recent_runs(
    state: &AppState,
    page: u32,
    per_page: u32,
) -> Result<Vec<BackupRunResponse>, AppError> {
    read_runs::Selection::Page { page, per_page }
        .many(state)
        .await
}
async fn load_run_by_id(state: &AppState, run_id: Uuid) -> Result<BackupRunResponse, AppError> {
    read_runs::Selection::Id(run_id)
        .optional(state)
        .await?
        .ok_or(AppError::NotFound)
}

async fn load_artifacts(state: &AppState) -> Result<Vec<BackupArtifactResponse>, AppError> {
    Ok(CatalogSelection::Artifacts(None)
        .load::<BackupArtifactRow>(state)
        .await?
        .into_iter()
        .map(map_artifact_row)
        .collect())
}

async fn load_artifact_by_id(
    state: &AppState,
    artifact_id: Uuid,
) -> Result<BackupArtifactResponse, AppError> {
    CatalogSelection::Artifacts(Some(artifact_id))
        .load::<BackupArtifactRow>(state)
        .await?
        .pop()
        .map(map_artifact_row)
        .ok_or(AppError::NotFound)
}

async fn load_restore_requests(
    state: &AppState,
) -> Result<Vec<BackupRestoreRequestResponse>, AppError> {
    Ok(CatalogSelection::Requests(None)
        .load::<BackupRestoreRequestRow>(state)
        .await?
        .into_iter()
        .map(map_restore_request_row)
        .collect())
}

async fn load_restore_request_by_id(
    state: &AppState,
    request_id: Uuid,
) -> Result<BackupRestoreRequestResponse, AppError> {
    CatalogSelection::Requests(Some(request_id))
        .load::<BackupRestoreRequestRow>(state)
        .await?
        .pop()
        .map(map_restore_request_row)
        .ok_or(AppError::NotFound)
}

async fn load_latest_successful_run(
    state: &AppState,
) -> Result<Option<BackupRunResponse>, AppError> {
    read_runs::Selection::LatestSuccess.optional(state).await
}

fn map_settings_row(
    row: BackupSettingsRow,
    app_timezone: String,
) -> Result<BackupSettingsResponse, AppError> {
    Ok(BackupSettingsResponse {
        enabled: row.enabled,
        frequency: BackupFrequency::try_from(row.frequency.as_str())?,
        run_at: row.run_at.format("%H:%M").to_string(),
        timezone: app_timezone,
        day_of_week: row.day_of_week,
        day_of_month: row.day_of_month,
        retention_count: row.retention_count,
        local_enabled: row.local_enabled,
        s3_enabled: row.s3_enabled,
        target_type: BackupTargetType::try_from(row.target_type.as_str())?,
        endpoint: row.endpoint,
        region: row.region,
        bucket: row.bucket,
        prefix: row.prefix,
        access_key: row.access_key,
        has_secret_key: row.has_secret_key,
        updated_at: Some(row.updated_at),
    })
}

fn map_run_row(row: BackupRunRow) -> Result<BackupRunResponse, AppError> {
    Ok(BackupRunResponse {
        id: row.id,
        trigger: BackupRunTrigger::try_from(row.trigger.as_str())?,
        status: BackupRunStatus::try_from(row.status.as_str())?,
        phase: row.phase,
        progress_percent: row.progress_percent,
        artifact_key: row.artifact_key,
        started_at: row.started_at,
        completed_at: row.completed_at,
        error_message: row.error_message,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn map_artifact_row(row: BackupArtifactRow) -> BackupArtifactResponse {
    BackupArtifactResponse {
        id: row.id,
        run_id: row.run_id,
        storage_type: row.storage_type,
        file_name: row.file_name,
        storage_path: row.storage_path,
        size_bytes: row.size_bytes,
        checksum_sha256: row.checksum_sha256,
        manifest: row.manifest,
        created_at: row.created_at,
    }
}

fn map_restore_request_row(row: BackupRestoreRequestRow) -> BackupRestoreRequestResponse {
    BackupRestoreRequestResponse {
        id: row.id,
        artifact_id: row.artifact_id,
        requested_by: row.requested_by,
        status: row.status,
        confirmation_phrase: row.confirmation_phrase,
        notes: row.notes,
        error_message: row.error_message,
        requested_at: row.requested_at,
        updated_at: row.updated_at,
    }
}

fn default_settings(timezone: String) -> BackupSettingsResponse {
    BackupSettingsResponse {
        enabled: false,
        frequency: BackupFrequency::Weekly,
        run_at: "03:00".into(),
        timezone,
        day_of_week: Some(0),
        day_of_month: Some(1),
        retention_count: 8,
        local_enabled: true,
        s3_enabled: false,
        target_type: BackupTargetType::S3,
        endpoint: String::new(),
        region: None,
        bucket: String::new(),
        prefix: "panda".into(),
        access_key: None,
        has_secret_key: false,
        updated_at: None,
    }
}

fn parse_run_time(value: &str) -> Result<NaiveTime, AppError> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M")
        .map_err(|_| AppError::Validation("run_at must use HH:MM 24-hour time".into()))
}

fn parse_timezone(value: &str) -> Result<Tz, AppError> {
    value
        .trim()
        .parse::<Tz>()
        .map_err(|_| AppError::Validation("timezone must be a valid IANA timezone".into()))
}

fn validate_schedule(
    frequency: &BackupFrequency,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
) -> Result<(), AppError> {
    let (day, interval, message) = match frequency {
        BackupFrequency::Daily => return Ok(()),
        BackupFrequency::Weekly => (
            day_of_week,
            0..=6,
            "weekly backups require day_of_week between 0 and 6",
        ),
        BackupFrequency::Monthly => (
            day_of_month,
            1..=31,
            "monthly backups require day_of_month between 1 and 31",
        ),
    };
    day.filter(|day| interval.contains(day))
        .map(|_| ())
        .ok_or_else(|| AppError::Validation(message.into()))
}

fn validate_target(
    target_type: BackupTargetType,
    local_enabled: bool,
    s3_enabled: bool,
    endpoint: &str,
    bucket: &str,
) -> Result<(), AppError> {
    let invalid = [
        (
            target_type != BackupTargetType::S3,
            "target_type must be s3",
        ),
        (
            !local_enabled && !s3_enabled,
            "Enable at least one backup destination",
        ),
        (
            s3_enabled && bucket.trim().is_empty(),
            "bucket is required when S3 backups are enabled",
        ),
    ]
    .into_iter()
    .find(|(invalid, _)| *invalid);
    if let Some((_, message)) = invalid {
        return Err(AppError::Validation(message.into()));
    }
    match (s3_enabled, endpoint.trim()) {
        (true, endpoint) if !endpoint.is_empty() => {
            let parsed = url::Url::parse(endpoint).map_err(|_| {
                AppError::Validation("S3 endpoint must be a valid HTTP or HTTPS URL".into())
            })?;
            if !["http", "https"].contains(&parsed.scheme()) {
                return Err(AppError::Validation(
                    "S3 endpoint must use HTTP or HTTPS".into(),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn normalize_prefix(value: &str) -> Result<String, AppError> {
    let prefix = crate::services::artifact_files::validate_prefix(value)?;
    Ok(if prefix.is_empty() {
        "panda".into()
    } else {
        prefix
    })
}

fn encrypt_secret(age_key: &str, secret_key: Option<&str>) -> Result<Option<Vec<u8>>, AppError> {
    let Some(secret_key) = secret_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let identity = age_key.parse::<age::x25519::Identity>().map_err(|_| {
        AppError::Internal(anyhow::anyhow!("invalid age key for backup secret storage"))
    })?;
    let ciphertext = encrypt_json(&secret_key.to_string(), &identity)?;
    Ok(Some(ciphertext))
}

fn compute_next_run(settings: &BackupSettingsResponse) -> Result<Option<DateTime<Utc>>, AppError> {
    if !settings.enabled {
        return Ok(None);
    }
    let timezone = parse_timezone(&settings.timezone)?;
    let at = parse_run_time(&settings.run_at)?;
    let cadence = match settings.frequency {
        BackupFrequency::Daily => calendar::Cadence::Daily,
        BackupFrequency::Weekly => calendar::Cadence::Weekly(settings.day_of_week.unwrap_or(0)),
        BackupFrequency::Monthly => calendar::Cadence::Monthly(settings.day_of_month.unwrap_or(1)),
    };
    Ok(Some(
        calendar::occurrence(Utc::now().with_timezone(&timezone), at, cadence, true)
            .with_timezone(&Utc),
    ))
}

#[cfg(test)]
fn next_daily_run(now_local: chrono::DateTime<Tz>, run_at: NaiveTime) -> chrono::DateTime<Tz> {
    calendar::occurrence(now_local, run_at, calendar::Cadence::Daily, true)
}
#[cfg(test)]
fn next_weekly_run(
    now_local: chrono::DateTime<Tz>,
    run_at: NaiveTime,
    day_of_week: i16,
) -> chrono::DateTime<Tz> {
    calendar::occurrence(
        now_local,
        run_at,
        calendar::Cadence::Weekly(day_of_week),
        true,
    )
}
#[cfg(test)]
fn next_monthly_run(
    now_local: chrono::DateTime<Tz>,
    run_at: NaiveTime,
    day_of_month: i16,
) -> chrono::DateTime<Tz> {
    calendar::occurrence(
        now_local,
        run_at,
        calendar::Cadence::Monthly(day_of_month),
        true,
    )
}

async fn require_admin(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    super::installation_admin(&state.pool, user_id)
        .await?
        .then_some(())
        .ok_or(AppError::Forbidden)
}
#[derive(Debug, Deserialize)]
struct BackupOverviewQuery {
    page: Option<u32>,
    per_page: Option<u32>,
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[tokio::test]
    async fn materialized_preflight_cleans_published_files_unless_handed_to_a_durable_job() {
        let root = tempfile::tempdir().unwrap();
        let limits = crate::config::RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        };
        for retained in [false, true] {
            let id = Uuid::new_v4();
            let mut input: &[u8] = b"bounded published package";
            let path =
                crate::services::artifact_files::stage_reader(root.path(), id, &mut input, &limits)
                    .await
                    .unwrap();
            let artifact = MaterializedArtifact {
                root: root.path().to_owned(),
                staging_id: id,
                path: path.to_string_lossy().into_owned(),
                retained: false,
            };
            if retained {
                artifact.retain();
            } else {
                drop(artifact);
            }
            assert_eq!(path.exists(), retained);
            if retained {
                crate::services::artifact_files::remove_staged(root.path(), id).unwrap();
            }
        }
        assert_eq!(
            std::fs::read_dir(root.path().join(".remote-staging"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[cfg(test)]
#[path = "__tests__/backup_workflows.rs"]
mod workflow_contracts;

enum ArtifactOrigin<'a> {
    Local {
        root: &'a std::path::Path,
        path: &'a std::path::Path,
    },
    Remote {
        settings: Box<s3_backups::S3Settings>,
        key: String,
    },
}

impl<'a> ArtifactOrigin<'a> {
    async fn resolve(
        state: &'a AppState,
        artifact: &'a BackupArtifactResponse,
    ) -> Result<Self, AppError> {
        match artifact.storage_type.as_str() {
            "s3" => {
                let (settings, key) = resolve_remote_artifact(state, artifact).await?;
                Ok(Self::Remote {
                    settings: Box::new(settings),
                    key,
                })
            }
            _ => Ok(Self::Local {
                root: std::path::Path::new(&state.config.backup_artifact_dir),
                path: std::path::Path::new(&artifact.storage_path),
            }),
        }
    }

    async fn body(self) -> Result<Body, AppError> {
        let reader: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>> = match self {
            Self::Local { root, path } => Box::pin(fs::File::from_std(
                crate::services::artifact_files::open(root, path)?,
            )),
            Self::Remote { settings, key } => Box::pin(
                s3_backups::download_stream(&settings, &key)
                    .await
                    .map_err(|error| AppError::DependencyUnavailable(format!("{error:#}")))?
                    .into_async_read(),
            ),
        };
        Ok(Body::from_stream(ReaderStream::new(reader)))
    }

    async fn stage(
        self,
        root: &std::path::Path,
        id: Uuid,
        state: &AppState,
    ) -> Result<std::path::PathBuf, AppError> {
        match self {
            Self::Local {
                root: source_root,
                path,
            } => {
                let mut source =
                    fs::File::from_std(crate::services::artifact_files::open(source_root, path)?);
                crate::services::artifact_files::stage_reader(
                    root,
                    id,
                    &mut source,
                    &state.config.recovery,
                )
                .await
            }
            Self::Remote { settings, key } => {
                s3_backups::download(&settings, &key, root, id, &state.config.recovery)
                    .await
                    .map_err(|error| AppError::DependencyUnavailable(format!("{error:#}")))
            }
        }
    }
}

enum RecoveryCommand {
    Upload {
        auth: AuthUser,
        headers: HeaderMap,
        body: Body,
    },
    Preflight(PreflightRestoreBody),
    Restore {
        auth: AuthUser,
        body: StartRestoreBody,
    },
}
enum RecoveryResponse {
    Upload(Box<(StatusCode, Json<UploadBackupResponse>)>),
    Preflight(Box<Json<RestorePreflightResponse>>),
    Restore(Box<(StatusCode, Json<StartRestoreResponse>)>),
}
impl RecoveryCommand {
    async fn execute(self, state: AppState, user_id: Uuid) -> Result<RecoveryResponse, AppError> {
        require_admin(&state, user_id).await?;
        backup_service::run_recovery_operation(state.pool.clone(), async move {
            match self {
                Self::Upload {
                    auth,
                    headers,
                    body,
                } => upload_backup_artifact_admitted(state, auth, headers, body)
                    .await
                    .map(Box::new)
                    .map(RecoveryResponse::Upload),
                Self::Preflight(body) => preflight_restore_admitted(state, body)
                    .await
                    .map(Box::new)
                    .map(RecoveryResponse::Preflight),
                Self::Restore { auth, body } => start_restore_admitted(state, auth, body)
                    .await
                    .map(Box::new)
                    .map(RecoveryResponse::Restore),
            }
        })
        .await
    }
}

async fn finish_restore_handoff(state: AppState, job_id: Uuid, request_id: Uuid) {
    let error = match prepare_and_handoff_restore(state.clone(), job_id).await {
        Ok(()) => return,
        Err(error) => error,
    };
    if let Ok(job) = restore_jobs::read(&state.config, job_id).await {
        cleanup_remote_staging_path(&job.artifact_path).await;
    }
    let _ = restore_jobs::fail(&state.config, job_id, &error).await;
    let _ = sqlx::query(
        "UPDATE panda.backup_restore_requests SET status='failed',error_message=$2,updated_at=now() WHERE id=$1")
        .bind(request_id).bind(error.to_string()).execute(&state.pool).await;
}

enum CatalogSelection {
    Artifacts(Option<Uuid>),
    Requests(Option<Uuid>),
}
impl CatalogSelection {
    async fn load<T>(self, state: &AppState) -> Result<Vec<T>, AppError>
    where
        T: for<'row> sqlx::FromRow<'row, sqlx::postgres::PgRow> + Send + Unpin,
    {
        let (columns, table, order, limit, id) = match self {
            Self::Artifacts(id) => ("id,run_id,storage_type,file_name,storage_path,size_bytes,checksum_sha256,manifest,created_at",
                "panda.backup_artifacts", "created_at DESC", 200_i64, id),
            Self::Requests(id) => ("id,artifact_id,requested_by,status,confirmation_phrase,notes,error_message,requested_at,updated_at",
                "panda.backup_restore_requests", "requested_at DESC", 10_i64, id),
        };
        let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT ");
        query.push(columns).push(" FROM ").push(table);
        match id {
            Some(id) => {
                query.push(" WHERE id=").push_bind(id);
            }
            None => {
                query
                    .push(" ORDER BY ")
                    .push(order)
                    .push(" LIMIT ")
                    .push_bind(limit);
            }
        }
        query
            .build_query_as::<T>()
            .fetch_all(&state.pool)
            .await
            .map_err(AppError::from)
    }
}

enum BackupSubmission {
    Manual,
    Request(CreateRestoreRequestBody),
}
impl BackupSubmission {
    async fn submit(self, state: &AppState, user: Uuid) -> Result<Uuid, AppError> {
        require_admin(state, user).await?;
        match self {
            Self::Manual => {
                backup_service::start_backup_now(
                    &state.pool,
                    &state.config,
                    Some(user),
                    backup_service::BackupRunTrigger::Manual,
                )
                .await
            }
            Self::Request(body) => {
                backup_service::create_restore_request(
                    &state.pool,
                    body.artifact_id,
                    user,
                    &body.confirmation_phrase,
                    body.notes,
                )
                .await
            }
        }
    }
}

struct PreparedRecovery {
    staged: MaterializedArtifact,
    plan: restore_compatibility::RestorePlan,
}
impl PreparedRecovery {
    async fn inspect(state: &AppState, artifact_id: Uuid) -> Result<Self, AppError> {
        let artifact = load_artifact_by_id(state, artifact_id).await?;
        let staged = materialize_artifact(state, &artifact).await?;
        let path = std::path::Path::new(&staged.path);
        let package =
            backup_service::validate_recovery_package_with_limits(path, &state.config.recovery)
                .await?;
        let inspection = restore_compatibility::inspect_recovery_dump(
            path,
            std::path::Path::new(&state.config.backup_artifact_dir),
            &state.config.recovery,
        )
        .await?;
        let plan = restore_compatibility::plan_restore(
            &package.manifest,
            &package.checksum_sha256,
            &state.pool,
            Some(&inspection),
        )
        .await?;
        Ok(Self { staged, plan })
    }

    fn confirm(&self, request: &StartRestoreBody) -> Result<(), AppError> {
        let plan = &self.plan;
        match (plan.compatible,
               request.plan_id == plan.plan_id && request.package_checksum_sha256 == plan.package_checksum_sha256) {
            (true, true) => Ok(()),
            (true, false) => Err(AppError::Conflict(
                "The recovery package or target schema changed after preflight. Run preflight again.".into(),
            )),
            (false, _) => Err(AppError::Validation(plan.blocking_errors.iter()
                .map(|error| error.message.as_str()).collect::<Vec<_>>().join(" "))),
        }
    }

    async fn submit(
        self,
        state: AppState,
        user_id: Uuid,
        request: StartRestoreBody,
    ) -> Result<(StatusCode, Json<StartRestoreResponse>), AppError> {
        let request_id = backup_service::create_restore_request(
            &state.pool,
            request.artifact_id,
            user_id,
            &request.confirmation_phrase,
            request.notes,
        )
        .await?;
        let (job, capability_token) = restore_jobs::create(
            &state.config,
            request.artifact_id,
            self.staged.path.clone(),
            request_id,
            self.plan,
        )
        .await?;
        let job_id = job.id;
        tokio::spawn(finish_restore_handoff(state, job_id, request_id));
        self.staged.retain();
        Ok((
            StatusCode::ACCEPTED,
            Json(StartRestoreResponse {
                job: job.public(),
                capability_token,
            }),
        ))
    }
}

use super::*;
use crate::services::artifact_files::StagingFile;
use std::path::Path;

struct ImportIntent {
    run: Uuid,
    original_name: String,
}
impl ImportIntent {
    fn admit(headers: &HeaderMap, limit: u64) -> Result<Self, AppError> {
        let length = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if length.is_some_and(|length| length > limit) {
            return Err(too_large());
        }
        let original_name = headers
            .get("x-panda-file-name")
            .and_then(|v| v.to_str().ok())
            .map(sanitize_upload_name)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "external-backup.rma.tar.gz".into());
        if !original_name.ends_with(".rma.tar.gz") {
            return Err(AppError::Validation(
                "Upload a Panda .rma.tar.gz recovery package.".into(),
            ));
        }
        Ok(Self {
            run: Uuid::new_v4(),
            original_name,
        })
    }
}
fn too_large() -> AppError {
    AppError::RecoveryTooLarge("Recovery package exceeds the configured upload limit.".into())
}
fn expired() -> AppError {
    AppError::RecoveryDeadline("Recovery upload exceeded the configured deadline.".into())
}

struct ReceivedImport {
    destination: StagingFile,
    intent: ImportIntent,
    package: backup_service::ValidatedRecoveryPackage,
}
impl ReceivedImport {
    async fn receive(
        state: &AppState,
        intent: ImportIntent,
        destination: StagingFile,
        body: Body,
    ) -> Result<Self, AppError> {
        let root = Path::new(&state.config.backup_artifact_dir);
        let mut file = fs::File::from_std(destination.file.try_clone()?);
        let mut input = body.into_data_stream();
        let deadline = Instant::now()
            + TokioDuration::from_secs(state.config.recovery.upload_deadline_seconds);
        let mut used = 0u64;
        loop {
            let Some(chunk) =
                tokio::time::timeout_at(deadline, futures::StreamExt::next(&mut input))
                    .await
                    .map_err(|_| expired())?
            else {
                break;
            };
            if Instant::now() >= deadline {
                return Err(expired());
            }
            let chunk = chunk.map_err(|error| {
                AppError::Validation(format!("Backup upload was interrupted: {error}"))
            })?;
            used = used.checked_add(chunk.len() as u64).ok_or_else(|| {
                AppError::RecoveryTooLarge("Recovery upload size overflowed.".into())
            })?;
            if used > state.config.recovery.max_upload_bytes {
                return Err(too_large());
            }
            ensure_recovery_free_space(
                root,
                state
                    .config
                    .recovery
                    .min_free_bytes
                    .saturating_add(chunk.len() as u64),
            )?;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);
        progress(state, intent.run, "validating", 90).await?;
        let mut input = destination.file.try_clone()?;
        std::io::Seek::seek(&mut input, std::io::SeekFrom::Start(0))?;
        let package =
            backup_service::validate_open_recovery_package(input, &state.config.recovery).await?;
        Ok(Self {
            destination,
            intent,
            package,
        })
    }
    async fn publish(self, state: &AppState) -> Result<BackupArtifactResponse, AppError> {
        let name = self.destination.file_name().to_owned();
        let path = self.destination.path.to_string_lossy().into_owned();
        self.destination.publish()?;
        let metadata = serde_json::json!({
            "artifact_kind": "recovery_package",
            "format": self.package.manifest.get("format").cloned().unwrap_or(Value::Null),
            "original_file_name": self.intent.original_name,
            "package": self.package.manifest,
        });
        let artifact: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.backup_artifacts (run_id, storage_type, file_name, storage_path, size_bytes, checksum_sha256, manifest)
             VALUES ($1, 'uploaded', $2, $3, $4, $5, $6) RETURNING id",
        ).bind(self.intent.run).bind(name).bind(&path).bind(self.package.size_bytes).bind(self.package.checksum_sha256).bind(metadata)
            .fetch_one(&state.pool).await?;
        sqlx::query(
            "UPDATE panda.backup_runs SET status = 'succeeded', phase = 'completed', progress_percent = 100,
             artifact_key = $2, completed_at = now(), updated_at = now() WHERE id = $1 AND status = 'running'",
        ).bind(self.intent.run).bind(path).execute(&state.pool).await?;
        load_artifact_by_id(state, artifact).await
    }
}
async fn progress(state: &AppState, run: Uuid, phase: &str, percent: i16) -> Result<(), AppError> {
    sqlx::query("UPDATE panda.backup_runs SET phase = $2, progress_percent = $3, updated_at = now() WHERE id = $1 AND status = 'running'")
        .bind(run).bind(phase).bind(percent).execute(&state.pool).await?;
    Ok(())
}

pub(super) async fn execute(
    state: AppState,
    auth: AuthUser,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadBackupResponse>), AppError> {
    let intent = ImportIntent::admit(&headers, state.config.recovery.max_upload_bytes)?;
    let run = intent.run;
    let root = Path::new(&state.config.backup_artifact_dir);
    fs::create_dir_all(root).await?;
    ensure_recovery_free_space(root, state.config.recovery.min_free_bytes)?;
    let destination = StagingFile::upload(root, run)?;
    let path = destination.path.clone();
    let result = async {
        sqlx::query("INSERT INTO panda.backup_runs (id, trigger, status, phase, progress_percent, requested_by, started_at, updated_at) VALUES ($1, 'upload', 'running', 'queued', 0, $2, now(), now())")
            .bind(run).bind(auth.user_id).execute(&state.pool).await?;
        progress(&state, run, "uploading", 10).await?;
        ReceivedImport::receive(&state, intent, destination, body).await?.publish(&state).await
    }.await;
    match result {
        Ok(artifact) => Ok((StatusCode::CREATED, Json(UploadBackupResponse { artifact }))),
        Err(error) => {
            let _ = crate::services::artifact_files::remove(root, &path);
            let _ = sqlx::query("UPDATE panda.backup_runs SET status = 'failed', phase = 'failed', completed_at = now(), updated_at = now(), error_message = $2 WHERE id = $1 AND status = 'running'")
                .bind(run).bind(error.to_string()).execute(&state.pool).await;
            Err(error)
        }
    }
}

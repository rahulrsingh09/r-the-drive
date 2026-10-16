use super::*;

struct Destinations {
    local: Option<&'static str>,
    remote: bool,
    mirror: bool,
}
impl Destinations {
    fn select(settings: &BackupSettings, trigger: BackupRunTrigger) -> Self {
        let safety = matches!(trigger, BackupRunTrigger::PreRestore);
        let keep = settings.local_enabled
            || safety
            || crate::private_deployment::history_backup::configured();
        Self {
            local: keep.then_some(if safety { "safety" } else { "local" }),
            remote: settings.s3_enabled && !safety,
            mirror: !safety,
        }
    }
}

struct Package {
    path: PathBuf,
    name: String,
    location: String,
    size: i64,
    checksum: String,
    manifest: serde_json::Value,
}
impl Package {
    async fn prepare(
        pool: &PgPool,
        config: &Config,
        settings: &BackupSettings,
        run: Uuid,
        trigger: BackupRunTrigger,
        created: DateTime<Utc>,
        path: &Path,
    ) -> Result<Self, AppError> {
        update_backup_progress(pool, run, "preparing", 5).await?;
        fs::create_dir_all(&config.backup_artifact_dir).await?;
        let package = execute_recovery_package(pool, config, run, path, trigger, created).await?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("backup artifact path is invalid")))?
            .to_owned();
        let opened =
            crate::services::artifact_files::open(Path::new(&config.backup_artifact_dir), path)?;
        let size = i64::try_from(opened.metadata()?.len()).unwrap_or(i64::MAX);
        let checksum = validate_open_recovery_package(opened, &config.recovery)
            .await?
            .checksum_sha256;
        let database = current_database_name(pool).await?;
        let timescale = current_timescale_version(pool).await?;
        Ok(Self {
            path: path.to_owned(),
            name,
            location: path.to_string_lossy().into_owned(),
            size,
            checksum,
            manifest: json!({
                "artifact_kind": BackupDriver::from_config(&config.backup_driver).label(),
                "format": RECOVERY_PACKAGE_FORMAT, "package": package,
                "created_at": created, "database": database, "timescale_version": timescale,
                "trigger": trigger.as_str(), "prefix": settings.prefix, "retention_count": settings.retention_count,
            }),
        })
    }
    async fn catalog(
        &self,
        pool: &PgPool,
        run: Uuid,
        storage: &str,
        location: &str,
        fallback: bool,
    ) -> Result<Uuid, AppError> {
        insert_artifact(
            pool,
            InsertArtifactParams {
                run_id: Some(run),
                storage_type: storage,
                file_name: &self.name,
                storage_path: location,
                size_bytes: self.size,
                checksum_sha256: &self.checksum,
                manifest: with_storage(&self.manifest, storage, fallback),
            },
        )
        .await
    }
    async fn keep_after_remote_failure(
        &self,
        pool: &PgPool,
        run: Uuid,
        destinations: &Destinations,
    ) -> Result<(), AppError> {
        if destinations.local.is_none() {
            self.catalog(pool, run, "local", &self.location, true)
                .await?;
        }
        Ok(())
    }
}

pub(super) async fn execute(
    pool: &PgPool,
    config: &Config,
    settings: &BackupSettings,
    run: Uuid,
    trigger: BackupRunTrigger,
    created: DateTime<Utc>,
    path: &Path,
) -> Result<BackupExecutionResult, AppError> {
    let package = Package::prepare(pool, config, settings, run, trigger, created, path).await?;
    let destinations = Destinations::select(settings, trigger);
    let mut artifacts = Vec::new();
    let mut published = None;
    if let Some(storage) = destinations.local {
        artifacts.push(
            package
                .catalog(pool, run, storage, &package.location, false)
                .await?,
        );
        published = Some(package.location.clone());
    }
    if destinations.remote {
        update_backup_progress(pool, run, "uploading", 92).await?;
        let remote = settings.s3.as_ref().ok_or_else(|| {
            AppError::Validation("S3 is enabled but its credentials are incomplete".into())
        })?;
        let key = s3_backups::object_key(&settings.prefix, created, run)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if let Err(error) =
            s3_backups::upload(remote, &key, &package.path, &package.checksum, run, created).await
        {
            package
                .keep_after_remote_failure(pool, run, &destinations)
                .await?;
            return Err(AppError::DependencyUnavailable(format!(
                "S3 upload failed; the recovery package was retained locally at {}: {error}",
                package.location
            )));
        }
        let location = s3_backups::locator(&remote.bucket, &key);
        artifacts.push(package.catalog(pool, run, "s3", &location, false).await?);
        published = Some(location);
    }
    if destinations.mirror {
        crate::private_deployment::history_backup::mirror(config, &package.path).await?;
    }
    update_backup_progress(pool, run, "finalizing", 98).await?;
    prune_retained_artifacts(pool, config, settings.retention_count).await?;
    if let Some(remote) = settings.s3.as_ref().filter(|_| destinations.remote) {
        if let Err(error) = prune_remote_artifacts(pool, remote, settings.retention_count).await {
            package
                .keep_after_remote_failure(pool, run, &destinations)
                .await?;
            return Err(AppError::DependencyUnavailable(format!(
                "S3 retention failed; the recovery package was retained locally at {}: {error}",
                package.location
            )));
        }
    }
    if destinations.remote && destinations.local.is_none() {
        if let Err(error) = fs::remove_file(&package.path).await {
            tracing::warn!(path = %package.path.display(), error = %error, "backup.cleanup.s3_staging_failed");
        }
    }
    sqlx::query(
        "UPDATE panda.backup_runs SET status = 'succeeded', phase = 'completed', progress_percent = 100,
         artifact_key = $2, completed_at = now(), updated_at = now(), error_message = NULL
         WHERE id = $1 AND status = 'running'",
    ).bind(run).bind(published).execute(pool).await?;
    Ok(BackupExecutionResult {
        run_id: run,
        artifact_ids: artifacts,
    })
}

pub(super) async fn fail(
    pool: &PgPool,
    run: Uuid,
    path: &Path,
    error: &AppError,
) -> Result<(), AppError> {
    mark_backup_run_failed(pool, run, error).await?;
    let retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.backup_artifacts WHERE run_id = $1 AND storage_type IN ('local', 'safety'))",
    ).bind(run).fetch_one(pool).await?;
    if !retained && fs::try_exists(path).await.unwrap_or(false) {
        let _ = fs::remove_file(path).await;
    }
    Ok(())
}

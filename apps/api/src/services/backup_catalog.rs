//! Discover, admit and publish independently verified local recovery files.
use crate::{
    config::Config,
    errors::AppError,
    services::backups::{validate_open_recovery_package, ValidatedRecoveryPackage},
};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;
use walkdir::WalkDir;

pub async fn reconcile_local_catalog(pool: &PgPool, config: &Config) -> Result<usize, AppError> {
    let root = Path::new(&config.backup_artifact_dir);
    if !fs::try_exists(root).await.unwrap_or(false) {
        return Ok(0);
    }
    let mut inserted = 0;
    for path in scan_local_packages(root, config).await? {
        let Some(candidate) = LocalCandidate::admit(pool, path).await? else {
            continue;
        };
        if let Some(verified) = candidate.verify(config).await {
            inserted += verified.publish(pool, root).await?;
        }
    }
    Ok(inserted)
}

async fn scan_local_packages(root: &Path, config: &Config) -> Result<Vec<PathBuf>, AppError> {
    let scan_root = root.to_owned();
    let limits = config.recovery.clone();
    tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let mut paths = Vec::new();
        for entry in WalkDir::new(scan_root).into_iter().filter_entry(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| !name.starts_with('.'))
        }) {
            if started.elapsed() >= std::time::Duration::from_secs(limits.restore_deadline_seconds)
            {
                return Err(AppError::RecoveryDeadline(
                    "Recovery catalog scan deadline exceeded.".into(),
                ));
            }
            let Ok(entry) = entry else { continue };
            if entry.file_type().is_file()
                && entry
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".rma.tar.gz"))
            {
                if paths.len() >= limits.max_members
                    || entry.path().as_os_str().as_encoded_bytes().len() > 64 * 1024
                {
                    return Err(AppError::RecoveryTooLarge(
                        "Recovery catalog scan exceeds its path budget.".into(),
                    ));
                }
                paths.push(entry.into_path());
            }
        }
        paths.sort();
        Ok::<_, AppError>(paths)
    })
    .await
    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
}

struct LocalCandidate {
    path: PathBuf,
    storage_path: String,
    repair: bool,
}

struct VerifiedLocalPackage {
    path: PathBuf,
    storage_path: String,
    repair: bool,
    validated: ValidatedRecoveryPackage,
}

impl LocalCandidate {
    async fn admit(pool: &PgPool, path: PathBuf) -> Result<Option<Self>, AppError> {
        if let Some(run) = backup_run_id_from_artifact_path(&path) {
            let active: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM panda.backup_runs WHERE id = $1 AND status IN ('pending', 'running'))")
                .bind(run).fetch_one(pool).await?;
            if active {
                return Ok(None);
            }
        }
        let storage_path = path.to_string_lossy().into_owned();
        let (exists, unavailable): (bool, bool) = sqlx::query_as(
            "SELECT count(*) > 0, COALESCE(bool_or(manifest->>'restore_availability'='unavailable'), false) FROM panda.backup_artifacts WHERE storage_type <> 's3' AND storage_path=$1",
        ).bind(&storage_path).fetch_one(pool).await?;
        match (exists, unavailable) {
            (true, false) => Ok(None),
            _ => Ok(Some(Self {
                path,
                storage_path,
                repair: exists,
            })),
        }
    }

    async fn verify(self, config: &Config) -> Option<VerifiedLocalPackage> {
        let file = match crate::services::artifact_files::open(
            Path::new(&config.backup_artifact_dir),
            &self.path,
        ) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(error = %error, "backup.catalog.unsafe_local_path");
                return None;
            }
        };
        match validate_open_recovery_package(file, &config.recovery).await {
            Ok(validated) => Some(VerifiedLocalPackage {
                path: self.path,
                storage_path: self.storage_path,
                repair: self.repair,
                validated,
            }),
            Err(error) => {
                tracing::warn!(path = %self.path.display(), error = %error, "backup.catalog.invalid_local_package");
                None
            }
        }
    }
}

impl VerifiedLocalPackage {
    async fn publish(self, pool: &PgPool, root: &Path) -> Result<usize, AppError> {
        if self.repair {
            sqlx::query("UPDATE panda.backup_artifacts SET checksum_sha256=$2, size_bytes=$3, manifest=CASE WHEN jsonb_typeof(manifest)='object' THEN manifest ELSE '{}'::jsonb END || jsonb_build_object('restore_availability','available','package',$4::jsonb) WHERE storage_type <> 's3' AND storage_path=$1")
                .bind(&self.storage_path).bind(&self.validated.checksum_sha256).bind(self.validated.size_bytes).bind(&self.validated.manifest).execute(pool).await?;
            return Ok(0);
        }
        let storage_type = if self.path.starts_with(root.join("imports")) {
            "uploaded"
        } else if self
            .validated
            .manifest
            .get("trigger")
            .and_then(serde_json::Value::as_str)
            == Some("pre_restore")
        {
            "safety"
        } else {
            "local"
        };
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("recovery-package.rma.tar.gz");
        let created_at = self
            .validated
            .manifest
            .get("created_at")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<DateTime<Utc>>().ok())
            .unwrap_or_else(Utc::now);
        let result = sqlx::query(
            r#"
            INSERT INTO panda.backup_artifacts
              (run_id, storage_type, file_name, storage_path, size_bytes, checksum_sha256, manifest, created_at)
            SELECT NULL, $1, $2, $3, $4, $5, $6, $7
            WHERE NOT EXISTS (
              SELECT 1 FROM panda.backup_artifacts
              WHERE storage_type <> 's3' AND storage_path = $3
            )
            "#,
        )
        .bind(storage_type)
        .bind(file_name)
        .bind(&self.storage_path)
        .bind(self.validated.size_bytes)
        .bind(&self.validated.checksum_sha256)
        .bind(json!({
            "artifact_kind": "recovery_package",
            "format": self.validated.manifest.get("format").cloned().unwrap_or(serde_json::Value::Null),
            "package": self.validated.manifest,
            "restore_availability": "available",
            "catalog_source": "filesystem_rescan"
        }))
        .bind(created_at)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }
}

fn backup_run_id_from_artifact_path(path: &Path) -> Option<Uuid> {
    let name = path.file_name()?.to_str()?.strip_suffix(".rma.tar.gz")?;
    name.rsplit_once('-')
        .and_then(|(_, id)| Uuid::parse_str(id).ok())
}

#[cfg(test)]
#[path = "backup_catalog_recovery_operations_tests.rs"]
mod recovery_operations_tests;

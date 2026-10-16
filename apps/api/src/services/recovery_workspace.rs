//! Ephemeral recovery work belongs on the configured backup volume, not /tmp.
use crate::{config::RecoveryConfig, errors::AppError};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) struct RecoveryWorkspace {
    parent: Dir,
    name: String,
    path: PathBuf,
}

impl RecoveryWorkspace {
    pub(crate) fn new(root: &Path, limits: &RecoveryConfig) -> Result<Arc<Self>, AppError> {
        std::fs::create_dir_all(root)?;
        if fs2::available_space(root)? < limits.min_free_bytes {
            return Err(AppError::RecoveryInsufficientStorage(
                "Recovery workspace would consume reserved free space.".into(),
            ));
        }
        let root_dir = Dir::open_ambient_dir(root, ambient_authority())?;
        match root_dir.create_dir(".recovery-work") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let parent = root_dir.open_dir_nofollow(".recovery-work")?;
        let name = uuid::Uuid::new_v4().to_string();
        let builder = cap_std::fs::DirBuilder::new();
        // cap-std directory handles may use O_PATH on Linux, which cannot be
        // fchmod'ed. Apply private permissions atomically at mkdir instead.
        #[cfg(unix)]
        let builder = {
            use cap_std::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        parent.create_dir_with(&name, &builder)?;
        let workspace = Arc::new(Self {
            parent,
            path: root.join(".recovery-work").join(&name),
            name,
        });
        Ok(workspace)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn snapshot(
        self: &Arc<Self>,
        source: &Path,
        limits: &RecoveryConfig,
    ) -> Result<FrozenPackage, AppError> {
        let started = Instant::now();
        let mut source = std::fs::File::open(source)?;
        let metadata = source.metadata()?;
        if !metadata.is_file() {
            return Err(AppError::RecoveryInvalid(
                "Recovery input is not a regular file.".into(),
            ));
        }
        if metadata.len() > limits.max_upload_bytes {
            return Err(AppError::RecoveryTooLarge(
                "Recovery input exceeds the compressed byte limit.".into(),
            ));
        }
        let name = format!(".input-{}.rma.tar.gz", uuid::Uuid::new_v4());
        let directory = self.parent.open_dir_nofollow(&self.name)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = directory.open_with(&name, &options)?.into_std();
        let frozen = FrozenPackage {
            _workspace: Arc::clone(self),
            path: self.path.join(&name),
            directory,
            name,
        };
        let mut buffer = [0; 64 * 1024];
        let mut copied = 0_u64;
        loop {
            if started.elapsed() >= Duration::from_secs(limits.restore_deadline_seconds) {
                return Err(AppError::RecoveryDeadline(
                    "Recovery input snapshot deadline exceeded.".into(),
                ));
            }
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            copied = copied.checked_add(count as u64).ok_or_else(|| {
                AppError::RecoveryTooLarge("Recovery input size overflowed.".into())
            })?;
            if copied > limits.max_upload_bytes {
                return Err(AppError::RecoveryTooLarge(
                    "Recovery input exceeds the compressed byte limit.".into(),
                ));
            }
            if fs2::available_space(self.path())?
                < limits.min_free_bytes.saturating_add(count as u64)
            {
                return Err(AppError::RecoveryInsufficientStorage(
                    "Recovery input snapshot would consume reserved free space.".into(),
                ));
            }
            file.write_all(&buffer[..count])?;
        }
        if copied != metadata.len() {
            return Err(AppError::RecoveryInvalid(
                "Recovery input size changed while snapshotting.".into(),
            ));
        }
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o400))?;
        }
        drop(file);
        Ok(frozen)
    }

    pub(crate) async fn extract(
        self: &Arc<Self>,
        package: &Path,
        limits: &RecoveryConfig,
    ) -> Result<(), AppError> {
        let workspace = Arc::clone(self);
        let package = package.to_owned();
        let limits = limits.clone();
        // A cancelled HTTP future must not remove the workspace underneath
        // spawn_blocking: its last owner cleans up when extraction finishes.
        tokio::task::spawn_blocking(move || {
            let frozen = workspace.snapshot(&package, &limits)?;
            crate::services::recovery_extract::extract(frozen.path(), workspace.path(), &limits)
        })
        .await
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
        .map_err(crate::services::backups::classify_recovery_error)
    }
}

pub(crate) struct FrozenPackage {
    _workspace: Arc<RecoveryWorkspace>,
    directory: Dir,
    name: String,
    path: PathBuf,
}

impl FrozenPackage {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FrozenPackage {
    fn drop(&mut self) {
        let _ = self.directory.remove_file(&self.name);
    }
}

impl Drop for RecoveryWorkspace {
    fn drop(&mut self) {
        let _ = self.parent.remove_dir_all(&self.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_backup_volume_preserves_reserve_and_removes_only_its_workspace() {
        let root = tempfile::tempdir().unwrap();
        let mut limits = RecoveryConfig {
            min_free_bytes: u64::MAX,
            ..RecoveryConfig::default()
        };
        assert!(matches!(
            RecoveryWorkspace::new(root.path(), &limits),
            Err(AppError::RecoveryInsufficientStorage(_))
        ));
        limits.min_free_bytes = 0;
        let first = RecoveryWorkspace::new(root.path(), &limits).unwrap();
        let second = RecoveryWorkspace::new(root.path(), &limits).unwrap();
        let first_path = first.path().to_owned();
        assert!(first_path.starts_with(root.path().join(".recovery-work")));
        std::fs::write(first_path.join("database.dump"), b"private fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&first_path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        drop(first);
        assert!(!first_path.exists());
        assert!(second.path().is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_workspace_parent() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join(".recovery-work")).unwrap();
        let limits = RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        };
        assert!(RecoveryWorkspace::new(root.path(), &limits).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn snapshot_is_independent_and_owns_its_workspace_until_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let limits = RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        };
        let source = root.path().join("source");
        std::fs::write(&source, b"original").unwrap();
        let workspace = RecoveryWorkspace::new(root.path(), &limits).unwrap();
        let path = workspace.path().to_owned();
        let frozen = workspace.snapshot(&source, &limits).unwrap();
        std::fs::write(&source, b"replaced").unwrap();
        drop(workspace);
        assert!(path.exists());
        assert_eq!(std::fs::read(frozen.path()).unwrap(), b"original");
        drop(frozen);
        assert!(!path.exists());
        assert_eq!(std::fs::read(source).unwrap(), b"replaced");
    }

    #[test]
    fn snapshot_limits_and_failures_leave_no_partial_input() {
        let root = tempfile::tempdir().unwrap();
        let limits = RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        };
        let source = root.path().join("source");
        std::fs::write(&source, b"original").unwrap();
        let workspace = RecoveryWorkspace::new(root.path(), &limits).unwrap();
        for case in 0..3 {
            let mut config = limits.clone();
            match case {
                0 => config.max_upload_bytes = 1,
                1 => config.min_free_bytes = u64::MAX,
                _ => config.restore_deadline_seconds = 0,
            }
            assert!(workspace.snapshot(&source, &config).is_err());
            assert_eq!(std::fs::read_dir(workspace.path()).unwrap().count(), 0);
        }
    }

    #[tokio::test]
    async fn cancelled_waiter_cannot_remove_a_blocking_workers_workspace() {
        let root = tempfile::tempdir().unwrap();
        let limits = RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        };
        let workspace = RecoveryWorkspace::new(root.path(), &limits).unwrap();
        let path = workspace.path().to_owned();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finish, wait) = std::sync::mpsc::channel();
        let (cleaned, cleanup) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                drop(workspace);
                cleaned.send(()).unwrap();
            })
            .await
            .unwrap();
        });
        ready.await.unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert!(path.is_dir());
        finish.send(()).unwrap();
        cleanup.await.unwrap();
        assert!(!path.exists());
    }
}

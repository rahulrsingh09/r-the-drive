//! Directory-handle based access to backup artifacts. Catalog paths are data,
//! never ambient filesystem authority.
use crate::errors::AppError;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::{
    fs::File,
    path::{Component, Path, PathBuf},
};

pub fn validate_prefix(value: &str) -> Result<String, AppError> {
    if value.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "Backup paths cannot contain control characters.".into(),
        ));
    }
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return Ok(String::new());
    }
    for part in value.split('/') {
        validate_component(part)?;
    }
    Ok(value.to_owned())
}

fn validate_component(part: &str) -> Result<(), AppError> {
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if part.is_empty()
        || part.starts_with('.')
        || part.ends_with(['.', ' '])
        || part.chars().any(|c| {
            c.is_control() || matches!(c, '\\' | ':' | '/' | '<' | '>' | '"' | '|' | '?' | '*')
        })
        || reserved
    {
        return Err(AppError::Validation(
            "Backup paths must contain ordinary relative path components.".into(),
        ));
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    // Path::components normalizes interior `.` segments. Reject them before
    // normalization so imported catalog paths have one unambiguous spelling.
    if path
        .to_string_lossy()
        .split(['/', '\\'])
        .any(|part| part == "." || part == ".." || part.chars().any(char::is_control))
    {
        return Err(AppError::Validation("Unsafe backup artifact path.".into()));
    }
    let root = if root.is_absolute() {
        root.to_owned()
    } else {
        std::env::current_dir()?.join(root)
    };
    let path = if path.is_absolute() {
        path.strip_prefix(&root)
            .map_err(|_| {
                AppError::Validation("Backup artifact is outside the configured directory.".into())
            })?
            .to_owned()
    } else if let Ok(relative) =
        path.strip_prefix(root.strip_prefix(std::env::current_dir()?).unwrap_or(&root))
    {
        relative.to_owned()
    } else {
        path.to_owned()
    };
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(AppError::Validation("Unsafe backup artifact path.".into()));
    }
    Ok(path)
}

fn parent(
    root: &Path,
    path: &Path,
    create: bool,
    managed: bool,
) -> Result<(Dir, String), AppError> {
    let path = relative_path(root, path)?;
    let mut parts = path
        .iter()
        .map(|p| {
            p.to_str()
                .ok_or_else(|| AppError::Validation("Invalid backup artifact path.".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let name = parts
        .pop()
        .ok_or_else(|| AppError::Validation("Missing backup artifact name.".into()))?;
    validate_component(name)?;
    if !name.ends_with(".rma.tar.gz") {
        return Err(AppError::Validation(
            "Not a recovery package artifact.".into(),
        ));
    }
    for (index, part) in parts.iter().enumerate() {
        if !(managed && index == 0 && matches!(*part, ".remote-staging" | ".restore-staging")) {
            validate_component(part)?;
        }
    }
    let mut dir = Dir::open_ambient_dir(root, ambient_authority())?;
    for part in &parts {
        if create {
            match dir.create_dir(part) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        dir = dir.open_dir_nofollow(part)?;
    }
    Ok((dir, name.to_owned()))
}

pub fn open(root: &Path, path: &Path) -> Result<File, AppError> {
    let (dir, name) = parent(root, path, false, false)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(name, &options)?;
    if !file.metadata()?.is_file() {
        return Err(AppError::Validation(
            "Backup artifact is not a regular file.".into(),
        ));
    }
    Ok(file.into_std())
}

pub fn remove(root: &Path, path: &Path) -> Result<(), AppError> {
    let (dir, name) = parent(root, path, false, false)?;
    // Refuse final symlinks as well as parent symlinks. remove_file itself does
    // not follow a replacement symlink and the parent handle remains pinned.
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(&name, &options)?;
    if !file.metadata()?.is_file() {
        return Err(AppError::Validation(
            "Backup artifact is not a regular file.".into(),
        ));
    }
    drop(file);
    dir.remove_file(name)?;
    Ok(())
}

pub fn remove_staged(root: &Path, id: uuid::Uuid) -> Result<(), AppError> {
    let path = root
        .join(".remote-staging")
        .join(format!("{id}.rma.tar.gz"));
    let (dir, name) = parent(root, &path, false, true)?;
    dir.remove_file(name)?;
    Ok(())
}

pub fn create(root: &Path, path: &Path) -> Result<File, AppError> {
    let (dir, name) = parent(root, path, true, false)?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    Ok(dir.open_with(name, &options)?.into_std())
}

/// A service-owned staging destination; callers choose a UUID, never a catalog
/// path. Temporary and final names share a pinned directory handle.
pub struct StagingFile {
    dir: Dir,
    temporary: String,
    final_name: String,
    pub file: File,
    pub path: PathBuf,
}
impl StagingFile {
    pub fn new(root: &Path, id: uuid::Uuid) -> Result<Self, AppError> {
        Self::new_at(root, ".remote-staging", format!("{id}.rma.tar.gz"))
    }
    pub fn upload(root: &Path, id: uuid::Uuid) -> Result<Self, AppError> {
        Self::new_at(
            root,
            "imports",
            format!(
                "import-{}-{}.rma.tar.gz",
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
                id.simple()
            ),
        )
    }
    fn restore(root: &Path, id: uuid::Uuid, host: bool) -> Result<Self, AppError> {
        Self::new_at(
            root,
            &format!(".restore-staging/{}{id}", if host { "host-" } else { "" }),
            "recovery-package.rma.tar.gz".into(),
        )
    }
    fn new_at(root: &Path, directory: &str, name: String) -> Result<Self, AppError> {
        let path = root.join(directory).join(name);
        let (dir, final_name) = parent(root, &path, true, true)?;
        let temporary = format!("{}.downloading", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = dir.open_with(&temporary, &options)?.into_std();
        Ok(Self {
            dir,
            temporary,
            final_name,
            file,
            path,
        })
    }
    pub fn publish(self) -> Result<PathBuf, AppError> {
        self.file.sync_all()?;
        self.dir
            .rename(&self.temporary, &self.dir, &self.final_name)?;
        Ok(self.path.clone())
    }
    pub fn file_name(&self) -> &str {
        &self.final_name
    }
}
impl Drop for StagingFile {
    fn drop(&mut self) {
        let _ = self.dir.remove_file(&self.temporary);
    }
}

pub async fn stage_reader<R: tokio::io::AsyncRead + Unpin>(
    root: &Path,
    id: uuid::Uuid,
    reader: &mut R,
    limits: &crate::config::RecoveryConfig,
) -> Result<PathBuf, AppError> {
    let destination = StagingFile::new(root, id)?;
    copy_to_staging(root, destination, reader, limits).await
}

/// Internal restore copies use the same byte, disk, deadline and cleanup
/// policy as remote materialization; copying a validated archive is still I/O.
pub async fn stage_restore_reader<R: tokio::io::AsyncRead + Unpin>(
    root: &Path,
    id: uuid::Uuid,
    host: bool,
    reader: &mut R,
    limits: &crate::config::RecoveryConfig,
) -> Result<PathBuf, AppError> {
    let destination = StagingFile::restore(root, id, host)?;
    copy_to_staging(root, destination, reader, limits).await
}

async fn copy_to_staging<R: tokio::io::AsyncRead + Unpin>(
    root: &Path,
    destination: StagingFile,
    reader: &mut R,
    limits: &crate::config::RecoveryConfig,
) -> Result<PathBuf, AppError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(limits.restore_deadline_seconds),
        async {
            let mut file = tokio::fs::File::from_std(destination.file.try_clone()?);
            let mut total = 0_u64;
            let mut buffer = vec![0_u8; 64 * 1024];
            loop {
                let length = reader.read(&mut buffer).await?;
                if length == 0 {
                    break;
                }
                total = total.checked_add(length as u64).ok_or_else(|| {
                    AppError::RecoveryTooLarge("Recovery download size overflowed.".into())
                })?;
                if total > limits.max_upload_bytes {
                    return Err(AppError::RecoveryTooLarge(
                        "Recovery download exceeds the configured compressed byte limit.".into(),
                    ));
                }
                if fs2::available_space(root)? < limits.min_free_bytes.saturating_add(length as u64)
                {
                    return Err(AppError::RecoveryInsufficientStorage(
                        "Recovery download would consume reserved free space.".into(),
                    ));
                }
                file.write_all(&buffer[..length]).await?;
            }
            file.flush().await?;
            drop(file);
            destination.publish()
        },
    )
    .await;
    result.map_err(|_| AppError::RecoveryDeadline("Recovery download deadline exceeded.".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn internal_restore_copies_preserve_disk_reserve_and_remove_partial_files() {
        let temp = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4();
        let mut limits = crate::config::RecoveryConfig {
            max_upload_bytes: 8,
            min_free_bytes: 0,
            ..Default::default()
        };
        let mut oversized: &[u8] = b"oversized fixture";
        assert!(matches!(
            stage_restore_reader(temp.path(), id, false, &mut oversized, &limits).await,
            Err(AppError::RecoveryTooLarge(_))
        ));
        let staging = temp.path().join(".restore-staging").join(id.to_string());
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
        limits.min_free_bytes = u64::MAX;
        let mut small: &[u8] = b"small";
        assert!(matches!(
            stage_restore_reader(temp.path(), id, false, &mut small, &limits).await,
            Err(AppError::RecoveryInsufficientStorage(_))
        ));
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
        limits.min_free_bytes = 0;
        let path = stage_restore_reader(temp.path(), id, true, &mut &b"small"[..], &limits)
            .await
            .unwrap();
        assert_eq!(
            path,
            temp.path()
                .join(".restore-staging")
                .join(format!("host-{id}"))
                .join("recovery-package.rma.tar.gz")
        );
        assert_eq!(std::fs::read(path).unwrap(), b"small");
    }
    #[tokio::test]
    async fn oversized_and_stalled_materialization_remove_partial_files() {
        let temp = tempfile::tempdir().unwrap();
        let limits = crate::config::RecoveryConfig {
            max_upload_bytes: 8,
            min_free_bytes: 0,
            restore_deadline_seconds: 1,
            ..Default::default()
        };
        let mut source: &[u8] = b"oversized fixture";
        assert!(matches!(
            stage_reader(temp.path(), uuid::Uuid::new_v4(), &mut source, &limits).await,
            Err(AppError::RecoveryTooLarge(_))
        ));
        let (mut stalled, _writer) = tokio::io::duplex(64);
        assert!(matches!(
            stage_reader(temp.path(), uuid::Uuid::new_v4(), &mut stalled, &limits).await,
            Err(AppError::RecoveryDeadline(_))
        ));
        assert_eq!(
            std::fs::read_dir(temp.path().join(".remote-staging"))
                .unwrap()
                .count(),
            0
        );
    }
    #[test]
    fn rejects_portable_traversal_and_control_paths() {
        for prefix in [
            "../other",
            "/etc",
            "a/../b",
            "a\\b",
            "C:/tmp",
            ".restore-jobs",
            "CON",
            "a//b",
            "a\n",
        ] {
            assert!(validate_prefix(prefix).is_err(), "{prefix:?}");
        }
        assert_eq!(validate_prefix("panda/prod/").unwrap(), "panda/prod");
        assert_eq!(validate_prefix("").unwrap(), "");
    }
    #[test]
    fn only_regular_artifacts_under_root_are_accessible() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backups");
        std::fs::create_dir(&root).unwrap();
        let artifact = root.join("one/test.rma.tar.gz");
        create(&root, &artifact).unwrap();
        assert!(open(&root, &artifact).is_ok());
        assert!(open(&root, &temp.path().join("secret.rma.tar.gz")).is_err());
        assert!(open(&root, &root.join("one/../one/test.rma.tar.gz")).is_err());
        assert!(open(&root, &root.join("one/./test.rma.tar.gz")).is_err());
        assert!(open(&root, &root.join(".remote-staging/test.rma.tar.gz")).is_err());
        remove(&root, &artifact).unwrap();
        assert!(!artifact.exists());
    }
    #[cfg(unix)]
    #[test]
    fn refuses_parent_and_final_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backups");
        std::fs::create_dir(&root).unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let secret = outside.join("secret.rma.tar.gz");
        std::fs::write(&secret, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
        std::os::unix::fs::symlink(&secret, root.join("secret.rma.tar.gz")).unwrap();
        for path in [
            root.join("linked/secret.rma.tar.gz"),
            root.join("secret.rma.tar.gz"),
        ] {
            assert!(open(&root, &path).is_err());
            assert!(remove(&root, &path).is_err());
        }
        assert!(secret.exists());
    }

    #[cfg(windows)]
    #[test]
    fn refuses_directory_junctions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backups");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let secret = outside.join("secret.rma.tar.gz");
        std::fs::write(&secret, b"secret").unwrap();
        let linked = root.join("linked");
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:PANDA_JUNCTION_LINK -Target $env:PANDA_JUNCTION_TARGET | Out-Null"])
            .env("PANDA_JUNCTION_LINK", &linked)
            .env("PANDA_JUNCTION_TARGET", &outside)
            .status().unwrap();
        assert!(status.success(), "create disposable directory junction");
        let path = linked.join("secret.rma.tar.gz");
        let read_rejected = open(&root, &path).is_err();
        let delete_rejected = remove(&root, &path).is_err();
        let create_rejected = create(&root, &linked.join("new.rma.tar.gz")).is_err();
        std::fs::remove_dir(&linked).unwrap();
        assert!(read_rejected && delete_rejected && create_rejected);
        assert!(secret.exists());
        assert!(!outside.join("new.rma.tar.gz").exists());
    }
}

#[cfg(test)]
mod recovery_admission_tests {
    use super::*;
    use std::{io, pin::Pin, task::{Context, Poll}};
    use tokio::io::{AsyncRead, ReadBuf};

    struct PartialFailure { sent: bool }
    impl AsyncRead for PartialFailure {
        fn poll_read(mut self: Pin<&mut Self>, _: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            if self.sent { Poll::Ready(Err(io::Error::other("synthetic reader failure"))) }
            else { self.sent = true; buffer.put_slice(b"partial"); Poll::Ready(Ok(())) }
        }
    }

    #[tokio::test]
    async fn exact_byte_limit_names_and_empty_stream_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let limits = crate::config::RecoveryConfig { max_upload_bytes: 7, min_free_bytes: 0, ..Default::default() };
        let id = uuid::Uuid::new_v4();
        let path = stage_reader(root.path(), id, &mut &b"1234567"[..], &limits).await.unwrap();
        assert_eq!(path, root.path().join(".remote-staging").join(format!("{id}.rma.tar.gz")));
        assert_eq!(std::fs::read(&path).unwrap(), b"1234567");
        remove_staged(root.path(), id).unwrap();
        let path = stage_reader(root.path(), id, &mut tokio::io::empty(), &limits).await.unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        let upload = StagingFile::upload(root.path(), id).unwrap();
        assert!(upload.file_name().starts_with("import-"));
        assert!(upload.file_name().ends_with(&format!("-{}.rma.tar.gz", id.simple())));
        assert_eq!(upload.path.parent().unwrap(), root.path().join("imports"));
    }

    #[tokio::test]
    async fn input_error_and_cancelled_copy_leave_no_partial_or_final_file() {
        let root = tempfile::tempdir().unwrap();
        let limits = crate::config::RecoveryConfig { min_free_bytes: 0, ..Default::default() };
        let id = uuid::Uuid::new_v4();
        let error = stage_reader(root.path(), id, &mut PartialFailure { sent: false }, &limits).await.unwrap_err();
        assert!(matches!(error, AppError::Io(_)));
        assert_eq!(std::fs::read_dir(root.path().join(".remote-staging")).unwrap().count(), 0);
        let (mut source, mut writer) = tokio::io::duplex(64);
        use tokio::io::AsyncWriteExt;
        writer.write_all(b"partial").await.unwrap();
        let work = stage_reader(root.path(), id, &mut source, &limits);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), work).await.is_err());
        assert_eq!(std::fs::read_dir(root.path().join(".remote-staging")).unwrap().count(), 0);
    }

    #[test]
    fn admitted_paths_resolve_relative_absolute_and_portable_names() {
        let root = tempfile::tempdir().unwrap();
        let relative = Path::new("prefix/fixture.rma.tar.gz");
        create(root.path(), relative).unwrap();
        assert!(open(root.path(), relative).is_ok());
        assert!(open(root.path(), &root.path().join(relative)).is_ok());
        for prefix in ["com1.log", "LPT9", "nul.json", "a/b.", "a/b /x", "a/b?c", "a/b*c", "a/b|c", "a/b<c", "a/b>c", "a/b\"c", "a/.hidden"] {
            assert!(validate_prefix(prefix).is_err(), "{prefix}");
        }
        assert_eq!(validate_prefix("  panda/prod///  ").unwrap(), "panda/prod");
        assert_eq!(validate_prefix("///").unwrap(), "");
        assert!(validate_prefix("COM10").is_ok());
        let directory = root.path().join("dir.rma.tar.gz");
        std::fs::create_dir(&directory).unwrap();
        assert!(open(root.path(), &directory).is_err());
        assert!(remove(root.path(), &directory).is_err());
    }

    #[test]
    fn recovery_regression_invalid_late_component_has_no_creation_side_effects() {
        let root = tempfile::tempdir().unwrap();
        assert!(create(root.path(), Path::new("new/.hidden/file.rma.tar.gz")).is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn publication_failure_removes_temporary_and_preserves_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4();
        let staged = StagingFile::new(root.path(), id).unwrap();
        std::fs::create_dir(&staged.path).unwrap();
        let path = staged.path.clone();
        assert!(staged.publish().is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn staged_publication_and_cleanup_use_the_pinned_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4();
        let staged = StagingFile::new(root.path(), id).unwrap();
        let original = root.path().join(".remote-staging");
        let moved = root.path().join("moved");
        std::fs::rename(&original, &moved).unwrap();
        std::os::unix::fs::symlink(outside.path(), &original).unwrap();
        staged.publish().unwrap();
        assert!(moved.join(format!("{id}.rma.tar.gz")).is_file());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(StagingFile::new(root.path(), uuid::Uuid::new_v4()).is_err());
    }
}

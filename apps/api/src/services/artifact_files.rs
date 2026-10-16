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

struct ArtifactLocation {
    parents: Vec<String>,
    name: String,
}

impl ArtifactLocation {
    fn admit(root: &Path, path: &Path, managed: bool) -> Result<Self, AppError> {
        let path = relative_path(root, path)?;
        let mut parts = path
            .iter()
            .map(|part| {
                part.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| AppError::Validation("Invalid backup artifact path.".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let name = parts
            .pop()
            .ok_or_else(|| AppError::Validation("Missing backup artifact name.".into()))?;
        validate_component(&name)?;
        if !name.ends_with(".rma.tar.gz") {
            return Err(AppError::Validation(
                "Not a recovery package artifact.".into(),
            ));
        }
        for (index, part) in parts.iter().enumerate() {
            if !(managed
                && index == 0
                && matches!(part.as_str(), ".remote-staging" | ".restore-staging"))
            {
                validate_component(part)?;
            }
        }
        Ok(Self {
            parents: parts,
            name,
        })
    }

    fn bind(self, root: &Path, create: bool) -> Result<ArtifactParent, AppError> {
        let mut dir = Dir::open_ambient_dir(root, ambient_authority())?;
        for part in self.parents {
            if create {
                match dir.create_dir(&part) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
            }
            dir = dir.open_dir_nofollow(part)?;
        }
        Ok(ArtifactParent {
            dir,
            name: self.name,
        })
    }
}

struct ArtifactParent {
    dir: Dir,
    name: String,
}

impl ArtifactParent {
    fn regular_file(&self) -> Result<File, AppError> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self.dir.open_with(&self.name, &options)?;
        if !file.metadata()?.is_file() {
            return Err(AppError::Validation(
                "Backup artifact is not a regular file.".into(),
            ));
        }
        Ok(file.into_std())
    }
}

pub fn open(root: &Path, path: &Path) -> Result<File, AppError> {
    ArtifactLocation::admit(root, path, false)?
        .bind(root, false)?
        .regular_file()
}

pub fn remove(root: &Path, path: &Path) -> Result<(), AppError> {
    let artifact = ArtifactLocation::admit(root, path, false)?.bind(root, false)?;
    drop(artifact.regular_file()?);
    artifact.dir.remove_file(artifact.name)?;
    Ok(())
}

pub fn remove_staged(root: &Path, id: uuid::Uuid) -> Result<(), AppError> {
    let path = root
        .join(".remote-staging")
        .join(format!("{id}.rma.tar.gz"));
    let artifact = ArtifactLocation::admit(root, &path, true)?.bind(root, false)?;
    artifact.dir.remove_file(artifact.name)?;
    Ok(())
}

pub fn create(root: &Path, path: &Path) -> Result<File, AppError> {
    let artifact = ArtifactLocation::admit(root, path, false)?.bind(root, true)?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    Ok(artifact.dir.open_with(artifact.name, &options)?.into_std())
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
        let ArtifactParent {
            dir,
            name: final_name,
        } = ArtifactLocation::admit(root, &path, true)?.bind(root, true)?;
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
    tokio::time::timeout(
        std::time::Duration::from_secs(limits.restore_deadline_seconds),
        async {
            let mut file = tokio::fs::File::from_std(destination.file.try_clone()?);
            let mut admission = TransferAdmission {
                root,
                limits,
                total: 0,
            };
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let length = reader.read(&mut buffer).await?;
                if length == 0 {
                    break;
                }
                admission.accept(length)?;
                file.write_all(&buffer[..length]).await?;
            }
            file.flush().await?;
            drop(file);
            destination.publish()
        },
    )
    .await
    .map_err(|_| AppError::RecoveryDeadline("Recovery download deadline exceeded.".into()))?
}

struct TransferAdmission<'a> {
    root: &'a Path,
    limits: &'a crate::config::RecoveryConfig,
    total: u64,
}

impl TransferAdmission<'_> {
    fn accept(&mut self, length: usize) -> Result<(), AppError> {
        let total = self.total.checked_add(length as u64).ok_or_else(|| {
            AppError::RecoveryTooLarge("Recovery download size overflowed.".into())
        })?;
        if total > self.limits.max_upload_bytes {
            return Err(AppError::RecoveryTooLarge(
                "Recovery download exceeds the configured compressed byte limit.".into(),
            ));
        }
        if fs2::available_space(self.root)?
            < self.limits.min_free_bytes.saturating_add(length as u64)
        {
            return Err(AppError::RecoveryInsufficientStorage(
                "Recovery download would consume reserved free space.".into(),
            ));
        }
        self.total = total;
        Ok(())
    }
}

#[cfg(test)]
#[path = "__tests__/artifact_files_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "__tests__/artifact_files_recovery_admission_tests.rs"]
mod recovery_admission_tests;

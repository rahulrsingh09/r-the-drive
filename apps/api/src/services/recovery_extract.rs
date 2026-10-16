//! Admission-bound streaming extraction into a service-owned recovery directory.
use crate::{config::RecoveryConfig, errors::AppError};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::{Dir, OpenOptions}};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tar::Archive;

pub(crate) fn extract(
    package_path: &Path,
    destination: &Path,
    limits: &RecoveryConfig,
) -> Result<(), AppError> {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(limits.restore_deadline_seconds);
    // Validate before extraction and then perform a second constrained stream
    // pass. Validation is not reusable state: callers may replace a path
    // between two operations, so the extractor must defend itself as well.
    let admitted = crate::services::backups::validate_recovery_package_sync_with_limits(package_path, limits)?;
    std::fs::create_dir_all(destination)?;

    let compressed_bytes = std::fs::metadata(package_path)?.len();
    if compressed_bytes > limits.max_upload_bytes {
        return Err(AppError::Validation(
            "Recovery package exceeds the configured compressed byte limit.".into(),
        ));
    }
    let file = PackageStream::open(package_path, limits, started)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut transaction = ExtractionOutput::new(destination)?;
    let result = (|| -> Result<(), AppError> {
        let mut members = HashSet::<String>::new();
        let mut expanded_bytes = 0_u64;
        let mut member_count = 0_usize;
        for entry in archive
            .entries()
            .map_err(|error| AppError::Validation(format!("Invalid recovery archive: {error}")))?
        {
            let mut entry = entry.map_err(|error| {
                AppError::Validation(format!("Invalid recovery archive entry: {error}"))
            })?;
            let path = entry
                .path()
                .map_err(|error| AppError::Validation(format!("Invalid archive path: {error}")))?;
            let normalized = path.to_string_lossy().replace('\\', "/");
            let entry_type = entry.header().entry_type();
            if !entry_type.is_file() && !entry_type.is_dir()
                || path.is_absolute()
                || normalized.starts_with('/')
                || normalized.trim_end_matches('/').split('/').any(|segment| {
                    segment == "."
                        || segment == ".."
                        || segment.is_empty()
                        || segment.contains(':')
                        || segment.chars().any(char::is_control)
                })
                || !members.insert(normalized.clone())
            {
                return Err(AppError::Validation(
                    "Unsafe recovery archive member.".into(),
                ));
            }
            member_count += 1;
            if member_count > limits.max_members {
                return Err(AppError::Validation(
                    "Recovery package has too many archive members.".into(),
                ));
            }
            let allowed = matches!(
                normalized.as_str(),
                "manifest.json"
                    | "database.dump"
                    | "backup-settings.json"
                    | "operational-history.json"
            ) || normalized == "vehicle-image-cache"
                || normalized.starts_with("vehicle-image-cache/");
            if !allowed {
                return Err(AppError::Validation(format!(
                    "Unexpected recovery package member: {normalized}"
                )));
            }
            if entry_type.is_dir() {
                transaction.directory(&normalized)?;
                continue;
            }
            if std::time::Instant::now() >= deadline {
                return Err(AppError::RecoveryDeadline(
                    "Recovery validation deadline exceeded.".into(),
                ));
            }
            let entry_size = entry.size();
            if entry_size > crate::services::backups::metadata_limit(&normalized, limits) {
                return Err(AppError::Validation(format!("Recovery package member {normalized} exceeds its configured metadata or member size limit.")));
            }
            expanded_bytes = expanded_bytes.checked_add(entry_size).ok_or_else(|| {
                AppError::Validation("Recovery package expanded size overflowed.".into())
            })?;
            if entry_size > limits.max_member_bytes || expanded_bytes > limits.max_expanded_bytes {
                return Err(AppError::Validation(
                    "Recovery package exceeds extraction limits.".into(),
                ));
            }
            if expanded_bytes
                > compressed_bytes
                    .max(1)
                    .saturating_mul(limits.max_compression_ratio)
            {
                return Err(AppError::Validation(
                    "Recovery package exceeds the maximum expansion ratio.".into(),
                ));
            }
            let mut output = transaction.file(&normalized)?;
            let mut copied = 0_u64;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                if std::time::Instant::now() >= deadline {
                    return Err(AppError::RecoveryDeadline(
                        "Recovery extraction deadline exceeded.".into(),
                    ));
                }
                let length = entry.read(&mut buffer)?;
                if length == 0 {
                    break;
                }
                if fs2::available_space(destination)?
                    < limits.min_free_bytes.saturating_add(length as u64)
                {
                    return Err(AppError::RecoveryInsufficientStorage(
                        "Recovery extraction would consume reserved free space.".into(),
                    ));
                }
                output.write_all(&buffer[..length])?;
                copied += length as u64;
            }
            if copied != entry_size {
                return Err(AppError::Validation(format!(
                    "Recovery package member {normalized} size changed during extraction."
                )));
            }
            output.flush()?;
        }
        let mut decoder = archive.into_inner();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            check_deadline(started, limits)?;
            if decoder.read(&mut buffer).map_err(stream_error)? == 0 { break; }
        }
        let mut source = decoder.into_inner();
        while source.read(&mut buffer).map_err(stream_error)? != 0 {}
        if hex::encode(source.digest.finalize()) != admitted.checksum_sha256 {
            return Err(invalid("Recovery package changed after admission."));
        }
        Ok(())
    })();
    transaction.committed = result.is_ok();
    result
}

fn invalid(message: impl Into<String>) -> AppError { AppError::Validation(message.into()) }

fn check_deadline(started: Instant, limits: &RecoveryConfig) -> Result<(), AppError> {
    if started.elapsed() >= Duration::from_secs(limits.restore_deadline_seconds) {
        Err(AppError::RecoveryDeadline("Recovery extraction deadline exceeded.".into()))
    } else { Ok(()) }
}

fn stream_error(error: io::Error) -> AppError {
    if error.get_ref().is_some_and(|inner| inner.is::<AppError>()) {
        *error.into_inner().unwrap().downcast::<AppError>().unwrap()
    } else { error.into() }
}

struct PackageStream<'a> {
    file: File,
    digest: Sha256,
    consumed: u64,
    limits: &'a RecoveryConfig,
    started: Instant,
}

impl<'a> PackageStream<'a> {
    fn open(path: &Path, limits: &'a RecoveryConfig, started: Instant) -> Result<Self, AppError> {
        check_deadline(started, limits)?;
        let file = File::open(path)?;
        if file.metadata()?.len() > limits.max_upload_bytes {
            return Err(invalid("Recovery package exceeds the configured compressed byte limit."));
        }
        Ok(Self { file, digest: Sha256::new(), consumed: 0, limits, started })
    }
}

impl Read for PackageStream<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check_deadline(self.started, self.limits).map_err(io::Error::other)?;
        let length = self.file.read(buffer)?;
        self.consumed = self.consumed.checked_add(length as u64).ok_or_else(|| io::Error::other(invalid("Recovery package size overflowed.")))?;
        if self.consumed > self.limits.max_upload_bytes {
            return Err(io::Error::other(invalid("Recovery package exceeds the configured compressed byte limit.")));
        }
        self.digest.update(&buffer[..length]);
        Ok(length)
    }
}

struct ExtractionOutput {
    root: Dir,
    created: Vec<(PathBuf, bool)>,
    committed: bool,
}

impl ExtractionOutput {
    fn new(destination: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(destination)?;
        let root = if let Some(name) = destination.file_name() {
            let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
            Dir::open_ambient_dir(parent, ambient_authority())?.open_dir_nofollow(name)?
        } else { Dir::open_ambient_dir(destination, ambient_authority())? };
        Ok(Self { root, created: Vec::new(), committed: false })
    }

    fn directory(&mut self, path: &str) -> Result<Dir, AppError> {
        let mut dir = self.root.try_clone()?;
        let mut relative = PathBuf::new();
        for part in path.split('/').filter(|part| !part.is_empty()) {
            relative.push(part);
            match dir.create_dir(part) {
                Ok(()) => self.created.push((relative.clone(), true)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(error.into()),
            }
            dir = dir.open_dir_nofollow(part)?;
        }
        Ok(dir)
    }

    fn file(&mut self, path: &str) -> Result<File, AppError> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let dir = self.directory(parent)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).follow(FollowSymlinks::No);
        let file = dir.open_with(name, &options).map_err(|error| invalid(format!("Recovery package would overwrite an existing file: {error}")))?;
        self.created.push((PathBuf::from(path), false));
        Ok(file.into_std())
    }

    fn remove_created(&self, path: &Path, directory: bool) -> io::Result<()> {
        let mut parent = self.root.try_clone()?;
        for part in path.parent().into_iter().flat_map(Path::iter) { parent = parent.open_dir_nofollow(part)?; }
        let name = path.file_name().ok_or_else(|| io::Error::other("Missing extraction member name"))?;
        if directory { parent.remove_dir(name) } else { parent.remove_file(name) }
    }
}

impl Drop for ExtractionOutput {
    fn drop(&mut self) {
        if !self.committed {
            for (path, directory) in self.created.iter().rev() { let _ = self.remove_created(path, *directory); }
        }
    }
}

#[cfg(test)]
pub(crate) mod recovery_validation_fixtures {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use sha2::{Digest, Sha256};
    use tar::{Builder, EntryType, Header};

    pub(crate) type Member = (String, Vec<u8>, EntryType);

    pub(crate) fn members(version: u64) -> Vec<Member> {
        let mut manifest = serde_json::json!({
            "format": format!("panda-recovery-v{version}"), "format_version": version,
            "source": {
                "postgres_major": 17, "migration_version": 1,
                "migration_ledger": [{"version": 1, "checksum_sha384": "synthetic"}],
                "schema_fingerprint": "synthetic", "migration_chain_id": "synthetic",
                "migration_catalog_digest": "synthetic", "schema_contract_version": "1",
                "app_version": "synthetic"
            },
            "restore": {"engine_version": 1}, "components": {}
        });
        let mut result = Vec::new();
        for (component, path, bytes) in [
            ("database", "database.dump", &b"PGDMPsynthetic"[..]),
            ("backup_settings", "backup-settings.json", &b"{}"[..]),
            ("operational_history", "operational-history.json", &b"{}"[..]),
        ] {
            manifest["components"][component] = serde_json::json!({
                "version": 1, "restore_policy": "replace", "redactions": [],
                "path": path, "size_bytes": bytes.len(), "sha256": hex::encode(Sha256::digest(bytes))
            });
            result.push((path.to_owned(), bytes.to_vec(), EntryType::Regular));
        }
        let files = serde_json::json!([]);
        manifest["components"]["vehicle_image_cache"] = serde_json::json!({
            "version": 1, "restore_policy": "replace", "redactions": [],
            "files": files, "file_count": 0, "size_bytes": 0,
            "sha256": hex::encode(Sha256::digest(serde_json::to_vec(&files).unwrap()))
        });
        result.insert(0, ("manifest.json".into(), serde_json::to_vec(&manifest).unwrap(), EntryType::Regular));
        result.push(("vehicle-image-cache/".into(), Vec::new(), EntryType::Directory));
        result
    }

    pub(crate) fn write_archive(path: &Path, members: &[Member]) {
        let mut archive = Builder::new(GzEncoder::new(File::create(path).unwrap(), Compression::default()));
        for (name, bytes, kind) in members {
            let mut header = Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            assert!(name.len() < 100);
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            archive.append(&header, &bytes[..]).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }

    pub(crate) fn limits() -> RecoveryConfig {
        RecoveryConfig { min_free_bytes: 0, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::{recovery_validation_fixtures::*, *};
    use tar::EntryType;

    #[test]
    fn recovery_contract_extracts_all_supported_formats_and_empty_cache() {
        let root = tempfile::tempdir().unwrap();
        for version in 1..=3 {
            let package = root.path().join(format!("{version}.rma.tar.gz"));
            let destination = root.path().join(format!("out-{version}"));
            write_archive(&package, &members(version));
            extract(&package, &destination, &limits()).unwrap();
            assert_eq!(std::fs::read(destination.join("database.dump")).unwrap(), b"PGDMPsynthetic");
            assert!(destination.join("vehicle-image-cache").is_dir());
            assert!(package.exists());
        }
    }

    #[test]
    fn recovery_contract_rejects_paths_types_and_duplicates_before_writing() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let destination = root.path().join("out");
        for name in ["../escape", "/escape", "C:/escape", "vehicle-image-cache/../escape", "vehicle-image-cache\\..\\escape", "vehicle-image-cache//x", "vehicle-image-cache/./x", "vehicle-image-cache/x\n", "unknown", "manifest.json"] {
            let mut entries = members(1);
            entries.push((name.into(), b"x".to_vec(), EntryType::Regular));
            write_archive(&package, &entries);
            assert!(extract(&package, &destination, &limits()).is_err(), "{name:?}");
            assert!(!destination.exists());
        }
        for kind in [EntryType::Symlink, EntryType::Link, EntryType::Fifo, EntryType::Char, EntryType::Block] {
            let mut entries = members(1);
            entries.push(("vehicle-image-cache/link".into(), Vec::new(), kind));
            write_archive(&package, &entries);
            assert!(extract(&package, &destination, &limits()).is_err(), "{kind:?}");
            assert!(!destination.exists());
        }
    }

    #[test]
    fn recovery_contract_metadata_checksum_magic_and_required_members_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        for case in 0..5 {
            let mut entries = members(1);
            match case {
                0 => entries[0].1 = b"{".to_vec(),
                1 => { entries.remove(0); },
                2 => { entries.remove(1); },
                3 => entries[1].1 = b"PGDMPdifferent".to_vec(),
                _ => entries[1].1 = b"plain SQL".to_vec(),
            }
            write_archive(&package, &entries);
            let destination = root.path().join(format!("out-{case}"));
            assert!(extract(&package, &destination, &limits()).is_err());
            assert!(!destination.exists());
        }
    }

    #[test]
    fn recovery_contract_enforces_each_configured_limit() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let entries = members(1);
        write_archive(&package, &entries);
        for case in 0..9 {
            let mut config = limits();
            match case {
                0 => config.max_upload_bytes = 1,
                1 => config.max_members = 1,
                2 => config.max_member_bytes = 1,
                3 => config.max_expanded_bytes = 1,
                4 => config.max_compression_ratio = 0,
                5 => config.max_manifest_bytes = 1,
                6 => config.max_settings_bytes = 1,
                7 => config.max_history_bytes = 1,
                _ => config.restore_deadline_seconds = 0,
            }
            let destination = root.path().join(format!("out-{case}"));
            let error = crate::services::backups::classify_recovery_error(extract(&package, &destination, &config).unwrap_err());
            assert!(matches!(error, AppError::RecoveryTooLarge(_) | AppError::RecoveryDeadline(_)), "{case}: {error:?}");
            assert!(!destination.exists());
        }
    }

    #[test]
    fn recovery_contract_preserves_existing_files_and_cleans_partial_writes() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        write_archive(&package, &members(1));
        let destination = root.path().join("out");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("database.dump"), b"existing").unwrap();
        assert!(extract(&package, &destination, &limits()).is_err());
        assert_eq!(std::fs::read(destination.join("database.dump")).unwrap(), b"existing");
        assert!(!destination.join("manifest.json").exists());
        let mut config = limits();
        config.min_free_bytes = u64::MAX;
        assert!(matches!(extract(&package, &root.path().join("reserved"), &config), Err(AppError::RecoveryInsufficientStorage(_))));
        assert_eq!(std::fs::read_dir(root.path().join("reserved")).unwrap().count(), 0);
    }

    #[test]
    fn recovery_regression_rejects_missing_or_corrupt_gzip_footer() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        write_archive(&package, &members(1));
        let original = std::fs::read(&package).unwrap();
        for case in 0..2 {
            let mut bytes = original.clone();
            if case == 0 { bytes.truncate(bytes.len() - 8); } else { let end = bytes.len(); bytes[end - 8] ^= 1; }
            std::fs::write(&package, bytes).unwrap();
            let destination = root.path().join(format!("out-{case}"));
            assert!(extract(&package, &destination, &limits()).is_err(), "footer case {case} accepted");
            assert!(!destination.exists() || std::fs::read_dir(&destination).unwrap().count() == 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn recovery_regression_refuses_existing_destination_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let mut entries = members(1);
        entries.push(("vehicle-image-cache/new.webp".into(), b"image".to_vec(), EntryType::Regular));
        write_archive(&package, &entries);
        let destination = root.path().join("out");
        let outside = root.path().join("outside");
        std::fs::create_dir(&destination).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, destination.join("vehicle-image-cache")).unwrap();
        assert!(extract(&package, &destination, &limits()).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        assert!(!destination.join("manifest.json").exists());
    }

    #[test]
    fn recovery_regression_rolls_back_created_directories_on_late_failure() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let mut entries = members(1);
        entries.insert(0, ("vehicle-image-cache/nested/".into(), Vec::new(), EntryType::Directory));
        write_archive(&package, &entries);
        let destination = root.path().join("out");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("database.dump"), b"existing").unwrap();
        assert!(extract(&package, &destination, &limits()).is_err());
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
        assert_eq!(std::fs::read(destination.join("database.dump")).unwrap(), b"existing");
    }
}

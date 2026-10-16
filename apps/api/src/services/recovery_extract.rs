//! Admission-bound streaming extraction into a service-owned recovery directory.
use crate::{config::RecoveryConfig, errors::AppError};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read, Write},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
use tar::Archive;

pub(crate) fn extract(
    package_path: &Path,
    destination: &Path,
    limits: &RecoveryConfig,
) -> Result<(), AppError> {
    let started = Instant::now();
    let admitted =
        crate::services::backups::validate_recovery_package_sync_with_limits(package_path, limits)?;
    extract_admitted(
        package_path,
        destination,
        limits,
        started,
        &admitted.checksum_sha256,
    )
}

fn extract_admitted(
    package: &Path,
    destination: &Path,
    limits: &RecoveryConfig,
    started: Instant,
    checksum: &str,
) -> Result<(), AppError> {
    let source = PackageStream::open(package, limits, started)?;
    let mut meter = MemberAdmission::new(limits, source.file.metadata()?.len());
    let mut archive = Archive::new(GzDecoder::new(source));
    let mut output = ExtractionOutput::new(destination, limits, started)?;
    for entry in archive.entries().map_err(stream_error)? {
        check_deadline(started, limits)?;
        let mut entry = entry.map_err(stream_error)?;
        let member = meter.admit(&entry)?;
        if member.directory {
            output.directory(&member.path)?;
            continue;
        }
        let mut file = output.file(&member.path)?;
        let mut copied = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            check_deadline(started, limits)?;
            let length = entry.read(&mut buffer).map_err(stream_error)?;
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
            file.write_all(&buffer[..length])?;
            copied = copied
                .checked_add(length as u64)
                .ok_or_else(|| invalid("Recovery package expanded size overflowed."))?;
        }
        if copied != member.size {
            return Err(invalid(
                "Recovery package member size changed during extraction.",
            ));
        }
        file.flush()?;
    }
    // Tar EOF can precede gzip's integrity trailer; admission covers every byte.
    let mut decoder = archive.into_inner();
    let mut buffer = [0_u8; 64 * 1024];
    let tail_limit = limits
        .max_expanded_bytes
        .saturating_sub(meter.expanded)
        .saturating_add(buffer.len() as u64);
    let mut tail_bytes = 0_u64;
    loop {
        check_deadline(started, limits)?;
        let length = decoder.read(&mut buffer).map_err(stream_error)?;
        if length == 0 {
            break;
        }
        tail_bytes = tail_bytes
            .checked_add(length as u64)
            .ok_or_else(|| invalid("Recovery package expanded size overflowed."))?;
        if tail_bytes > tail_limit {
            return Err(invalid(
                "Recovery package exceeds the expanded byte limit after tar end.",
            ));
        }
    }
    let mut source = decoder.into_inner();
    while source.read(&mut buffer).map_err(stream_error)? != 0 {}
    if hex::encode(source.digest.finalize()) != checksum {
        return Err(invalid("Recovery package changed after admission."));
    }
    output.committed = true;
    Ok(())
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(message.into())
}

fn check_deadline(started: Instant, limits: &RecoveryConfig) -> Result<(), AppError> {
    if started.elapsed() >= Duration::from_secs(limits.restore_deadline_seconds) {
        Err(AppError::RecoveryDeadline(
            "Recovery extraction deadline exceeded.".into(),
        ))
    } else {
        Ok(())
    }
}

fn stream_error(error: io::Error) -> AppError {
    if error.get_ref().is_some_and(|inner| inner.is::<AppError>()) {
        *error.into_inner().unwrap().downcast::<AppError>().unwrap()
    } else {
        error.into()
    }
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
            return Err(invalid(
                "Recovery package exceeds the configured compressed byte limit.",
            ));
        }
        Ok(Self {
            file,
            digest: Sha256::new(),
            consumed: 0,
            limits,
            started,
        })
    }
}

impl Read for PackageStream<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        check_deadline(self.started, self.limits).map_err(io::Error::other)?;
        let length = self.file.read(buffer)?;
        self.consumed = self
            .consumed
            .checked_add(length as u64)
            .ok_or_else(|| io::Error::other(invalid("Recovery package size overflowed.")))?;
        if self.consumed > self.limits.max_upload_bytes {
            return Err(io::Error::other(invalid(
                "Recovery package exceeds the configured compressed byte limit.",
            )));
        }
        self.digest.update(&buffer[..length]);
        Ok(length)
    }
}

struct AdmittedMember {
    path: String,
    directory: bool,
    size: u64,
}

struct MemberAdmission<'a> {
    limits: &'a RecoveryConfig,
    compressed: u64,
    names: HashSet<String>,
    expanded: u64,
}

impl<'a> MemberAdmission<'a> {
    fn new(limits: &'a RecoveryConfig, compressed: u64) -> Self {
        Self {
            limits,
            compressed,
            names: HashSet::new(),
            expanded: 0,
        }
    }

    fn admit<R: Read>(&mut self, entry: &tar::Entry<'_, R>) -> Result<AdmittedMember, AppError> {
        let path = entry.path().map_err(stream_error)?;
        let normalized = path
            .to_str()
            .ok_or_else(|| invalid("Invalid recovery archive path encoding."))?
            .replace('\\', "/");
        let directory = entry.header().entry_type().is_dir();
        let name = normalized.trim_end_matches('/');
        if (!directory && !entry.header().entry_type().is_file())
            || (!directory && normalized.ends_with('/'))
            || path.is_absolute()
            || name.split('/').any(|part| {
                part.is_empty()
                    || part == "."
                    || part == ".."
                    || part.contains(':')
                    || part.chars().any(char::is_control)
            })
        {
            return Err(invalid("Unsafe recovery archive member."));
        }
        if !self.names.insert(name.to_owned()) {
            return Err(invalid("Recovery package contains duplicate members."));
        }
        if self.names.len() > self.limits.max_members {
            return Err(invalid("Recovery package has too many archive members."));
        }
        if !matches!(
            name,
            "manifest.json"
                | "database.dump"
                | "backup-settings.json"
                | "operational-history.json"
                | "vehicle-image-cache"
        ) && !name.starts_with("vehicle-image-cache/")
        {
            return Err(invalid(format!(
                "Unexpected recovery package member: {name}"
            )));
        }
        let size = entry.size();
        if directory && size != 0 {
            return Err(invalid("Recovery archive directory contains a payload."));
        }
        if size > crate::services::backups::metadata_limit(name, self.limits) {
            return Err(invalid(format!("Recovery package member {name} exceeds its configured metadata or member size limit.")));
        }
        self.expanded = self
            .expanded
            .checked_add(size)
            .ok_or_else(|| invalid("Recovery package expanded size overflowed."))?;
        if self.expanded > self.limits.max_expanded_bytes {
            return Err(invalid("Recovery package exceeds extraction limits."));
        }
        if self.expanded
            > self
                .compressed
                .max(1)
                .saturating_mul(self.limits.max_compression_ratio)
        {
            return Err(invalid(
                "Recovery package exceeds the maximum expansion ratio.",
            ));
        }
        Ok(AdmittedMember {
            path: name.to_owned(),
            directory,
            size,
        })
    }
}

struct OutputPath {
    parent: Option<Rc<OutputPath>>,
    name: String,
}

impl Drop for OutputPath {
    fn drop(&mut self) {
        // Release deep member paths without recursive destruction.
        let mut parent = self.parent.take();
        while let Some(node) = parent {
            let Ok(mut node) = Rc::try_unwrap(node) else {
                break;
            };
            parent = node.parent.take();
        }
    }
}

struct ExtractionOutput<'a> {
    root: Dir,
    // Shared ancestors avoid quadratic storage for deep member paths.
    created: Vec<(Rc<OutputPath>, bool)>,
    committed: bool,
    limits: &'a RecoveryConfig,
    started: Instant,
}

impl<'a> ExtractionOutput<'a> {
    fn new(
        destination: &Path,
        limits: &'a RecoveryConfig,
        started: Instant,
    ) -> Result<Self, AppError> {
        std::fs::create_dir_all(destination)?;
        let root = if let Some(name) = destination.file_name() {
            let parent = destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Dir::open_ambient_dir(parent, ambient_authority())?.open_dir_nofollow(name)?
        } else {
            Dir::open_ambient_dir(destination, ambient_authority())?
        };
        Ok(Self {
            root,
            created: Vec::new(),
            committed: false,
            limits,
            started,
        })
    }

    fn directory(&mut self, path: &str) -> Result<(Dir, Option<Rc<OutputPath>>), AppError> {
        let mut dir = self.root.try_clone()?;
        let mut relative = None;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            check_deadline(self.started, self.limits)?;
            let node = Rc::new(OutputPath {
                parent: relative,
                name: part.to_owned(),
            });
            match dir.create_dir(part) {
                Ok(()) => self.created.push((Rc::clone(&node), true)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            dir = dir.open_dir_nofollow(part)?;
            relative = Some(node);
        }
        Ok((dir, relative))
    }

    fn file(&mut self, path: &str) -> Result<File, AppError> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let (dir, parent) = self.directory(parent)?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = dir.open_with(name, &options).map_err(|error| {
            invalid(format!(
                "Recovery package would overwrite an existing file: {error}"
            ))
        })?;
        self.created.push((
            Rc::new(OutputPath {
                parent,
                name: name.to_owned(),
            }),
            false,
        ));
        Ok(file.into_std())
    }

    fn remove_created(&self, path: &OutputPath, directory: bool) -> io::Result<()> {
        let mut parts = Vec::new();
        let mut next = path.parent.as_deref();
        while let Some(parent) = next {
            parts.push(parent.name.as_str());
            next = parent.parent.as_deref();
        }
        let mut parent = self.root.try_clone()?;
        for part in parts.into_iter().rev() {
            parent = parent.open_dir_nofollow(part)?;
        }
        if directory {
            parent.remove_dir(&path.name)
        } else {
            parent.remove_file(&path.name)
        }
    }
}

impl Drop for ExtractionOutput<'_> {
    fn drop(&mut self) {
        if !self.committed {
            for (path, directory) in self.created.iter().rev() {
                let _ = self.remove_created(path, *directory);
            }
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
            (
                "operational_history",
                "operational-history.json",
                &b"{}"[..],
            ),
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
        result.insert(
            0,
            (
                "manifest.json".into(),
                serde_json::to_vec(&manifest).unwrap(),
                EntryType::Regular,
            ),
        );
        result.push((
            "vehicle-image-cache/".into(),
            Vec::new(),
            EntryType::Directory,
        ));
        result
    }

    pub(crate) fn write_archive(path: &Path, members: &[Member]) {
        let mut archive = Builder::new(GzEncoder::new(
            File::create(path).unwrap(),
            Compression::default(),
        ));
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
        RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        }
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
            assert_eq!(
                std::fs::read(destination.join("database.dump")).unwrap(),
                b"PGDMPsynthetic"
            );
            assert!(destination.join("vehicle-image-cache").is_dir());
            assert!(package.exists());
        }
    }

    #[test]
    fn recovery_contract_rejects_paths_types_and_duplicates_before_writing() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let destination = root.path().join("out");
        for name in [
            "../escape",
            "/escape",
            "C:/escape",
            "vehicle-image-cache/../escape",
            "vehicle-image-cache\\..\\escape",
            "vehicle-image-cache//x",
            "vehicle-image-cache/./x",
            "vehicle-image-cache/x\n",
            "unknown",
            "manifest.json",
        ] {
            let mut entries = members(1);
            entries.push((name.into(), b"x".to_vec(), EntryType::Regular));
            write_archive(&package, &entries);
            assert!(
                extract(&package, &destination, &limits()).is_err(),
                "{name:?}"
            );
            assert!(!destination.exists());
        }
        for kind in [
            EntryType::Symlink,
            EntryType::Link,
            EntryType::Fifo,
            EntryType::Char,
            EntryType::Block,
        ] {
            let mut entries = members(1);
            entries.push(("vehicle-image-cache/link".into(), Vec::new(), kind));
            write_archive(&package, &entries);
            assert!(
                extract(&package, &destination, &limits()).is_err(),
                "{kind:?}"
            );
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
                1 => {
                    entries.remove(0);
                }
                2 => {
                    entries.remove(1);
                }
                3 => entries[1].1 = b"PGDMPdifferent".to_vec(),
                _ => {
                    entries[1].1 = b"plain SQL".to_vec();
                    let mut manifest: serde_json::Value =
                        serde_json::from_slice(&entries[0].1).unwrap();
                    manifest["components"]["database"]["sha256"] =
                        hex::encode(Sha256::digest(&entries[1].1)).into();
                    manifest["components"]["database"]["size_bytes"] =
                        (entries[1].1.len() as u64).into();
                    entries[0].1 = serde_json::to_vec(&manifest).unwrap();
                }
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
            let error = crate::services::backups::classify_recovery_error(
                extract(&package, &destination, &config).unwrap_err(),
            );
            assert!(
                matches!(
                    error,
                    AppError::RecoveryTooLarge(_) | AppError::RecoveryDeadline(_)
                ),
                "{case}: {error:?}"
            );
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
        assert_eq!(
            std::fs::read(destination.join("database.dump")).unwrap(),
            b"existing"
        );
        assert!(!destination.join("manifest.json").exists());
        let mut config = limits();
        config.min_free_bytes = u64::MAX;
        assert!(matches!(
            extract(&package, &root.path().join("reserved"), &config),
            Err(AppError::RecoveryInsufficientStorage(_))
        ));
        assert_eq!(
            std::fs::read_dir(root.path().join("reserved"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn recovery_regression_rejects_missing_or_corrupt_gzip_footer() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        write_archive(&package, &members(1));
        let original = std::fs::read(&package).unwrap();
        for case in 0..2 {
            let mut bytes = original.clone();
            if case == 0 {
                bytes.truncate(bytes.len() - 8);
            } else {
                let end = bytes.len();
                bytes[end - 8] ^= 1;
            }
            std::fs::write(&package, bytes).unwrap();
            let destination = root.path().join(format!("out-{case}"));
            assert!(
                extract(&package, &destination, &limits()).is_err(),
                "footer case {case} accepted"
            );
            assert!(!destination.exists() || std::fs::read_dir(&destination).unwrap().count() == 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn recovery_regression_refuses_existing_destination_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let mut entries = members(1);
        entries.push((
            "vehicle-image-cache/new.webp".into(),
            b"image".to_vec(),
            EntryType::Regular,
        ));
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
        entries.insert(
            0,
            (
                "vehicle-image-cache/nested/".into(),
                Vec::new(),
                EntryType::Directory,
            ),
        );
        write_archive(&package, &entries);
        let destination = root.path().join("out");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("database.dump"), b"existing").unwrap();
        assert!(extract(&package, &destination, &limits()).is_err());
        assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
        assert_eq!(
            std::fs::read(destination.join("database.dump")).unwrap(),
            b"existing"
        );
    }
}

#[cfg(test)]
mod recovery_stream_tests {
    use super::{recovery_validation_fixtures::*, *};
    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, EntryType, Header};

    #[test]
    fn changed_package_after_admission_is_rejected_and_rolled_back() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        write_archive(&package, &members(1));
        let admitted = crate::services::backups::validate_recovery_package_sync_with_limits(
            &package,
            &limits(),
        )
        .unwrap();
        let mut altered = members(1);
        altered.push((
            "vehicle-image-cache/extra".into(),
            b"changed".to_vec(),
            EntryType::Regular,
        ));
        write_archive(&package, &altered);
        let destination = root.path().join("out");
        let result = extract_admitted(
            &package,
            &destination,
            &limits(),
            Instant::now(),
            &admitted.checksum_sha256,
        );
        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("changed after admission"))
        );
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
    }

    #[test]
    fn truncated_member_and_invalid_tar_header_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        write_archive(&package, &members(1));
        let mut tar = Vec::new();
        GzDecoder::new(File::open(&package).unwrap())
            .read_to_end(&mut tar)
            .unwrap();
        for case in 0..2 {
            let mut bytes = tar.clone();
            if case == 0 {
                bytes.truncate(600);
            } else {
                bytes[0] ^= 1;
            }
            let mut encoder =
                GzEncoder::new(File::create(&package).unwrap(), Compression::default());
            encoder.write_all(&bytes).unwrap();
            encoder.finish().unwrap();
            assert!(extract(
                &package,
                &root.path().join(format!("out-{case}")),
                &limits()
            )
            .is_err());
        }
    }

    #[test]
    fn gnu_long_artwork_paths_preserve_compatibility() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let name = format!("vehicle-image-cache/{}image.webp", "nested/".repeat(25));
        let mut archive = Builder::new(GzEncoder::new(
            File::create(&package).unwrap(),
            Compression::default(),
        ));
        let mut entries = members(1);
        entries.push((name.clone(), b"image".to_vec(), EntryType::Regular));
        for (path, bytes, kind) in entries {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_entry_type(kind);
            header.set_cksum();
            archive.append_data(&mut header, path, &bytes[..]).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        let destination = root.path().join("out");
        extract(&package, &destination, &limits()).unwrap();
        assert_eq!(std::fs::read(destination.join(name)).unwrap(), b"image");
    }

    #[test]
    fn member_admission_preserves_exact_resource_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let entries = members(1);
        write_archive(&package, &entries);
        let mut config = limits();
        config.max_upload_bytes = std::fs::metadata(&package).unwrap().len();
        config.max_members = entries.len();
        config.max_expanded_bytes = entries.iter().map(|(_, bytes, _)| bytes.len() as u64).sum();
        config.max_member_bytes = entries
            .iter()
            .map(|(_, bytes, _)| bytes.len() as u64)
            .max()
            .unwrap();
        config.max_manifest_bytes = entries[0].1.len() as u64;
        config.max_settings_bytes = 2;
        config.max_history_bytes = 2;
        config.max_compression_ratio = config.max_expanded_bytes.div_ceil(config.max_upload_bytes);
        extract(&package, &root.path().join("out"), &config).unwrap();
    }

    #[test]
    fn ambiguous_directories_and_directory_payloads_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        for case in 0..2 {
            let mut entries = members(1);
            if case == 0 {
                entries.push((
                    "vehicle-image-cache".into(),
                    Vec::new(),
                    EntryType::Directory,
                ));
            } else {
                entries.last_mut().unwrap().1 = b"payload".to_vec();
            }
            write_archive(&package, &entries);
            let destination = root.path().join(format!("out-{case}"));
            assert!(extract(&package, &destination, &limits()).is_err());
            assert!(!destination.exists() || std::fs::read_dir(destination).unwrap().count() == 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn rollback_does_not_follow_replaced_parent_and_rejects_symlinked_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let destination = root.path().join("out");
        let config = limits();
        let mut output = ExtractionOutput::new(&destination, &config, Instant::now()).unwrap();
        output.file("vehicle-image-cache/image").unwrap();
        let original = destination.join("vehicle-image-cache");
        std::fs::rename(&original, destination.join("moved")).unwrap();
        std::fs::write(outside.path().join("image"), b"untouched").unwrap();
        std::os::unix::fs::symlink(outside.path(), &original).unwrap();
        drop(output);
        assert_eq!(
            std::fs::read(outside.path().join("image")).unwrap(),
            b"untouched"
        );
        let linked = root.path().join("linked");
        std::os::unix::fs::symlink(outside.path(), &linked).unwrap();
        assert!(ExtractionOutput::new(&linked, &config, Instant::now()).is_err());
    }

    #[test]
    fn input_growth_cannot_bypass_compressed_byte_limit() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        std::fs::write(&package, b"1234").unwrap();
        let mut config = limits();
        config.max_upload_bytes = 4;
        let mut stream = PackageStream::open(&package, &config, Instant::now()).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&package)
            .unwrap();
        file.write_all(b"5").unwrap();
        let error = stream_error(stream.read(&mut [0_u8; 8]).unwrap_err());
        assert!(matches!(
            crate::services::backups::classify_recovery_error(error),
            AppError::RecoveryTooLarge(_)
        ));
    }
}

#[cfg(test)]
mod recovery_path_encoding_tests {
    use super::{recovery_validation_fixtures::*, *};
    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, EntryType, Header};

    #[test]
    fn portable_archive_separators_use_the_admitted_directory_tree() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let mut entries = members(1);
        entries.push((
            "vehicle-image-cache\\nested\\image.webp".into(),
            b"image".to_vec(),
            EntryType::Regular,
        ));
        write_archive(&package, &entries);
        let destination = root.path().join("out");
        extract(&package, &destination, &limits()).unwrap();
        assert_eq!(
            std::fs::read(destination.join("vehicle-image-cache/nested/image.webp")).unwrap(),
            b"image"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_archive_names_are_rejected_without_lossy_renaming() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let mut archive = Builder::new(GzEncoder::new(
            File::create(&package).unwrap(),
            Compression::default(),
        ));
        for (name, bytes, kind) in members(1) {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_entry_type(kind);
            header.set_cksum();
            archive.append_data(&mut header, name, &bytes[..]).unwrap();
        }
        let mut header = Header::new_gnu();
        let name = b"vehicle-image-cache/invalid-\xff";
        header.as_mut_bytes()[..name.len()].copy_from_slice(name);
        header.set_size(0);
        header.set_cksum();
        archive.append(&header, io::empty()).unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        let destination = root.path().join("out");
        assert!(
            matches!(extract(&package, &destination, &limits()), Err(AppError::Validation(message)) if message.contains("encoding"))
        );
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod recovery_tail_budget_tests {
    use super::{recovery_validation_fixtures::*, *};
    use flate2::{write::GzEncoder, Compression};

    #[test]
    fn post_tar_expansion_is_bounded_and_padding_remains_compatible() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let entries = members(1);
        write_archive(&package, &entries);
        let mut tar = Vec::new();
        GzDecoder::new(File::open(&package).unwrap())
            .read_to_end(&mut tar)
            .unwrap();
        let mut config = limits();
        config.max_expanded_bytes = entries.iter().map(|(_, bytes, _)| bytes.len() as u64).sum();
        for padding in [8 * 1024, 128 * 1024] {
            let mut encoder =
                GzEncoder::new(File::create(&package).unwrap(), Compression::default());
            encoder.write_all(&tar).unwrap();
            encoder.write_all(&vec![0_u8; padding]).unwrap();
            encoder.finish().unwrap();
            let destination = root.path().join(format!("out-{padding}"));
            let result = extract(&package, &destination, &config);
            if padding == 8 * 1024 {
                result.unwrap();
            } else {
                let error = crate::services::backups::classify_recovery_error(result.unwrap_err());
                assert!(
                    matches!(error, AppError::RecoveryTooLarge(message) if message.contains("after tar end"))
                );
                assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
            }
        }
    }
}

#[cfg(test)]
mod recovery_output_resource_tests {
    use super::{recovery_validation_fixtures::*, *};

    #[test]
    fn deep_rollback_path_ownership_does_not_overflow_the_stack() {
        let mut path = None;
        for _ in 0..50_000 {
            path = Some(Rc::new(OutputPath {
                parent: path,
                name: "part".into(),
            }));
        }
        drop(path);
    }

    #[test]
    fn directory_creation_observes_the_deadline_before_each_component() {
        let root = tempfile::tempdir().unwrap();
        let mut config = limits();
        config.restore_deadline_seconds = 0;
        let mut output = ExtractionOutput::new(root.path(), &config, Instant::now()).unwrap();
        assert!(matches!(
            output.directory("vehicle-image-cache/nested"),
            Err(AppError::RecoveryDeadline(_))
        ));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

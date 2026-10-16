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
#[path = "__tests__/recovery_extract_recovery_validation_fixtures.rs"]
pub(crate) mod recovery_validation_fixtures;

#[cfg(test)]
#[path = "__tests__/recovery_extract_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "__tests__/recovery_extract_recovery_stream_tests.rs"]
mod recovery_stream_tests;

#[cfg(test)]
#[path = "__tests__/recovery_extract_recovery_path_encoding_tests.rs"]
mod recovery_path_encoding_tests;

#[cfg(test)]
#[path = "__tests__/recovery_extract_recovery_tail_budget_tests.rs"]
mod recovery_tail_budget_tests;

#[cfg(test)]
#[path = "__tests__/recovery_extract_recovery_output_resource_tests.rs"]
mod recovery_output_resource_tests;

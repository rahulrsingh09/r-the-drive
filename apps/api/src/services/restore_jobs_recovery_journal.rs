//! A bounded journal transaction owns its lock, staging file and publication.
use super::RestoreJob;
use crate::{config::Config, errors::AppError};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, DirBuilder, OpenOptions},
};
use fs2::FileExt;
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct Journal {
    directory: Dir,
    maximum: u64,
    deadline: Duration,
    started: Instant,
}

impl Journal {
    fn open(config: &Config, create: bool) -> Result<Self, AppError> {
        let root = Path::new(&config.backup_artifact_dir);
        if create {
            std::fs::create_dir_all(root)?;
        }
        let root = Dir::open_ambient_dir(root, ambient_authority()).map_err(read_error)?;
        if create {
            let mut builder = DirBuilder::new();
            #[cfg(unix)]
            {
                use cap_std::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match root.create_dir_with(".restore-jobs", &builder) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let directory = root
            .open_dir_nofollow(".restore-jobs")
            .map_err(read_error)?;
        Ok(Self {
            directory,
            maximum: config
                .recovery
                .max_history_bytes
                .saturating_add(config.recovery.max_manifest_bytes.saturating_mul(4))
                .saturating_add(64 * 1024),
            deadline: Duration::from_secs(config.recovery.restore_deadline_seconds),
            started: Instant::now(),
        })
    }

    fn check_deadline(&self) -> Result<(), AppError> {
        if self.started.elapsed() >= self.deadline {
            return Err(AppError::RecoveryDeadline(
                "Restore journal deadline exceeded.".into(),
            ));
        }
        Ok(())
    }

    fn lock(&self, id: Uuid) -> Result<File, AppError> {
        let mut options = private_options();
        options.read(true).write(true).create(true);
        let file = self
            .directory
            .open_with(format!(".{id}.lock"), &options)?
            .into_std();
        if !file.metadata()?.is_file() {
            return Err(AppError::Validation(
                "Restore journal lock is not a regular file.".into(),
            ));
        }
        loop {
            self.check_deadline()?;
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn read(&self, id: Uuid) -> Result<RestoreJob, AppError> {
        self.check_deadline()?;
        let mut options = private_options();
        options.read(true);
        let mut file = self
            .directory
            .open_with(format!("{id}.json"), &options)
            .map_err(read_error)?;
        if !file.metadata()?.is_file() {
            return Err(AppError::Validation(
                "Restore journal record is not a regular file.".into(),
            ));
        }
        if file.metadata()?.len() > self.maximum {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            self.check_deadline()?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if (bytes.len() as u64).saturating_add(count as u64) > self.maximum {
                return Err(too_large());
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let job: RestoreJob = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Internal(anyhow::anyhow!("invalid restore job record: {error}"))
        })?;
        if job.id != id {
            return Err(AppError::Validation(
                "Restore journal identity does not match its file.".into(),
            ));
        }
        Ok(job)
    }

    fn publish(&self, job: &RestoreJob) -> Result<(), AppError> {
        self.check_deadline()?;
        let name = format!(".{}.{}.tmp", job.id, Uuid::new_v4());
        let mut options = private_options();
        options.write(true).create_new(true);
        let file = self.directory.open_with(&name, &options)?.into_std();
        let pending = Pending {
            directory: &self.directory,
            name,
            file,
        };
        let mut writer = JournalWriter {
            journal: self,
            file: &pending.file,
            written: 0,
        };
        if let Err(error) = serde_json::to_writer_pretty(&mut writer, job) {
            if writer.written > self.maximum {
                return Err(too_large());
            }
            self.check_deadline()?;
            return Err(AppError::Internal(anyhow::anyhow!(
                "restore journal serialization failed: {error}"
            )));
        }
        pending.file.sync_all()?;
        self.check_deadline()?;
        self.directory
            .rename(&pending.name, &self.directory, format!("{}.json", job.id))?;
        Ok(())
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn read_error(error: io::Error) -> AppError {
    if error.kind() == io::ErrorKind::NotFound {
        AppError::NotFound
    } else {
        error.into()
    }
}

fn too_large() -> AppError {
    AppError::RecoveryTooLarge("Restore journal exceeds the configured metadata budget.".into())
}

struct Pending<'a> {
    directory: &'a Dir,
    name: String,
    file: File,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        let _ = self.directory.remove_file(&self.name);
    }
}

struct JournalWriter<'a> {
    journal: &'a Journal,
    file: &'a File,
    written: u64,
}

impl Write for JournalWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.journal.check_deadline().map_err(io::Error::other)?;
        let requested = self.written.saturating_add(bytes.len() as u64);
        if requested > self.journal.maximum {
            self.written = requested;
            return Err(io::Error::other(too_large()));
        }
        let count = self.file.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

pub(super) async fn read(config: &Config, id: Uuid) -> Result<RestoreJob, AppError> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || Journal::open(&config, false)?.read(id))
        .await
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
}

pub(super) async fn write(config: &Config, job: &RestoreJob) -> Result<(), AppError> {
    let config = config.clone();
    let job = job.clone();
    tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&config, true)?;
        let _lock = journal.lock(job.id)?;
        journal.publish(&job)
    })
    .await
    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
}

pub(super) async fn mutate(
    config: &Config,
    id: Uuid,
    change: impl FnOnce(&mut RestoreJob) + Send + 'static,
) -> Result<RestoreJob, AppError> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&config, false)?;
        let _lock = journal.lock(id)?;
        let mut job = journal.read(id)?;
        change(&mut job);
        job.updated_at = chrono::Utc::now();
        journal.publish(&job)?;
        Ok(job)
    })
    .await
    .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?
}

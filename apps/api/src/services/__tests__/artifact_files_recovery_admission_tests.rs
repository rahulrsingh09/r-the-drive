use super::*;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

struct PartialFailure {
    sent: bool,
}
impl AsyncRead for PartialFailure {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.sent {
            Poll::Ready(Err(io::Error::other("synthetic reader failure")))
        } else {
            self.sent = true;
            buffer.put_slice(b"partial");
            Poll::Ready(Ok(()))
        }
    }
}

#[tokio::test]
async fn exact_byte_limit_names_and_empty_stream_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let limits = crate::config::RecoveryConfig {
        max_upload_bytes: 7,
        min_free_bytes: 0,
        ..Default::default()
    };
    let id = uuid::Uuid::new_v4();
    let path = stage_reader(root.path(), id, &mut &b"1234567"[..], &limits)
        .await
        .unwrap();
    assert_eq!(
        path,
        root.path()
            .join(".remote-staging")
            .join(format!("{id}.rma.tar.gz"))
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"1234567");
    remove_staged(root.path(), id).unwrap();
    let path = stage_reader(root.path(), id, &mut tokio::io::empty(), &limits)
        .await
        .unwrap();
    assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
    let upload = StagingFile::upload(root.path(), id).unwrap();
    assert!(upload.file_name().starts_with("import-"));
    assert!(upload
        .file_name()
        .ends_with(&format!("-{}.rma.tar.gz", id.simple())));
    assert_eq!(upload.path.parent().unwrap(), root.path().join("imports"));
}

#[tokio::test]
async fn input_error_and_cancelled_copy_leave_no_partial_or_final_file() {
    let root = tempfile::tempdir().unwrap();
    let limits = crate::config::RecoveryConfig {
        min_free_bytes: 0,
        ..Default::default()
    };
    let id = uuid::Uuid::new_v4();
    let error = stage_reader(
        root.path(),
        id,
        &mut PartialFailure { sent: false },
        &limits,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AppError::Io(_)));
    assert_eq!(
        std::fs::read_dir(root.path().join(".remote-staging"))
            .unwrap()
            .count(),
        0
    );
    let (mut source, mut writer) = tokio::io::duplex(64);
    use tokio::io::AsyncWriteExt;
    writer.write_all(b"partial").await.unwrap();
    let work = stage_reader(root.path(), id, &mut source, &limits);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), work)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_dir(root.path().join(".remote-staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn admitted_paths_resolve_relative_absolute_and_portable_names() {
    let root = tempfile::tempdir().unwrap();
    let relative = Path::new("prefix/fixture.rma.tar.gz");
    create(root.path(), relative).unwrap();
    assert!(open(root.path(), relative).is_ok());
    assert!(open(root.path(), &root.path().join(relative)).is_ok());
    for prefix in [
        "com1.log",
        "LPT9",
        "nul.json",
        "a/b.",
        "a/b /x",
        "a/b?c",
        "a/b*c",
        "a/b|c",
        "a/b<c",
        "a/b>c",
        "a/b\"c",
        "a/.hidden",
    ] {
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
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        1
    );
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

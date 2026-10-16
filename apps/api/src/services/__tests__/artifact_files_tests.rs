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

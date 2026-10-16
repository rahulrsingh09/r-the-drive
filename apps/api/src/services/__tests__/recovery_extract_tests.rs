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

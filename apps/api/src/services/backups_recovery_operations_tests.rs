use super::*;
use std::io::Write;
use tar::{EntryType, Header};

fn limits() -> RecoveryConfig {
    RecoveryConfig {
        min_free_bytes: 0,
        max_compression_ratio: 1000,
        ..Default::default()
    }
}

fn members(version: u64) -> Vec<(String, Vec<u8>, EntryType)> {
    let component = |path: &str, data: &[u8]| {
        json!({
            "path": path, "sha256": hex::encode(Sha256::digest(data)),
            "size_bytes": data.len(), "version": 1,
            "restore_policy": "synthetic", "redactions": []
        })
    };
    let manifest = json!({
        "format": format!("panda-recovery-v{version}"), "format_version": version,
        "source": {
            "app_version": "synthetic", "postgres_major": 18, "timescale_version": "2.28.3",
            "migration_version": restore_compatibility::latest_migration_version(),
            "migration_ledger": restore_compatibility::compiled_migration_ledger(),
            "migration_chain_id": migrations::MIGRATION_CHAIN_ID,
            "migration_catalog_digest": migrations::migration_catalog_digest(),
            "schema_contract_version": restore_compatibility::SCHEMA_CONTRACT_VERSION,
            "schema_fingerprint": "synthetic"
        },
        "restore": {"engine_version": restore_compatibility::RESTORE_ENGINE_VERSION},
        "components": {
            "database": component("database.dump", b"PGDMPsynthetic"),
            "backup_settings": component("backup-settings.json", b"{}"),
            "operational_history": component("operational-history.json", b"{}"),
            "vehicle_image_cache": {
                "path": "vehicle-image-cache", "files": [], "size_bytes": 0,
                "file_count": 0, "sha256": hex::encode(Sha256::digest(b"[]")),
                "version": 1, "restore_policy": "replace_atomically", "redactions": []
            }
        }
    });
    vec![
        (
            "manifest.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
            EntryType::Regular,
        ),
        (
            "database.dump".into(),
            b"PGDMPsynthetic".to_vec(),
            EntryType::Regular,
        ),
        (
            "backup-settings.json".into(),
            b"{}".to_vec(),
            EntryType::Regular,
        ),
        (
            "operational-history.json".into(),
            b"{}".to_vec(),
            EntryType::Regular,
        ),
        ("vehicle-image-cache/".into(), vec![], EntryType::Directory),
    ]
}

fn write_archive(path: &Path, entries: &[(String, Vec<u8>, EntryType)]) {
    let mut archive = Builder::new(GzEncoder::new(
        File::create(path).unwrap(),
        Compression::default(),
    ));
    for (path, contents, kind) in entries {
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_entry_type(*kind);
        header.set_mode(0o600);
        if path.len() <= 100 {
            header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            archive.append(&header, &contents[..]).unwrap();
        } else {
            archive
                .append_data(&mut header, path, &contents[..])
                .unwrap();
        }
    }
    archive.into_inner().unwrap().finish().unwrap();
}

#[test]
fn admission_characterizes_all_formats_and_entire_compressed_checksum() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    for version in 1..=3 {
        let entries = members(version);
        write_archive(&path, &entries);
        File::options()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"suffix")
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let result = validate_recovery_package_sync_with_limits(&path, &limits()).unwrap();
        assert_eq!(
            result.manifest,
            serde_json::from_slice::<serde_json::Value>(&entries[0].1).unwrap()
        );
        assert_eq!(result.checksum_sha256, hex::encode(Sha256::digest(&bytes)));
        assert_eq!(result.size_bytes, bytes.len() as i64);
    }
}

#[test]
fn admission_characterizes_component_and_migration_failures() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    for case in 0..9 {
        let mut entries = members(3);
        let mut manifest: serde_json::Value = serde_json::from_slice(&entries[0].1).unwrap();
        match case {
            0 => manifest["format_version"] = 2.into(),
            1 => manifest["components"]["database"]["sha256"] = "wrong".into(),
            2 => manifest["components"]["backup_settings"]["size_bytes"] = 55.into(),
            3 => manifest["components"]["operational_history"]["restore_policy"] = "".into(),
            4 => manifest["source"]["migration_version"] = 0.into(),
            5 => manifest["source"]["migration_ledger"] = json!([]),
            6 => manifest["source"]["app_version"] = "".into(),
            7 => manifest["restore"]["engine_version"] = 999.into(),
            _ => {
                entries[1].1 = b"plain SQL".to_vec();
                manifest["components"]["database"]["sha256"] =
                    hex::encode(Sha256::digest(&entries[1].1)).into();
                manifest["components"]["database"]["size_bytes"] = entries[1].1.len().into();
            }
        }
        entries[0].1 = serde_json::to_vec(&manifest).unwrap();
        write_archive(&path, &entries);
        assert!(
            validate_recovery_package_sync_with_limits(&path, &limits()).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn admission_characterizes_limits_at_and_above_the_boundary() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    let entries = members(1);
    write_archive(&path, &entries);
    let mut exact = limits();
    exact.max_upload_bytes = std::fs::metadata(&path).unwrap().len();
    exact.max_members = entries.len();
    exact.max_expanded_bytes = entries.iter().map(|(_, bytes, _)| bytes.len() as u64).sum();
    exact.max_member_bytes = entries[0].1.len() as u64;
    exact.max_manifest_bytes = exact.max_member_bytes;
    exact.max_settings_bytes = 2;
    exact.max_history_bytes = 2;
    validate_recovery_package_sync_with_limits(&path, &exact).unwrap();
    for case in 0..9 {
        let mut config = exact.clone();
        match case {
            0 => config.max_upload_bytes -= 1,
            1 => config.max_members -= 1,
            2 => config.max_expanded_bytes -= 1,
            3 => config.max_member_bytes -= 1,
            4 => config.max_manifest_bytes -= 1,
            5 => config.max_settings_bytes -= 1,
            6 => config.max_history_bytes -= 1,
            7 => config.max_compression_ratio = 0,
            _ => config.restore_deadline_seconds = 0,
        }
        let error = classify_recovery_error(
            validate_recovery_package_sync_with_limits(&path, &config).unwrap_err(),
        );
        assert!(
            matches!(
                error,
                AppError::RecoveryTooLarge(_) | AppError::RecoveryDeadline(_)
            ),
            "{case}: {error:?}"
        );
    }
}

#[test]
fn admission_regression_missing_or_corrupt_gzip_trailer() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    write_archive(&path, &members(1));
    let original = std::fs::read(&path).unwrap();
    for case in 0..2 {
        let mut bytes = original.clone();
        if case == 0 {
            bytes.truncate(bytes.len() - 8);
        } else {
            let end = bytes.len();
            bytes[end - 8] ^= 1;
        }
        std::fs::write(&path, bytes).unwrap();
        assert!(
            validate_recovery_package_sync_with_limits(&path, &limits()).is_err(),
            "standalone admission accepted gzip trailer case {case}"
        );
    }
}

#[test]
fn admission_regression_bounds_post_tar_expansion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    let entries = members(1);
    write_archive(&path, &entries);
    let mut raw = Vec::new();
    GzDecoder::new(File::open(&path).unwrap())
        .read_to_end(&mut raw)
        .unwrap();
    let mut config = limits();
    config.max_expanded_bytes = entries.iter().map(|(_, bytes, _)| bytes.len() as u64).sum();
    for tail in [8192, 128 * 1024] {
        let mut encoder = GzEncoder::new(File::create(&path).unwrap(), Compression::default());
        encoder.write_all(&raw).unwrap();
        encoder.write_all(&vec![0; tail]).unwrap();
        encoder.finish().unwrap();
        let result = validate_recovery_package_sync_with_limits(&path, &config);
        if tail == 8192 {
            result.unwrap();
        } else {
            assert!(
                matches!(
                    result.map_err(classify_recovery_error),
                    Err(AppError::RecoveryTooLarge(_))
                ),
                "unbounded post-tar expansion was accepted"
            );
        }
    }
}

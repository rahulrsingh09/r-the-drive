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

#[test]
fn admission_regression_bounds_hidden_gnu_and_pax_metadata() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    for kind in [
        EntryType::GNULongName,
        EntryType::GNULongLink,
        EntryType::XHeader,
    ] {
        let mut archive = Builder::new(GzEncoder::new(
            File::create(&path).unwrap(),
            Compression::default(),
        ));
        for (name, bytes, kind) in members(1) {
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_entry_type(kind);
            archive.append_data(&mut header, name, &bytes[..]).unwrap();
        }
        if kind == EntryType::XHeader {
            archive
                .append_pax_extensions([("comment", &vec![b'x'; 128 * 1024][..])])
                .unwrap();
        } else {
            let mut bytes = format!("vehicle-image-cache/{}", "a".repeat(128 * 1024)).into_bytes();
            bytes.push(0);
            let mut header = Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_entry_type(kind);
            archive
                .append_data(&mut header, "././@LongLink", &bytes[..])
                .unwrap();
        }
        let mut header = Header::new_gnu();
        header.set_size(1);
        archive
            .append_data(&mut header, "vehicle-image-cache/extra", &b"x"[..])
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        assert!(
            matches!(
                validate_recovery_package_sync_with_limits(&path, &limits())
                    .map_err(classify_recovery_error),
                Err(AppError::RecoveryTooLarge(_))
            ),
            "unbounded extension {kind:?} accepted"
        );
    }
}

#[test]
fn admission_regression_rejects_directory_payloads_and_aliases() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    for case in 0..2 {
        let mut entries = members(1);
        if case == 0 {
            entries.last_mut().unwrap().1 = b"hidden payload".to_vec();
        } else {
            entries.push(("vehicle-image-cache".into(), vec![], EntryType::Directory));
        }
        write_archive(&path, &entries);
        assert!(
            validate_recovery_package_sync_with_limits(&path, &limits()).is_err(),
            "ambiguous directory case {case} accepted"
        );
    }
}

#[test]
fn admission_stream_binds_manifest_and_hash_when_the_source_is_replaced() {
    struct ChangingFile {
        source: File,
        path: PathBuf,
        replacement: Vec<u8>,
        changed: bool,
    }
    impl Read for ChangingFile {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = self.source.read(buffer)?;
            if !self.changed {
                assert!(
                    count >= self.replacement.len(),
                    "fixture must fit in the first compressed read"
                );
                std::fs::write(&self.path, &self.replacement)?;
                self.changed = true;
            }
            Ok(count)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    write_archive(&path, &members(2));
    let mut replacement = std::fs::read(&path).unwrap();
    write_archive(&path, &members(1));
    let mut original = std::fs::read(&path).unwrap();
    let length = original.len().max(replacement.len());
    original.resize(length, 0);
    replacement.resize(length, 0);
    std::fs::write(&path, &original).unwrap();
    let input = ChangingFile {
        source: File::open(&path).unwrap(),
        path: path.clone(),
        replacement,
        changed: false,
    };
    let validated = validate_recovery_reader(input, length as u64, &limits()).unwrap();
    assert_eq!(validated.manifest["format_version"], 1);
    assert_eq!(
        validated.checksum_sha256,
        hex::encode(Sha256::digest(original))
    );
    let replaced = validate_recovery_package_sync_with_limits(&path, &limits()).unwrap();
    assert_eq!(replaced.manifest["format_version"], 2);
    assert_ne!(validated.checksum_sha256, replaced.checksum_sha256);
}

#[test]
fn admission_rejects_huge_extension_headers_before_reading_their_payload() {
    for kind in [
        EntryType::GNULongName,
        EntryType::GNULongLink,
        EntryType::XHeader,
    ] {
        let mut header = Header::new_gnu();
        header.set_size(1 << 40);
        header.set_entry_type(kind);
        header.set_cksum();
        let config = limits();
        let source = Cursor::new(header.as_bytes());
        let envelope = TarEnvelope::new(source, &config, std::time::Instant::now());
        let mut archive = Archive::new(envelope);
        let error = archive.entries().unwrap().next().unwrap().err().unwrap();
        assert!(
            matches!(stream_error(error), AppError::Validation(message) if message.contains("extension limit"))
        );
    }
}

#[test]
fn admission_preserves_pax_paths_and_effective_sizes_across_gnu_metadata() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    let mut archive = Builder::new(GzEncoder::new(
        File::create(&path).unwrap(),
        Compression::default(),
    ));
    for (name, bytes, kind) in members(1) {
        let mut header = Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_entry_type(kind);
        archive.append_data(&mut header, name, &bytes[..]).unwrap();
    }
    archive
        .append_pax_extensions([
            ("path", &b"vehicle-image-cache/pax"[..]),
            ("size", &b"3"[..]),
        ])
        .unwrap();
    let mut long = Header::new_gnu();
    long.set_entry_type(EntryType::GNULongName);
    let long_name = b"vehicle-image-cache/long-name\0";
    long.set_size(long_name.len() as u64);
    archive
        .append_data(&mut long, "././@LongLink", &long_name[..])
        .unwrap();
    let mut header = Header::new_gnu();
    header.set_size(3);
    archive
        .append_data(&mut header, "ignored", &b"xyz"[..])
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    validate_recovery_package_sync_with_limits(&path, &limits()).unwrap();
}

#[test]
fn admission_truncation_and_input_growth_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package.rma.tar.gz");
    write_archive(&path, &members(1));
    let original = std::fs::read(&path).unwrap();
    for length in [0, 1, 10, original.len() / 2, original.len() - 1] {
        assert!(
            validate_recovery_reader(Cursor::new(&original[..length]), length as u64, &limits())
                .is_err(),
            "{length}"
        );
    }
    let mut config = limits();
    config.max_upload_bytes = original.len() as u64 - 1;
    let input = CompressedInput::new(Cursor::new(original), &config, std::time::Instant::now());
    assert!(matches!(
        input
            .finish(serde_json::Value::Null)
            .map_err(classify_recovery_error),
        Err(AppError::RecoveryTooLarge(_))
    ));
}

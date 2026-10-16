use super::{recovery_validation_fixtures::*, *};
use flate2::{write::GzEncoder, Compression};
use tar::{Builder, EntryType, Header};

#[test]
fn changed_package_after_admission_is_rejected_and_rolled_back() {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("fixture.rma.tar.gz");
    write_archive(&package, &members(1));
    let admitted =
        crate::services::backups::validate_recovery_package_sync_with_limits(&package, &limits())
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
        let mut encoder = GzEncoder::new(File::create(&package).unwrap(), Compression::default());
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

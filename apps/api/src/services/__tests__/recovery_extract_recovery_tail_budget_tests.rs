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
        let mut encoder = GzEncoder::new(File::create(&package).unwrap(), Compression::default());
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
            match std::fs::symlink_metadata(&destination) {
                Ok(metadata) => {
                    assert!(metadata.is_dir());
                    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
                }
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
            }
        }
    }
}

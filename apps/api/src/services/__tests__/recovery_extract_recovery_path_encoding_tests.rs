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
    match std::fs::symlink_metadata(&destination) {
        Ok(metadata) => {
            assert!(metadata.is_dir());
            assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
        }
        Err(error) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
    }
}

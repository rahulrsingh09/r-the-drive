use super::*;
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tar::{Builder, EntryType, Header};

pub(crate) type Member = (String, Vec<u8>, EntryType);

pub(crate) fn members(version: u64) -> Vec<Member> {
    let mut manifest = serde_json::json!({
        "format": format!("panda-recovery-v{version}"), "format_version": version,
        "source": {
            "postgres_major": 17, "migration_version": 1,
            "migration_ledger": [{"version": 1, "checksum_sha384": "synthetic"}],
            "schema_fingerprint": "synthetic", "migration_chain_id": "synthetic",
            "migration_catalog_digest": "synthetic", "schema_contract_version": "1",
            "app_version": "synthetic"
        },
        "restore": {"engine_version": 1}, "components": {}
    });
    let mut result = Vec::new();
    for (component, path, bytes) in [
        ("database", "database.dump", &b"PGDMPsynthetic"[..]),
        ("backup_settings", "backup-settings.json", &b"{}"[..]),
        (
            "operational_history",
            "operational-history.json",
            &b"{}"[..],
        ),
    ] {
        manifest["components"][component] = serde_json::json!({
            "version": 1, "restore_policy": "replace", "redactions": [],
            "path": path, "size_bytes": bytes.len(), "sha256": hex::encode(Sha256::digest(bytes))
        });
        result.push((path.to_owned(), bytes.to_vec(), EntryType::Regular));
    }
    let files = serde_json::json!([]);
    manifest["components"]["vehicle_image_cache"] = serde_json::json!({
        "version": 1, "restore_policy": "replace", "redactions": [],
        "files": files, "file_count": 0, "size_bytes": 0,
        "sha256": hex::encode(Sha256::digest(serde_json::to_vec(&files).unwrap()))
    });
    result.insert(
        0,
        (
            "manifest.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
            EntryType::Regular,
        ),
    );
    result.push((
        "vehicle-image-cache/".into(),
        Vec::new(),
        EntryType::Directory,
    ));
    result
}

pub(crate) fn write_archive(path: &Path, members: &[Member]) {
    let mut archive = Builder::new(GzEncoder::new(
        File::create(path).unwrap(),
        Compression::default(),
    ));
    for (name, bytes, kind) in members {
        let mut header = Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        assert!(name.len() < 100);
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        archive.append(&header, &bytes[..]).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

pub(crate) fn limits() -> RecoveryConfig {
    RecoveryConfig {
        min_free_bytes: 0,
        ..Default::default()
    }
}

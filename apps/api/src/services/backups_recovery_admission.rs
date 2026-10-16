//! One bounded inventory supplies all manifest evidence.
use super::{check_deadline, metadata_limit, stream_error, RecoveryConfig};
use crate::errors::AppError;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, io::Read, time::Instant};
use tar::Archive;

enum Member {
    Metadata,
    File { size: u64, checksum: String },
}

pub(super) struct RecoveryInventory {
    members: HashMap<String, Member>,
    manifest: Option<Vec<u8>>,
    database_magic: Vec<u8>,
}

impl RecoveryInventory {
    pub(super) fn read<R: Read>(
        archive: &mut Archive<R>,
        limits: &RecoveryConfig,
        compressed: u64,
        started: Instant,
    ) -> Result<Self, AppError> {
        let mut inventory = Self {
            members: HashMap::new(),
            manifest: None,
            database_magic: Vec::with_capacity(5),
        };
        let mut expanded = 0_u64;
        let mut path_bytes = 0_usize;
        let mut buffer = [0; 64 * 1024];
        for entry in archive.entries().map_err(stream_error)? {
            check_deadline(started, limits)?;
            let mut entry = entry.map_err(stream_error)?;
            let path = entry.path().map_err(stream_error)?;
            let normalized = path
                .to_str()
                .ok_or_else(|| invalid("Invalid recovery archive path encoding."))?
                .replace('\\', "/");
            path_bytes = path_bytes
                .checked_add(normalized.len())
                .ok_or_else(|| invalid("Recovery path size overflowed."))?;
            if path_bytes as u64 > limits.max_manifest_bytes {
                return Err(invalid(
                    "Recovery package exceeds the configured metadata limit for archive paths.",
                ));
            }
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() {
                return Err(invalid(format!(
                    "Recovery package member {normalized} has unsupported archive type {}.",
                    kind.as_byte()
                )));
            }
            let name = normalized.trim_end_matches('/');
            if (kind.is_file() && normalized.ends_with('/'))
                || path.is_absolute()
                || normalized.starts_with('/')
                || name.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || part.contains(':')
                        || part.chars().any(char::is_control)
                })
            {
                return Err(invalid(format!(
                    "Unsafe recovery package path: {normalized}"
                )));
            }
            if inventory.members.len() >= limits.max_members {
                return Err(invalid(format!(
                    "Recovery package exceeds the {} member limit.",
                    limits.max_members
                )));
            }
            if inventory.members.contains_key(name) {
                return Err(invalid(format!(
                    "Recovery package contains duplicate member {normalized}."
                )));
            }
            if !matches!(
                normalized.as_str(),
                "manifest.json"
                    | "database.dump"
                    | "backup-settings.json"
                    | "operational-history.json"
                    | "vehicle-image-cache"
            ) && !normalized.starts_with("vehicle-image-cache/")
            {
                return Err(invalid(format!(
                    "Unexpected recovery package member: {normalized}"
                )));
            }
            if kind.is_dir() {
                inventory.members.insert(name.to_owned(), Member::Metadata);
                continue;
            }
            let size = entry.size();
            if size > metadata_limit(&normalized, limits) {
                return Err(invalid(format!("Recovery package member {normalized} exceeds its configured metadata or member size limit.")));
            }
            expanded = expanded
                .checked_add(size)
                .ok_or_else(|| invalid("Recovery package expanded size overflowed."))?;
            if expanded > limits.max_expanded_bytes {
                return Err(invalid(
                    "Recovery package exceeds the configured expanded byte limit.",
                ));
            }
            if expanded
                > compressed
                    .max(1)
                    .saturating_mul(limits.max_compression_ratio)
            {
                return Err(invalid(
                    "Recovery package exceeds the maximum expansion ratio.",
                ));
            }
            if normalized == "manifest.json" {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).map_err(stream_error)?;
                inventory.manifest = Some(bytes);
                inventory.members.insert(normalized, Member::Metadata);
                continue;
            }
            let mut digest = Sha256::new();
            loop {
                check_deadline(started, limits)?;
                let count = entry.read(&mut buffer).map_err(stream_error)?;
                if count == 0 {
                    break;
                }
                if normalized == "database.dump" && inventory.database_magic.len() < 5 {
                    let prefix = count.min(5 - inventory.database_magic.len());
                    inventory
                        .database_magic
                        .extend_from_slice(&buffer[..prefix]);
                }
                digest.update(&buffer[..count]);
            }
            inventory.members.insert(
                normalized,
                Member::File {
                    size,
                    checksum: hex::encode(digest.finalize()),
                },
            );
        }
        Ok(inventory)
    }

    pub(super) fn manifest(&self) -> Result<serde_json::Value, AppError> {
        let bytes = self
            .manifest
            .as_deref()
            .ok_or_else(|| invalid("Recovery package is missing manifest.json."))?;
        serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("Recovery manifest is invalid JSON: {error}")))
    }

    pub(super) fn checksum(&self, path: &str) -> Option<&str> {
        match self.members.get(path) {
            Some(Member::File { checksum, .. }) => Some(checksum.as_str()),
            _ => None,
        }
    }

    pub(super) fn size(&self, path: &str) -> Option<u64> {
        match self.members.get(path) {
            Some(Member::File { size, .. }) => Some(*size),
            _ => None,
        }
    }

    pub(super) fn custom_database_dump(&self) -> bool {
        self.database_magic == b"PGDMP"
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(message.into())
}

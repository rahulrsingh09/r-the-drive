use crate::{
    config::RecoveryConfig, errors::AppError, services::recovery_workspace::RecoveryWorkspace,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DumpInspection {
    pub has_panda_schema: bool,
    pub has_timeseries_schema: bool,
}

pub async fn inspect_recovery_dump(
    package: &Path,
    root: &Path,
    limits: &RecoveryConfig,
) -> Result<DumpInspection, AppError> {
    let workspace = RecoveryWorkspace::new(root, limits)?;
    workspace.extract(package, limits).await?;
    tokio::time::timeout(
        Duration::from_secs(limits.restore_deadline_seconds),
        async {
            let executable = resolve_pg_restore_executable().await.ok_or_else(|| {
                AppError::DependencyUnavailable(
                    "pg_restore is unavailable for restore preflight".into(),
                )
            })?;
            let mut command = Command::new(executable);
            command
                .arg("--schema-only")
                .arg("--file=-")
                .arg(workspace.path().join("database.dump"));
            inspect_command(command).await
        },
    )
    .await
    .map_err(|_| AppError::RecoveryDeadline("Recovery dump inspection deadline exceeded.".into()))?
}

async fn inspect_command(mut command: Command) -> Result<DumpInspection, AppError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            AppError::DependencyUnavailable(format!(
                "pg_restore is unavailable for restore preflight: {error}"
            ))
        })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (inspection, diagnostics) =
        tokio::try_join!(scan_schema(stdout), read_diagnostics(stderr))?;
    let status = child.wait().await?;
    if !status.success() {
        return Err(AppError::RecoveryInvalid(format!(
            "pg_restore could not inspect the recovery dump: {}",
            String::from_utf8_lossy(&diagnostics).trim()
        )));
    }
    Ok(inspection)
}

async fn consume_pipe<const BUFFER_BYTES: usize>(
    mut reader: impl AsyncRead + Unpin,
    mut consume: impl FnMut(&[u8]),
) -> Result<(), AppError> {
    let mut buffer = [0_u8; BUFFER_BYTES];
    loop {
        let length = reader.read(&mut buffer).await?;
        if length == 0 {
            return Ok(());
        }
        consume(&buffer[..length]);
    }
}

struct SchemaMarkers {
    prefixes: [usize; 2],
    found: [bool; 2],
}

impl SchemaMarkers {
    fn accept(&mut self, bytes: &[u8]) {
        for (index, marker) in [
            b"CREATE SCHEMA panda".as_slice(),
            b"CREATE SCHEMA timeseries".as_slice(),
        ]
        .iter()
        .enumerate()
        {
            if self.found[index] {
                continue;
            }
            for byte in bytes {
                let prefix = &mut self.prefixes[index];
                if *byte == marker[*prefix] {
                    *prefix += 1;
                } else {
                    *prefix = usize::from(*byte == marker[0]);
                }
                if *prefix == marker.len() {
                    self.found[index] = true;
                    break;
                }
            }
        }
    }
}

async fn scan_schema(stdout: impl AsyncRead + Unpin) -> Result<DumpInspection, AppError> {
    let mut markers = SchemaMarkers {
        prefixes: [0; 2],
        found: [false; 2],
    };
    consume_pipe::<{ 64 * 1024 }>(stdout, |bytes| markers.accept(bytes)).await?;
    Ok(DumpInspection {
        has_panda_schema: markers.found[0],
        has_timeseries_schema: markers.found[1],
    })
}

async fn read_diagnostics(stderr: impl AsyncRead + Unpin) -> Result<Vec<u8>, AppError> {
    let mut diagnostics = Vec::with_capacity(8 * 1024);
    consume_pipe::<{ 8 * 1024 }>(stderr, |bytes| {
        let keep = bytes.len().min(8 * 1024 - diagnostics.len());
        diagnostics.extend_from_slice(&bytes[..keep]);
    })
    .await?;
    Ok(diagnostics)
}

async fn resolve_pg_restore_executable() -> Option<PathBuf> {
    if probe_pg_restore(Command::new("pg_restore")).await {
        Some(PathBuf::from("pg_restore"))
    } else {
        find_windows_pg_restore()
    }
}

async fn probe_pg_restore(mut command: Command) -> bool {
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn find_windows_pg_restore() -> Option<PathBuf> {
    for root in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    {
        let postgres = PathBuf::from(root).join("PostgreSQL");
        let Ok(entries) = std::fs::read_dir(postgres) else {
            continue;
        };
        let mut versions = entries.filter_map(Result::ok).collect::<Vec<_>>();
        versions.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
        for version in versions {
            let candidate = version.path().join("bin").join("pg_restore.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn find_windows_pg_restore() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inspection_uses_configured_metadata_limits_and_cleans_up() {
        use flate2::{write::GzEncoder, Compression};
        use tar::{Builder, Header};
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        let mut archive = Builder::new(GzEncoder::new(
            std::fs::File::create(&package).unwrap(),
            Compression::default(),
        ));
        let mut header = Header::new_gnu();
        header.set_size(9);
        header.set_cksum();
        archive
            .append_data(&mut header, "manifest.json", &b"123456789"[..])
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        let backups = root.path().join("backups");
        let limits = RecoveryConfig {
            min_free_bytes: 0,
            max_manifest_bytes: 8,
            ..Default::default()
        };
        assert!(matches!(
            inspect_recovery_dump(&package, &backups, &limits).await,
            Err(AppError::RecoveryTooLarge(_))
        ));
        assert_eq!(
            std::fs::read_dir(backups.join(".recovery-work"))
                .unwrap()
                .count(),
            0
        );
        assert!(package.exists());
    }

    #[tokio::test]
    async fn schema_markers_cross_pipe_chunks_and_diagnostics_stay_bounded() {
        let (reader, mut writer) = tokio::io::duplex(1);
        let output = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer
                .write_all(b"CREATE SCHEMA panda;\nCREATE SCHEMA timeseries;\n")
                .await
                .unwrap();
        });
        let inspected = scan_schema(reader).await.unwrap();
        output.await.unwrap();
        assert!(inspected.has_panda_schema && inspected.has_timeseries_schema);
        assert_eq!(
            read_diagnostics(&vec![b'x'; 1024 * 1024][..])
                .await
                .unwrap()
                .len(),
            8 * 1024
        );
    }
}

#[cfg(test)]
mod recovery_inspection_tests {
    use super::*;

    #[test]
    fn schema_scanner_matches_existing_substring_contract_at_every_split() {
        for bytes in [
            &b""[..],
            &b"CREATE SCHEMA pand"[..],
            &b"CREATE SCHEMA timeserie"[..],
            &b"CCREATE SCHEMA panda;CREATE SCHEMA timeseries;"[..],
            &b"CREATE SCHEMA pCREATE SCHEMA panda;\0CREATE SCHEMA timeseries;"[..],
            &b"-- CREATE SCHEMA panda_extra\ncreate schema timeseries;"[..],
            &b"CREATE SCHEMA \"panda\";CREATE SCHEMA timeseriesx;"[..],
        ] {
            for split in 0..=bytes.len() {
                let mut scanner = SchemaMarkers {
                    prefixes: [0; 2],
                    found: [false; 2],
                };
                scanner.accept(&bytes[..split]);
                scanner.accept(&bytes[split..]);
                for (index, marker) in [
                    b"CREATE SCHEMA panda".as_slice(),
                    b"CREATE SCHEMA timeseries".as_slice(),
                ]
                .iter()
                .enumerate()
                {
                    assert_eq!(
                        scanner.found[index],
                        bytes.windows(marker.len()).any(|window| window == *marker),
                        "split={split}, bytes={bytes:?}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn diagnostics_preserve_prefix_and_drain_the_entire_input() {
        let mut input = vec![b'a'; 8 * 1024];
        input.extend_from_slice(&vec![b'b'; 2 * 1024 * 1024]);
        let mut reader = input.as_slice();
        let diagnostics = read_diagnostics(&mut reader).await.unwrap();
        assert_eq!(diagnostics, vec![b'a'; 8 * 1024]);
        assert!(reader.is_empty());
    }

    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inspection_drains_both_pipes_and_rejects_failed_processes() {
        let result = tokio::time::timeout(Duration::from_secs(5), inspect_command(shell(
            "i=0; while [ $i -lt 12000 ]; do printf 'diagnostic line\\n' >&2; printf 'schema output line\\n'; i=$((i+1)); done; printf 'CREATE SCHEMA panda;CREATE SCHEMA timeseries;'"
        ))).await.unwrap().unwrap();
        assert!(result.has_panda_schema && result.has_timeseries_schema);
        let error = inspect_command(shell(
            "printf 'CREATE SCHEMA panda;'; printf 'synthetic failure' >&2; exit 9",
        ))
        .await
        .unwrap_err();
        assert!(
            matches!(error, AppError::RecoveryInvalid(message) if message.ends_with("synthetic failure"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_the_inspection_child() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("unexpected-completion");
        let mut command = shell("sleep 1; printf completed > \"$PANDA_SYNTHETIC_MARKER\"");
        command.env("PANDA_SYNTHETIC_MARKER", &marker);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), inspect_command(command))
                .await
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn missing_inspection_executable_is_a_dependency_failure() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            inspect_command(Command::new(root.path().join("absent"))).await,
            Err(AppError::DependencyUnavailable(_))
        ));
    }
}

#[cfg(test)]
mod recovery_probe_tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn dependency_probe_discards_output_and_can_be_cancelled() {
        let mut flood = Command::new("/bin/sh");
        flood.args(["-c", "i=0; while [ $i -lt 12000 ]; do printf 'version output line\n'; printf 'version diagnostic line\n' >&2; i=$((i+1)); done"]);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), probe_pg_restore(flood))
                .await
                .unwrap()
        );
        let mut stalled = Command::new("/bin/sh");
        stalled.args(["-c", "exec sleep 5"]);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), probe_pg_restore(stalled))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn admitted_archive_inspection_failure_releases_workspace() {
        use crate::services::recovery_extract::recovery_validation_fixtures::{
            limits, members, write_archive,
        };
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("fixture.rma.tar.gz");
        write_archive(&package, &members(1));
        let backups = root.path().join("backups");
        let result = inspect_recovery_dump(&package, &backups, &limits()).await;
        assert!(matches!(
            result,
            Err(AppError::RecoveryInvalid(_)) | Err(AppError::DependencyUnavailable(_))
        ));
        assert_eq!(
            std::fs::read_dir(backups.join(".recovery-work"))
                .unwrap()
                .count(),
            0
        );
        assert!(package.exists());
    }
}

//! Bounded JSON parsing with one catalog record in flight. Historical JSON
//! shape is retained; fields are processed in dependency order on three passes.
use super::restore_jobs::{
    self, BackupArtifactSnapshot, BackupCatalogSnapshot, BackupRestoreRequestSnapshot,
    BackupRunSnapshot,
};
use serde::de::{DeserializeSeed, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::{
    fmt,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

struct BoundedRead {
    file: File,
    remaining: u64,
}
impl Read for BoundedRead {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let allowance = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let request = buffer.len().min(allowance.saturating_add(1));
        let count = self.file.read(&mut buffer[..request])?;
        self.remaining = self
            .remaining
            .checked_sub(count as u64)
            .ok_or_else(|| std::io::Error::other("recovery metadata exceeds its byte limit"))?;
        Ok(count)
    }
}
pub fn read_bounded_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    maximum: u64,
) -> anyhow::Result<T> {
    let file = File::open(path)?;
    match file.metadata()?.len() > maximum {
        true => anyhow::bail!("recovery metadata exceeds its byte limit"),
        false => {
            let input = BoundedRead {
                file,
                remaining: maximum,
            };
            let mut decoder = serde_json::Deserializer::from_reader(BufReader::new(input));
            let value = T::deserialize(&mut decoder)?;
            decoder.end()?;
            Ok(value)
        }
    }
}

#[derive(Clone, Copy)]
struct CatalogSeed<'a> {
    field: &'a str,
    sender: &'a tokio::sync::mpsc::Sender<ReplayRecord>,
}

struct ReplayRecord {
    snapshot: BackupCatalogSnapshot,
    committed: tokio::sync::oneshot::Sender<()>,
}
impl<'de> DeserializeSeed<'de> for CatalogSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for CatalogSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a recovery catalog object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let mut selected = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() == self.field {
                false => {
                    map.next_value::<IgnoredAny>()?;
                }
                true => {
                    if selected.replace(()).is_some() {
                        return Err(M::Error::custom("duplicate catalog field"));
                    }
                    map.next_value_seed(ArraySeed(self))?;
                }
            }
        }
        Ok(())
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        loop {
            let next = match self.field {
                "runs" => {
                    seq.next_element::<BackupRunSnapshot>()?
                        .map(|row| BackupCatalogSnapshot {
                            runs: vec![row],
                            ..Default::default()
                        })
                }
                "artifacts" => {
                    seq.next_element::<BackupArtifactSnapshot>()?
                        .map(|row| BackupCatalogSnapshot {
                            artifacts: vec![row],
                            ..Default::default()
                        })
                }
                "restore_requests" => {
                    seq.next_element::<BackupRestoreRequestSnapshot>()?
                        .map(|row| BackupCatalogSnapshot {
                            restore_requests: vec![row],
                            ..Default::default()
                        })
                }
                _ => unreachable!(),
            };
            let Some(mut snapshot) = next else {
                return Ok(());
            };
            restore_jobs::mark_source_artifact_availability(&mut snapshot, None);
            let (committed, receipt) = tokio::sync::oneshot::channel();
            self.sender
                .blocking_send(ReplayRecord {
                    snapshot,
                    committed,
                })
                .map_err(|_| S::Error::custom("catalog import cancelled"))?;
            receipt
                .blocking_recv()
                .map_err(|_| S::Error::custom("catalog import cancelled"))?;
        }
    }
}

struct ArraySeed<'a>(CatalogSeed<'a>);
impl<'de> DeserializeSeed<'de> for ArraySeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_seq(self.0)
    }
}

pub async fn merge(pool: &sqlx::PgPool, path: &Path, maximum: u64) -> anyhow::Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<ReplayRecord>(1);
    let path = path.to_owned();
    let task = tokio::task::spawn_blocking(move || {
        for field in ["runs", "artifacts", "restore_requests"] {
            let file = File::open(&path)?;
            anyhow::ensure!(
                file.metadata()?.len() <= maximum,
                "operational history exceeds its byte limit"
            );
            let mut de = serde_json::Deserializer::from_reader(BufReader::new(BoundedRead {
                file,
                remaining: maximum,
            }));
            CatalogSeed {
                field,
                sender: &sender,
            }
            .deserialize(&mut de)?;
            de.end()?;
        }
        Ok::<_, anyhow::Error>(())
    });
    while let Some(record) = receiver.recv().await {
        restore_jobs::merge_catalog_snapshot(pool, &record.snapshot).await?;
        let _ = record.committed.send(());
    }
    task.await??;
    Ok(())
}

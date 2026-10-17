use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use aws_sdk_s3::{
    config::{
        timeout::TimeoutConfig, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation,
    },
    primitives::ByteStream,
    types::{CompletedMultipartUpload, CompletedPart},
    Client,
};
use chrono::{DateTime, Utc};
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

mod catalog;
mod multipart;

const MULTIPART_THRESHOLD: u64 = 64 * 1024 * 1024;
const MULTIPART_PART_SIZE: usize = 64 * 1024 * 1024;
const MAX_CATALOG_OBJECTS: usize = 100;

#[derive(Clone, Debug)]
pub struct S3Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key: String,
    pub secret_key: String,
    pub policy: crate::services::s3_transport::S3Policy,
}

#[derive(Clone, Debug)]
pub struct RemoteObject {
    pub key: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub checksum_sha256: Option<String>,
    pub created_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

pub fn normalize_prefix(value: &str) -> Result<String> {
    Ok(crate::services::artifact_files::validate_prefix(value)?)
}

pub fn object_key(prefix: &str, created_at: DateTime<Utc>, run_id: Uuid) -> Result<String> {
    let prefix = normalize_prefix(prefix)?;
    let mut components = Vec::with_capacity(4);
    if !prefix.is_empty() {
        components.extend([
            prefix,
            created_at.format("%Y").to_string(),
            created_at.format("%m").to_string(),
        ]);
    }
    components.push(format!(
        "backup-{}-{}.rma.tar.gz",
        created_at.format("%Y%m%dT%H%M%SZ"),
        run_id.simple()
    ));
    Ok(components.join("/"))
}

pub fn locator(bucket: &str, key: &str) -> String {
    format!("s3://{}/{}", bucket.trim(), key.trim_start_matches('/'))
}

pub fn key_from_locator<'a>(bucket: &str, value: &'a str) -> Option<&'a str> {
    value.strip_prefix(&format!("s3://{}/", bucket.trim()))
}

pub fn key_belongs_to_prefix(prefix: &str, key: &str) -> bool {
    match normalize_prefix(prefix) {
        Ok(prefix) if key.ends_with(".rma.tar.gz") => {
            let belongs = prefix.is_empty()
                || key
                    .strip_prefix(&prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'));
            belongs && crate::services::artifact_files::validate_prefix(key).is_ok()
        }
        _ => false,
    }
}

fn client(settings: &S3Settings) -> Result<Client> {
    normalize_prefix(&settings.prefix)?;
    let credentials = Credentials::new(
        settings.access_key.clone(),
        settings.secret_key.clone(),
        None,
        None,
        "panda-backups",
    );
    let mut builder = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new(settings.region.clone()))
        .credentials_provider(credentials)
        .timeout_config(
            TimeoutConfig::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .read_timeout(std::time::Duration::from_secs(30))
                .operation_timeout(std::time::Duration::from_secs(300))
                .build(),
        )
        .http_client(crate::services::s3_transport::S3HttpClient(
            settings.policy.clone(),
        ))
        // Newer AWS SDK releases default to optional checksum trailers. Some
        // S3-compatible servers (including Garage) close those aws-chunked
        // requests, so only use protocol checksums when an operation requires one.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired);
    if !settings.endpoint.trim().is_empty() {
        builder = builder
            .endpoint_url(settings.endpoint.trim())
            .force_path_style(true);
    }
    Ok(Client::from_conf(builder.build()))
}

pub async fn test_connection(settings: &S3Settings) -> Result<()> {
    let client = client(settings)?;
    let key = format!(
        "{}/.panda-connection-test-{}",
        normalize_prefix(&settings.prefix)?,
        Uuid::new_v4()
    )
    .trim_start_matches('/')
    .to_string();
    let payload = format!("panda-s3-test:{}", Uuid::new_v4());
    client
        .put_object()
        .bucket(&settings.bucket)
        .key(&key)
        .body(ByteStream::from(payload.clone().into_bytes()))
        .send()
        .await
        .context("S3 write probe failed")?;
    let result = async {
        let listed = client
            .list_objects_v2()
            .bucket(&settings.bucket)
            .prefix(&key)
            .send()
            .await
            .context("S3 list probe failed")?;
        anyhow::ensure!(
            listed
                .contents()
                .iter()
                .any(|object| object.key() == Some(key.as_str())),
            "S3 list probe did not return the written object"
        );
        let response = client
            .get_object()
            .bucket(&settings.bucket)
            .key(&key)
            .send()
            .await
            .context("S3 read probe failed")?;
        let mut body = response
            .body
            .into_async_read()
            .take(payload.len() as u64 + 1);
        let mut downloaded = Vec::with_capacity(payload.len());
        body.read_to_end(&mut downloaded)
            .await
            .context("S3 read probe body failed")?;
        anyhow::ensure!(
            downloaded.len() <= payload.len(),
            "S3 read probe returned an oversized body"
        );
        anyhow::ensure!(
            downloaded == payload.as_bytes(),
            "S3 read probe returned different bytes"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let delete = client
        .delete_object()
        .bucket(&settings.bucket)
        .key(&key)
        .send()
        .await
        .context("S3 delete probe failed");
    result?;
    delete?;
    Ok(())
}

pub async fn upload(
    settings: &S3Settings,
    key: &str,
    path: &Path,
    checksum_sha256: &str,
    run_id: Uuid,
    created_at: DateTime<Utc>,
) -> Result<()> {
    let size = fs::metadata(path).await?.len();
    let metadata = HashMap::from([
        ("panda-format".to_string(), "panda-recovery-v3".to_string()),
        ("panda-sha256".to_string(), checksum_sha256.to_string()),
        ("panda-run-id".to_string(), run_id.to_string()),
        ("panda-created-at".to_string(), created_at.to_rfc3339()),
    ]);
    if size < MULTIPART_THRESHOLD {
        client(settings)?
            .put_object()
            .bucket(&settings.bucket)
            .key(key)
            .set_metadata(Some(metadata))
            .body(ByteStream::from_path(path).await?)
            .send()
            .await
            .context("S3 upload failed")?;
        return Ok(());
    }
    multipart_upload(settings, key, path, metadata).await
}

async fn multipart_upload(
    settings: &S3Settings,
    key: &str,
    path: &Path,
    metadata: HashMap<String, String>,
) -> Result<()> {
    multipart::transfer(settings, key, path, metadata).await
}

pub async fn download(
    settings: &S3Settings,
    key: &str,
    root: &Path,
    id: Uuid,
    limits: &crate::config::RecoveryConfig,
) -> Result<std::path::PathBuf> {
    anyhow::ensure!(
        key_belongs_to_prefix(&settings.prefix, key),
        "S3 object is outside the configured package prefix"
    );
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(limits.restore_deadline_seconds);
    let transfer = async {
        let response = client(settings)?
            .get_object()
            .bucket(&settings.bucket)
            .key(key)
            .send()
            .await
            .context("S3 download failed")?;
        match response.content_length() {
            None => {}
            Some(length) if length < 0 || length as u64 > limits.max_upload_bytes => {
                anyhow::bail!("S3 package exceeds the compressed byte limit");
            }
            Some(length) => {
                let required = limits.min_free_bytes.saturating_add(length as u64);
                anyhow::ensure!(
                    fs2::available_space(root)? >= required,
                    "insufficient free space for S3 package"
                );
            }
        }
        let mut body = response.body.into_async_read();
        crate::services::artifact_files::stage_reader(root, id, &mut body, limits)
            .await
            .map_err(anyhow::Error::from)
    };
    tokio::time::timeout_at(deadline, transfer)
        .await
        .context("S3 download deadline exceeded")?
}

pub async fn download_stream(settings: &S3Settings, key: &str) -> Result<ByteStream> {
    Ok(client(settings)?
        .get_object()
        .bucket(&settings.bucket)
        .key(key)
        .send()
        .await
        .context("S3 download failed")?
        .body)
}

pub async fn delete(settings: &S3Settings, key: &str) -> Result<()> {
    client(settings)?
        .delete_object()
        .bucket(&settings.bucket)
        .key(key)
        .send()
        .await
        .context("S3 delete failed")?;
    Ok(())
}

pub async fn list(settings: &S3Settings) -> Result<Vec<RemoteObject>> {
    catalog::read(settings).await
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "__tests__/backup_workflows/s3.rs"]
mod workflow_contracts;

#[cfg(test)]
#[path = "__tests__/backup_workflows/s3_fixture.rs"]
pub(crate) mod workflow_test_transport;

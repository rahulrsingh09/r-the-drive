use std::{io::Write, path::Path};

use age::x25519::Identity;
use anyhow::{anyhow, bail, Context};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "full_replacement_key_custody_backup_tests.rs"]
mod full_replacement_key_custody_tests;

use crate::{config::Config, errors::AppError};

const MAX_BYTES: usize = 20 * 1024 * 1024;
const RECEIVER: &str = "https://backup.example.invalid/backup";

pub fn configured() -> bool {
    std::env::var_os("HISTORY_BACKUP_TOKEN").is_some()
}

pub fn encrypt(plaintext: &[u8], key: &str) -> anyhow::Result<Vec<u8>> {
    if plaintext.is_empty() || plaintext.len() > MAX_BYTES - 64 * 1024 {
        bail!("recovery package exceeds the free backup size limit");
    }
    let identity: Identity = key
        .parse()
        .map_err(|_| anyhow!("invalid backup encryption key"))?;
    let recipient = identity.to_public();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .map_err(|_| anyhow!("backup encryption initialization failed"))?;
    let mut ciphertext = Vec::new();
    let mut writer = encryptor.wrap_output(&mut ciphertext)?;
    writer.write_all(plaintext)?;
    writer.finish()?;
    if ciphertext.len() > MAX_BYTES {
        bail!("encrypted recovery package exceeds the free backup size limit");
    }
    Ok(ciphertext)
}

#[derive(Deserialize)]
struct Receipt {
    key: String,
    sha256: String,
    bytes: usize,
}

async fn publish(ciphertext: Vec<u8>, token: &str, receiver: &str) -> anyhow::Result<()> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        bail!("backup receiver token is invalid");
    }
    let checksum = hex::encode(Sha256::digest(&ciphertext));
    let size = ciphertext.len();
    let client = reqwest::Client::builder()
        .user_agent("Panda-History-Backup/1.0")
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|_| anyhow!("backup upload client initialization failed"))?;
    let mut response = client
        .post(receiver)
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .header("X-Backup-Sha256", &checksum)
        .body(ciphertext)
        .send()
        .await
        .map_err(|_| anyhow!("backup upload request failed"))?;
    if response.status() != reqwest::StatusCode::CREATED {
        bail!("backup receiver did not accept the archive");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("backup receipt read failed"))?
    {
        if body.len() + chunk.len() > 1024 {
            bail!("backup receipt exceeds limit");
        }
        body.extend_from_slice(&chunk);
    }
    let receipt: Receipt = serde_json::from_slice(&body).context("invalid backup receipt")?;
    if receipt.sha256 != checksum
        || receipt.bytes != size
        || !receipt
            .key
            .strip_prefix("daily/")
            .is_some_and(|slot| slot.len() == 2 && slot.parse::<u8>().is_ok_and(|n| n < 31))
    {
        bail!("backup receipt does not match the uploaded archive");
    }
    Ok(())
}

pub async fn mirror(config: &Config, path: &Path) -> Result<(), AppError> {
    if !configured() {
        return Ok(());
    }
    let result = async {
        let token = std::env::var("HISTORY_BACKUP_TOKEN").context("backup token is unavailable")?;
        let key = config
            .age_encryption_key
            .as_ref()
            .context("backup encryption key is unavailable")?
            .clone();
        let path = path.to_owned();
        let ciphertext = tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let file = std::fs::File::open(path)?;
            let mut plaintext = Vec::new();
            file.take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut plaintext)?;
            encrypt(&plaintext, &key)
        })
        .await??;
        publish(ciphertext, &token, RECEIVER).await
    }
    .await;
    result.map_err(|_: anyhow::Error| AppError::DependencyUnavailable(
        "Encrypted Cloudflare backup failed. The recovery package remains local; check backup configuration and free storage limits.".into()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;
    use std::io::Read;

    #[test]
    fn encryption_roundtrip_requires_original_key_and_rejects_oversize() {
        let identity = Identity::generate();
        let secret = identity.to_string();
        let payload = b"synthetic lifetime trip and efficiency history";
        let encrypted = encrypt(payload, secret.expose_secret()).unwrap();
        assert!(!encrypted
            .windows(payload.len())
            .any(|window| window == payload));
        let decryptor = age::Decryptor::new(&encrypted[..]).unwrap();
        let mut decrypted = Vec::new();
        decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap()
            .read_to_end(&mut decrypted)
            .unwrap();
        assert_eq!(decrypted, payload);
        let wrong = Identity::generate();
        assert!(age::Decryptor::new(&encrypted[..])
            .unwrap()
            .decrypt(std::iter::once(&wrong as &dyn age::Identity))
            .is_err());
        assert!(encrypt(&[], secret.expose_secret()).is_err());
        assert!(encrypt(&vec![0; MAX_BYTES], secret.expose_secret()).is_err());
        assert!(encrypt(payload, "not-a-key").is_err());
    }

    #[tokio::test]
    async fn upload_verifies_receipt_and_does_not_follow_redirects() {
        use axum::{
            body::Bytes,
            http::{HeaderMap, StatusCode},
            routing::post,
            Router,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let token = "a".repeat(64);
        let app = Router::new()
            .route(
                "/ok",
                post(|headers: HeaderMap, body: Bytes| async move {
                    assert_eq!(
                        headers["authorization"],
                        format!("Bearer {}", "a".repeat(64))
                    );
                    assert_eq!(headers["content-type"], "application/octet-stream");
                    let sha = hex::encode(Sha256::digest(&body));
                    assert_eq!(headers["x-backup-sha256"], sha);
                    (
                        StatusCode::CREATED,
                        axum::Json(
                            serde_json::json!({"key":"daily/02","sha256":sha,"bytes":body.len()}),
                        ),
                    )
                }),
            )
            .route(
                "/redirect",
                post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "/ok")]) }),
            )
            .route("/bad", post(|| async { (StatusCode::CREATED, "{}") }))
            .route(
                "/large",
                post(|| async { (StatusCode::CREATED, "x".repeat(2048)) }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        assert!(publish(vec![1, 2, 3], &token, &format!("{base}/ok"))
            .await
            .is_ok());
        for path in ["redirect", "bad", "large"] {
            let error = publish(vec![1, 2, 3], &token, &format!("{base}/{path}"))
                .await
                .unwrap_err();
            assert!(!format!("{error:?}").contains(&token));
        }
        assert!(publish(vec![1], "bad-token", &format!("{base}/ok"))
            .await
            .is_err());
        server.abort();
    }
}

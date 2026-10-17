use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, FromRow)]
struct VehicleImageRow {
    placement: String,
    design: Option<String>,
    size: Option<String>,
    resolution: Option<String>,
    url: String,
    overlays: serde_json::Value,
    metadata: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VehicleImageOverlay {
    url: String,
    overlay: Option<String>,
    #[serde(rename = "zIndex")]
    z_index: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
struct DemoFixtureManifest {
    model: String,
    images: Vec<DemoFixtureImageEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct DemoFixtureImageEntry {
    placement: String,
    design: Option<String>,
    size: Option<String>,
    resolution: Option<String>,
    url: String,
    overlays: Vec<VehicleImageOverlay>,
    metadata: serde_json::Value,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    panda_api::services::outbound_policy::require_optional_traffic()?;
    let mut args = std::env::args().skip(1);
    let vehicle_id = args.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: cargo run --bin export_vehicle_image_fixture -- <vehicle_id> <model>"
        )
    })?;
    let model = args
        .next()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "usage: cargo run --bin export_vehicle_image_fixture -- <vehicle_id> <model>"
            )
        })?
        .to_uppercase();
    let vehicle_id = Uuid::parse_str(&vehicle_id)?;
    let database_url = std::env::var("DATABASE_URL")?;

    let pool = panda_api::db::pool::create_pool(&database_url).await?;
    let outcome = async {
        let rows = sqlx::query_as::<_, VehicleImageRow>(
            "SELECT placement, design, size, resolution, url, overlays, metadata
         FROM panda.vehicle_images
         WHERE vehicle_id = $1
         ORDER BY created_at, placement, design NULLS LAST",
        )
        .bind(vehicle_id)
        .fetch_all(&pool)
        .await?;

        if rows.is_empty() {
            anyhow::bail!("no cached vehicle_images rows found for {vehicle_id}");
        }

        let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/public/vehicle-images/fixtures")
            .join(model.to_lowercase());
        std::fs::create_dir_all(&output_dir)?;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let mut manifest_images = Vec::with_capacity(rows.len());
        let mut used_filenames = HashSet::new();

        let public_prefix = format!("/vehicle-images/fixtures/{}/", model.to_lowercase());
        for row in rows {
            let mut overlays = serde_json::from_value::<Vec<VehicleImageOverlay>>(row.overlays)
                .unwrap_or_default();
            let mut base_url = row.url;
            let source_url = base_url.clone();
            for url in std::iter::once(&mut base_url)
                .chain(overlays.iter_mut().map(|overlay| &mut overlay.url))
            {
                let filename =
                    download_fixture_asset(&client, &output_dir, url, &mut used_filenames).await?;
                *url = format!("{public_prefix}{filename}");
            }
            let mut metadata = row.metadata;
            if let Some(object) = metadata.as_object_mut() {
                object.extend([
                    ("source_url".into(), serde_json::Value::String(source_url)),
                    ("packaged".into(), serde_json::Value::Bool(true)),
                    ("model".into(), serde_json::Value::String(model.clone())),
                ]);
            }
            manifest_images.push(DemoFixtureImageEntry {
                placement: row.placement,
                design: row.design,
                size: row.size,
                resolution: row.resolution,
                url: base_url,
                overlays,
                metadata,
            });
        }

        let manifest = DemoFixtureManifest {
            model: model.clone(),
            images: manifest_images,
        };

        std::fs::write(
            output_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;

        println!(
            "exported {} image entries to {}",
            manifest.images.len(),
            output_dir.display()
        );
        Ok(())
    }
    .await;
    pool.close().await;
    outcome
}

async fn download_fixture_asset(
    client: &Client,
    output_dir: &Path,
    source_url: &str,
    used_filenames: &mut HashSet<String>,
) -> anyhow::Result<String> {
    panda_api::services::outbound_policy::require_optional_traffic()?;
    fetch_fixture_asset(client, output_dir, source_url, used_filenames).await
}

async fn fetch_fixture_asset(
    client: &Client,
    output_dir: &Path,
    source_url: &str,
    used_filenames: &mut HashSet<String>,
) -> anyhow::Result<String> {
    let response = client.get(source_url).send().await?.error_for_status()?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("image/webp")
        .to_owned();
    let bytes =
        panda_api::services::outbound_policy::read_limited(response, 5 * 1024 * 1024).await?;
    let filename = fixture_filename(source_url, &mime, used_filenames);
    tokio::fs::write(output_dir.join(&filename), bytes).await?;
    Ok(filename)
}

fn fixture_filename(source_url: &str, mime: &str, used: &mut HashSet<String>) -> String {
    let leaf = source_url
        .rsplit_once('/')
        .map_or(source_url, |(_, leaf)| leaf);
    let basename = leaf.split_once('?').map_or(leaf, |(name, _)| name);
    let digest = hex::encode(Sha256::digest(source_url.as_bytes()));
    let prefix = &digest[..12];
    let filename = format!("{prefix}_{basename}");
    if used.insert(filename.clone()) {
        return filename;
    }
    let extension = basename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or(if mime == "image/png" { "png" } else { "webp" });
    format!("{prefix}_dup.{extension}")
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    #[test]
    fn duplicate_asset_names_preserve_url_hash_and_legacy_suffix() {
        let mut used = HashSet::new();
        let url = "http://127.0.0.1/asset.png?size=2";
        let hash = hex::encode(Sha256::digest(url.as_bytes()));
        assert_eq!(
            fixture_filename(url, "image/webp", &mut used),
            format!("{}_asset.png", &hash[..12])
        );
        let duplicate = format!("{}_dup.png", &hash[..12]);
        assert_eq!(fixture_filename(url, "image/webp", &mut used), duplicate);
        assert_eq!(fixture_filename(url, "image/webp", &mut used), duplicate);
    }
    #[tokio::test]
    async fn downloads_use_local_http_and_preserve_failure_cleanup() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 3\r\nConnection: close\r\n\r\npng",
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut chunk = [0u8; 1024];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0, "client closed before sending headers");
                    request.extend_from_slice(&chunk[..count]);
                    assert!(request.len() <= 8192, "unexpected oversized fixture request");
                }
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let mut names = HashSet::new();
        let file = fetch_fixture_asset(
            &client,
            directory.path(),
            &format!("http://{address}/asset.png"),
            &mut names,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(directory.path().join(file)).unwrap(), b"png");
        assert!(fetch_fixture_asset(
            &client,
            directory.path(),
            &format!("http://{address}/failed.png"),
            &mut names
        )
        .await
        .is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        server.await.unwrap();
    }
}

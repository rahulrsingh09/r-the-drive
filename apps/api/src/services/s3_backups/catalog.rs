use super::*;
use aws_sdk_s3::types::Object;

struct Catalog {
    prefix: String,
    continuation: Option<String>,
    rows: Vec<RemoteObject>,
}
impl Catalog {
    fn new(prefix: String) -> Self {
        Self {
            prefix,
            continuation: None,
            rows: Vec::new(),
        }
    }
    async fn append(&mut self, client: &Client, bucket: &str, objects: &[Object]) -> Result<()> {
        for object in objects {
            let Some(key) = object.key().filter(|key| key.ends_with(".rma.tar.gz")) else {
                continue;
            };
            let head = client
                .head_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
                .context("S3 catalog metadata lookup failed")?;
            let metadata = head.metadata().cloned().unwrap_or_default();
            let instant = |value: &str| {
                DateTime::parse_from_rfc3339(value)
                    .ok()
                    .map(|value| value.with_timezone(&Utc))
            };
            let created_at = metadata
                .get("panda-created-at")
                .and_then(|v| instant(v))
                .or_else(|| object.last_modified().and_then(|v| instant(&v.to_string())))
                .unwrap_or_else(Utc::now);
            self.rows.push(RemoteObject {
                key: key.to_owned(),
                file_name: key.rsplit('/').next().unwrap_or(key).to_owned(),
                size_bytes: object.size().unwrap_or(0),
                checksum_sha256: metadata.get("panda-sha256").cloned(),
                created_at,
                metadata,
            });
            if self.rows.len() == MAX_CATALOG_OBJECTS {
                break;
            }
        }
        Ok(())
    }
}

pub(super) async fn read(settings: &S3Settings) -> Result<Vec<RemoteObject>> {
    let client = client(settings)?;
    let mut catalog = Catalog::new(normalize_prefix(&settings.prefix)?);
    loop {
        let page = client
            .list_objects_v2()
            .bucket(&settings.bucket)
            .set_prefix((!catalog.prefix.is_empty()).then(|| catalog.prefix.clone()))
            .set_continuation_token(catalog.continuation.take())
            .send()
            .await
            .context("S3 catalog listing failed")?;
        catalog
            .append(&client, &settings.bucket, page.contents())
            .await?;
        if catalog.rows.len() == MAX_CATALOG_OBJECTS || page.is_truncated() != Some(true) {
            break;
        }
        catalog.continuation = page.next_continuation_token().map(str::to_owned);
    }
    catalog
        .rows
        .sort_by_key(|row| std::cmp::Reverse(row.created_at));
    Ok(catalog.rows)
}

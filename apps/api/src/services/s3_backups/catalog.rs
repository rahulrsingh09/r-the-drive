use super::*;
use aws_sdk_s3::types::Object;
use std::{cmp::Reverse, collections::BTreeMap};

struct Catalog {
    prefix: String,
    continuation: Option<String>,
    rows: BTreeMap<Reverse<DateTime<Utc>>, Vec<RemoteObject>>,
    count: usize,
}
impl Catalog {
    fn new(prefix: String) -> Self {
        Self {
            prefix,
            continuation: None,
            rows: BTreeMap::new(),
            count: 0,
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
            self.rows
                .entry(Reverse(created_at))
                .or_default()
                .push(RemoteObject {
                    key: key.to_owned(),
                    file_name: key.rsplit('/').next().unwrap_or(key).to_owned(),
                    size_bytes: object.size().unwrap_or(0),
                    checksum_sha256: metadata.get("panda-sha256").cloned(),
                    created_at,
                    metadata,
                });
            self.count += 1;
            if self.count == MAX_CATALOG_OBJECTS {
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
        if catalog.count == MAX_CATALOG_OBJECTS || page.is_truncated() != Some(true) {
            break;
        }
        catalog.continuation = page.next_continuation_token().map(str::to_owned);
    }
    Ok(catalog.rows.into_values().flatten().collect())
}

#[cfg(test)]
mod full_replacement_catalog_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn paginated_catalog_preserves_duplicates_timestamp_fallback_and_stable_ties() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let settings = S3Settings {
            endpoint: endpoint.clone(),
            region: "fixture".into(),
            bucket: "fixture".into(),
            prefix: "archives".into(),
            access_key: "synthetic-access".into(),
            secret_key: "synthetic-secret".into(),
            policy: crate::services::s3_transport::S3Policy {
                development_origin: Some(url::Url::parse(&endpoint).unwrap()),
                ..Default::default()
            },
        };
        let server = tokio::spawn(async move {
            let page = |entries: &str, tail: &str| {
                format!(
                "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{entries}{tail}</ListBucketResult>"
            )
            };
            let entry = |key: &str| {
                format!(
                "<Contents><Key>archives/{key}.rma.tar.gz</Key><LastModified>2025-01-01T00:00:00Z</LastModified><Size>7</Size></Contents>"
            )
            };
            let responses = [
                ("GET", page(&(entry("a") + &entry("b")), "<IsTruncated>true</IsTruncated><NextContinuationToken>page-two</NextContinuationToken>"), String::new()),
                ("HEAD", String::new(), "x-amz-meta-panda-sha256: first\r\n".into()),
                ("HEAD", String::new(), "x-amz-meta-panda-created-at: 2026-01-01T00:00:00Z\r\nx-amz-meta-panda-sha256: second\r\n".into()),
                ("GET", page(&(entry("a") + &entry("c")), "<IsTruncated>false</IsTruncated>"), String::new()),
                ("HEAD", String::new(), "x-amz-meta-panda-created-at: 2026-01-01T00:00:00Z\r\nx-amz-meta-panda-sha256: third\r\n".into()),
                ("HEAD", String::new(), "x-amz-meta-panda-created-at: 2024-01-01T00:00:00Z\r\nx-amz-meta-panda-sha256: fourth\r\n".into()),
            ];
            for (index, (method, body, headers)) in responses.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    assert!(request.len() < 16 * 1024);
                    request.push(socket.read_u8().await.unwrap());
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(method));
                if index == 3 {
                    assert!(request.contains("continuation-token=page-two"));
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/xml\r\n{headers}Connection: close\r\n\r\n{body}", body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        let rows = tokio::time::timeout(std::time::Duration::from_secs(10), read(&settings))
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (
                    row.file_name.as_str(),
                    row.checksum_sha256.as_deref().unwrap()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("b.rma.tar.gz", "second"),
                ("a.rma.tar.gz", "third"),
                ("a.rma.tar.gz", "first"),
                ("c.rma.tar.gz", "fourth"),
            ]
        );
        assert_eq!(rows[2].created_at.to_rfc3339(), "2025-01-01T00:00:00+00:00");
        assert!(rows.iter().all(|row| row.size_bytes == 7));
    }

    #[tokio::test]
    async fn catalog_cap_counts_duplicate_rows_without_requesting_another_page() {
        use crate::services::s3_backups::workflow_test_transport::{head, reply, Fake};
        let entry = "<Contents><Key>fixture/repeated.rma.tar.gz</Key><Size>7</Size></Contents>";
        let page = format!(
            "<ListBucketResult>{}<IsTruncated>true</IsTruncated><NextContinuationToken>unused</NextContinuationToken></ListBucketResult>",
            entry.repeat(MAX_CATALOG_OBJECTS + 1)
        );
        let mut responses = vec![reply("GET", 200, page.into_bytes())];
        responses.extend(
            (0..MAX_CATALOG_OBJECTS).map(|_| head("2026-01-01T00:00:00Z", "same-checksum")),
        );
        let fixture = Fake::new("catalog-duplicate-cap", responses).await;
        let rows = read(&fixture.settings).await.unwrap();
        assert_eq!(rows.len(), MAX_CATALOG_OBJECTS);
        assert!(rows
            .iter()
            .all(|row| row.key == "fixture/repeated.rma.tar.gz"));
        assert_eq!(
            fixture.finish().as_array().unwrap().len(),
            MAX_CATALOG_OBJECTS + 1
        );
    }
}

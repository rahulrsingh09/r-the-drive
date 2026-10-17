use super::*;
use tokio::io::AsyncReadExt;

#[test]
fn normalizes_keys_and_locators() {
    let now = DateTime::parse_from_rfc3339("2026-07-22T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let id = Uuid::nil();
    let key = object_key("panda/prod/", now, id).unwrap();
    assert_eq!(
        key,
        "panda/prod/2026/07/backup-20260722T120000Z-00000000000000000000000000000000.rma.tar.gz"
    );
    let value = locator("backups", &key);
    assert_eq!(key_from_locator("backups", &value), Some(key.as_str()));
    assert_eq!(key_from_locator("other", &value), None);
    assert!(key_belongs_to_prefix("panda/prod", &key));
    assert!(!key_belongs_to_prefix("panda/other", &key));
}

#[tokio::test]
#[ignore = "requires a disposable Garage instance"]
async fn garage_upload_list_download_delete_round_trip() {
    let settings = S3Settings {
        endpoint: std::env::var("PANDA_TEST_S3_ENDPOINT").expect("test endpoint"),
        region: std::env::var("PANDA_TEST_S3_REGION").unwrap_or_else(|_| "garage".into()),
        bucket: std::env::var("PANDA_TEST_S3_BUCKET").expect("test bucket"),
        prefix: format!("integration/{}", Uuid::new_v4()),
        access_key: std::env::var("PANDA_TEST_S3_ACCESS_KEY").expect("test access key"),
        secret_key: std::env::var("PANDA_TEST_S3_SECRET_KEY").expect("test secret key"),
        policy: crate::services::s3_transport::S3Policy {
            development_origin: Some(
                url::Url::parse(&std::env::var("PANDA_TEST_S3_ENDPOINT").expect("test endpoint"))
                    .unwrap(),
            ),
            ..Default::default()
        },
    };
    test_connection(&settings).await.expect("connection probe");
    let directory = std::env::temp_dir().join(format!("panda-s3-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory)
        .await
        .expect("test directory");
    let source = directory.join("source.rma.tar.gz");
    fs::write(&source, b"garage integration sentinel")
        .await
        .expect("source file");
    let run_id = Uuid::new_v4();
    let key = object_key(&settings.prefix, Utc::now(), run_id).unwrap();
    upload(&settings, &key, &source, "test-sha256", run_id, Utc::now())
        .await
        .expect("upload");
    let rows = list(&settings).await.expect("list");
    assert!(rows
        .iter()
        .any(|row| row.key == key && row.checksum_sha256.as_deref() == Some("test-sha256")));
    let mut bytes = Vec::new();
    download_stream(&settings, &key)
        .await
        .expect("download")
        .into_async_read()
        .read_to_end(&mut bytes)
        .await
        .expect("download body");
    assert_eq!(bytes, b"garage integration sentinel");
    delete(&settings, &key).await.expect("delete");
    assert!(!list(&settings)
        .await
        .expect("list after delete")
        .iter()
        .any(|row| row.key == key));
    let _ = fs::remove_dir_all(directory).await;
}


#[test]
fn full_replacement_locator_and_prefix_contract() {
    assert_eq!(
        super::locator(" bucket ", "///one/two"),
        "s3://bucket/one/two"
    );
    assert_eq!(
        super::key_from_locator(" bucket ", "s3://bucket/a"),
        Some("a")
    );
    assert_eq!(
        super::key_from_locator("bucket", "s3://bucket-other/a"),
        None
    );
    for (prefix, key, allowed) in [
        ("a", "a/x.rma.tar.gz", true),
        ("a", "ab/x.rma.tar.gz", false),
        ("a", "a.rma.tar.gz", false),
        ("", "x.rma.tar.gz", true),
        ("", "a/../x.rma.tar.gz", false),
        ("", "a/.x.rma.tar.gz", false),
    ] {
        assert_eq!(
            super::key_belongs_to_prefix(prefix, key),
            allowed,
            "{prefix}/{key}"
        );
    }
}

#[tokio::test]
async fn full_replacement_probe_preserves_mismatch_error_and_deletes_oversized_response() {
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request, Response},
        routing::any,
        Router,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let payload = Arc::new(Mutex::new(Vec::new()));
    let deleted = Arc::new(AtomicBool::new(false));
    let cleanup = deleted.clone();
    let router = Router::new().fallback(any(move |request: Request<Body>| {
        let payload = payload.clone();
        let deleted = deleted.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = match parts.method {
                Method::PUT => {
                    let bytes = to_bytes(body, 1024).await.unwrap().to_vec();
                    *payload.lock().unwrap() = bytes;
                    Vec::new()
                }
                Method::GET if parts.uri.query().is_some_and(|query| query.contains("list-type=")) => {
                    let key = url::form_urlencoded::parse(parts.uri.query().unwrap().as_bytes())
                        .find(|(name, _)| name == "prefix").unwrap().1.into_owned();
                    format!("<ListBucketResult><Contents><Key>{key}</Key></Contents></ListBucketResult>").into_bytes()
                }
                Method::GET => {
                    let mut bytes = payload.lock().unwrap().clone();
                    bytes.extend_from_slice(b"excess response bytes");
                    bytes
                }
                Method::DELETE => {
                    deleted.store(true, Ordering::SeqCst);
                    Vec::new()
                }
                _ => panic!("unexpected probe request"),
            };
            Response::new(Body::from(bytes))
        }
    }));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let settings = S3Settings {
        endpoint: endpoint.clone(),
        region: "fixture".into(),
        bucket: "fixture".into(),
        prefix: "archives".into(),
        access_key: "synthetic-access".into(),
        secret_key: "synthetic-secret".into(),
        policy: crate::services::s3_transport::S3Policy {
            development_origin: Some(endpoint.parse().unwrap()),
            ..Default::default()
        },
    };
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        test_connection(&settings),
    )
    .await
    .unwrap()
    .unwrap_err();
    task.abort();
    assert!(
        cleanup.load(Ordering::SeqCst),
        "probe object must be deleted after mismatch"
    );
    assert_eq!(error.to_string(), "S3 read probe returned different bytes");
}

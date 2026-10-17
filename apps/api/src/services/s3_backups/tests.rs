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

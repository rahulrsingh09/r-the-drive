use super::*;
use serde_json::json;
#[allow(dead_code)]
#[path = "support.rs"]
mod support;

#[path = "s3_fixture.rs"]
mod fixture;
use fixture::*;

#[tokio::test]
async fn backup_workflow_s3_listing_and_transfer_contract() {
    let mut records = Vec::new();
    let fake=Fake::new("catalog",vec![
        reply("GET",200,b"<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>page-two</NextContinuationToken><Contents><Key>fixture/skip.txt</Key><Size>99</Size></Contents><Contents><Key>fixture/old.rma.tar.gz</Key><Size>8</Size><LastModified>2026-01-01T00:00:00Z</LastModified></Contents></ListBucketResult>".to_vec()),
        head("invalid","old-sha"),
        reply("GET",200,b"<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>fixture/new.rma.tar.gz</Key><Size>11</Size><LastModified>2026-01-02T00:00:00Z</LastModified></Contents><Contents><Key>fixture/null.rma.tar.gz</Key><LastModified>2026-01-02T00:00:00Z</LastModified></Contents></ListBucketResult>".to_vec()),
        head("2026-01-03T04:05:06+05:30","new-sha"),head("2026-01-03T04:05:06+05:30",""),
    ]).await;
    let rows=list(&fake.settings).await.unwrap().into_iter().map(|r|json!({"key":r.key,"file_name":r.file_name,"size_bytes":r.size_bytes,"checksum_sha256":r.checksum_sha256,"created_at":r.created_at,"metadata":r.metadata})).collect::<Vec<_>>();
    records.push(json!({"case":"paginated stable selection","rows":rows,"calls":fake.finish()}));
    for (name,replies) in [("list failure",vec![error("GET")]),("head failure",vec![reply("GET",200,b"<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>fixture/a.rma.tar.gz</Key></Contents></ListBucketResult>".to_vec()),error("HEAD")])]{
        let fake=Fake::new(name,replies).await;let err=list(&fake.settings).await.unwrap_err();records.push(json!({"case":name,"error":format!("{err:#}"),"calls":fake.finish()}));
    }
    let root =
        tempfile::tempdir_in(std::env::var("BACKUP_CONTRACT_ARTIFACT_DIR").unwrap()).unwrap();
    let path = root.path().join("upload.rma.tar.gz");
    fs::write(&path, b"synthetic package bytes").await.unwrap();
    let fake = Fake::new("single-upload", vec![reply("PUT", 200, vec![])]).await;
    upload(
        &fake.settings,
        "fixture/upload.rma.tar.gz",
        &path,
        "fixed-checksum",
        Uuid::nil(),
        "2026-01-01T00:00:00Z".parse().unwrap(),
    )
    .await
    .unwrap();
    records.push(json!({"case":"single upload metadata and bytes","calls":fake.finish()}));
    for (name, body, limit, success) in [
        ("download", b"fixed download".to_vec(), 14, true),
        ("oversize download", vec![1u8; 15], 14, false),
        ("empty download", vec![], 14, true),
    ] {
        let fake = Fake::new(name, vec![reply("GET", 200, body.clone())]).await;
        let limits = crate::config::RecoveryConfig {
            min_free_bytes: 0,
            max_upload_bytes: limit,
            ..Default::default()
        };
        let output = download(
            &fake.settings,
            "fixture/object.rma.tar.gz",
            root.path(),
            Uuid::nil(),
            &limits,
        )
        .await;
        assert_eq!(output.is_ok(), success);
        let result = match output {
            Ok(path) => {
                let bytes = fs::read(&path).await.unwrap();
                assert_eq!(bytes, body);
                fs::remove_file(path).await.unwrap();
                json!({"bytes":hex::encode(bytes)})
            }
            Err(e) => json!({"error":format!("{e:#}")}),
        };
        records.push(json!({"case":name,"result":result,"calls":fake.finish()}));
    }
    let fake = Fake::new("prefix-denied", vec![]).await;
    let denied = download(
        &fake.settings,
        "foreign/object.rma.tar.gz",
        root.path(),
        Uuid::nil(),
        &Default::default(),
    )
    .await
    .unwrap_err();
    records.push(json!({"case":"prefix denied before transport","error":denied.to_string(),"calls":fake.finish()}));
    let source = root.path().join("multipart.rma.tar.gz");
    let file = File::create(&source).await.unwrap();
    file.set_len(MULTIPART_PART_SIZE as u64 + 3).await.unwrap();
    drop(file);
    for (name, fail) in [
        ("multipart", false),
        ("part failure abort", true),
        ("complete failure abort", false),
    ] {
        let mut replies=vec![reply("POST",200,b"<InitiateMultipartUploadResult><UploadId>fixed-upload</UploadId></InitiateMultipartUploadResult>".to_vec())];
        replies.push(if fail {
            error("PUT")
        } else {
            Reply {
                method: "PUT",
                status: 200,
                headers: vec![("etag", "part-1".into())],
                body: vec![],
            }
        });
        if !fail {
            replies.push(Reply {
                method: "PUT",
                status: 200,
                headers: vec![("etag", "part-2".into())],
                body: vec![],
            });
            replies.push(if name=="complete failure abort"{error("POST")}else{reply("POST",200,b"<CompleteMultipartUploadResult><ETag>complete</ETag></CompleteMultipartUploadResult>".to_vec())});
        }
        if fail || name == "complete failure abort" {
            replies.push(reply("DELETE", 204, vec![]));
        }
        let fake = Fake::new(name, replies).await;
        let result = upload(
            &fake.settings,
            "fixture/multipart.rma.tar.gz",
            &source,
            "fixed-checksum",
            Uuid::nil(),
            "2026-01-01T00:00:00Z".parse().unwrap(),
        )
        .await
        .map_err(|e| format!("{e:#}"));
        assert_eq!(result.is_ok(), name == "multipart");
        records.push(json!({"case":name,"result":result,"calls":fake.finish()}));
    }
    support::record("s3", &json!(records), include_str!("s3.json"));
}

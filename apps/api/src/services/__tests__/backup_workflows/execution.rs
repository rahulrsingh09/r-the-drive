use super::*;
use crate::services::s3_backups::workflow_test_transport as fixture;
use serde_json::Value;

fn redact_clock(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, v) in fields {
                if ["created_at", "updated_at", "started_at", "completed_at"]
                    .contains(&key.as_str())
                    && v.is_string()
                {
                    *v = json!("<clock>")
                } else {
                    redact_clock(v)
                }
            }
        }
        Value::Array(values) => {
            for v in values {
                redact_clock(v)
            }
        }
        _ => {}
    }
}

#[tokio::test]
#[ignore = "requires fresh restricted DB fixtures and local scripted S3"]
async fn backup_workflow_execution_success_failure_and_fallback() {
    assert!(!crate::private_deployment::history_backup::configured());
    let mut evidence = Vec::new();
    for scenario in [
        "local",
        "safety",
        "remote",
        "upload-failure",
        "retention-failure",
    ] {
        let db = support::DatabaseFixture::new().await;
        let root =
            tempfile::tempdir_in(std::env::var("BACKUP_CONTRACT_ARTIFACT_DIR").unwrap()).unwrap();
        let mut cfg = config(&db, root.path());
        let remote = matches!(scenario, "remote" | "upload-failure" | "retention-failure");
        let fake = if remote {
            let mut replies = vec![if scenario == "upload-failure" {
                fixture::error("PUT")
            } else {
                fixture::reply("PUT", 200, vec![])
            }];
            if scenario != "upload-failure" {
                replies.push(if scenario == "retention-failure" {
                    fixture::error("GET")
                } else {
                    fixture::reply(
                        "GET",
                        200,
                        b"<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>"
                            .to_vec(),
                    )
                });
            }
            Some(fixture::Fake::new(scenario, replies).await)
        } else {
            None
        };
        if let Some(fake) = &fake {
            cfg.panda_env = Some("development".into());
            cfg.security.s3_allow_development_garage = true;
            cfg.s3_endpoint = Some(fake.settings.endpoint.clone());
            cfg.s3_access_key = Some(fake.settings.access_key.clone());
            cfg.s3_secret_key = Some(fake.settings.secret_key.clone());
        }
        sqlx::query("INSERT INTO panda.backup_settings(id,enabled,frequency,run_at,timezone,retention_count,local_enabled,s3_enabled,bucket,prefix) VALUES(TRUE,FALSE,'daily','03:00','UTC',8,$1,$2,'bucket','fixture')")
            .bind(!remote && scenario!="safety").bind(remote).execute(&db.pool).await.unwrap();
        let trigger = if scenario == "safety" {
            BackupRunTrigger::PreRestore
        } else {
            BackupRunTrigger::Manual
        };
        let result = run_backup_now(&db.pool, &cfg, None, trigger).await;
        assert_eq!(
            result.is_ok(),
            !scenario.ends_with("failure"),
            "{scenario}: {result:?}"
        );
        let raw = state(&db.pool).await;
        let runs = raw["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        let run_id: Uuid = run["id"].as_str().unwrap().parse().unwrap();
        let error = result.as_ref().err().map(ToString::to_string);
        let paths: Vec<String> = raw["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["storage_type"].as_str().unwrap().to_owned())
            .collect();
        let expected_types = match scenario {
            "local" => vec!["local"],
            "safety" => vec!["safety"],
            "remote" => vec!["s3"],
            "upload-failure" => vec!["local"],
            _ => vec!["s3", "local"],
        };
        assert_eq!(paths.len(), expected_types.len());
        for storage in &expected_types {
            assert!(paths.iter().any(|p| p == storage));
        }
        let artifacts = raw["artifacts"].as_array().unwrap();
        for artifact in artifacts {
            assert_eq!(artifact["run_id"], run["id"]);
            let storage = artifact["storage_type"].as_str().unwrap();
            let location = artifact["storage_path"].as_str().unwrap();
            let (bytes, package) = if storage != "s3" {
                let bytes = fs::read(location).await.unwrap();
                let package =
                    validate_recovery_package_with_limits(Path::new(location), &cfg.recovery)
                        .await
                        .unwrap();
                (bytes, package)
            } else {
                let dir = PathBuf::from(std::env::var("BACKUP_CONTRACT_ARTIFACT_DIR").unwrap());
                let phase =
                    std::env::var("BACKUP_CONTRACT_RUN").unwrap_or_else(|_| "actual".into());
                let source = dir.join(format!("{phase}-s3-{scenario}-0.body"));
                let bytes = fs::read(&source).await.unwrap();
                let package = validate_recovery_package_with_limits(&source, &cfg.recovery)
                    .await
                    .unwrap();
                (bytes, package)
            };
            assert_eq!(artifact["size_bytes"], bytes.len() as i64);
            assert_eq!(
                artifact["checksum_sha256"],
                hex::encode(Sha256::digest(&bytes))
            );
            let mut expected_manifest = json!({"artifact_kind":"recovery_package","format":"panda-recovery-v3","package":package.manifest,
                "created_at":artifact["manifest"]["created_at"],"database":db.database,"timescale_version":"2.28.3",
                "trigger":trigger.as_str(),"prefix":"fixture","retention_count":8,"storage_type":storage});
            if scenario.ends_with("failure") && storage == "local" {
                expected_manifest["emergency_fallback"] = json!(true);
            }
            assert_eq!(
                artifact["manifest"], expected_manifest,
                "complete catalog manifest"
            );
            let mut complete = artifact.clone();
            redact_clock(&mut complete);
            let mut normalized_manifest = expected_manifest;
            redact_clock(&mut normalized_manifest);
            assert_eq!(
                complete,
                json!({"id":artifact["id"],"run_id":run_id,"storage_type":storage,"file_name":artifact["file_name"],"storage_path":location,
                "size_bytes":bytes.len() as i64,"checksum_sha256":hex::encode(Sha256::digest(&bytes)),"manifest":normalized_manifest,"created_at":"<clock>"})
            );
        }
        let published = if scenario.ends_with("failure") {
            Value::Null
        } else {
            artifacts[0]["storage_path"].clone()
        };
        let mut complete = run.clone();
        redact_clock(&mut complete);
        assert_eq!(
            complete,
            json!({"id":run_id,"trigger":trigger.as_str(),"status":if error.is_some(){"failed"}else{"succeeded"},"phase":if error.is_some(){"failed"}else{"completed"},
            "progress_percent":match scenario{"upload-failure"=>92,"retention-failure"=>98,_=>100},"artifact_key":published,"requested_by":null,
            "started_at":"<clock>","completed_at":"<clock>","error_message":error,"created_at":"<clock>","updated_at":"<clock>"}),
            "complete run row"
        );
        let lock = BackupLock::acquire(&db.pool)
            .await
            .expect("terminal operation released admission");
        lock.release().await;
        evidence.push(json!({"scenario":scenario,"state":raw,"calls":fake.map(|f|f.finish())}));
        db.cleanup().await;
    }
    let dir = PathBuf::from(std::env::var("BACKUP_CONTRACT_ARTIFACT_DIR").unwrap());
    let phase = std::env::var("BACKUP_CONTRACT_RUN").unwrap_or_else(|_| "actual".into());
    std::fs::write(
        dir.join(format!("{phase}-execution-original.json")),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
}

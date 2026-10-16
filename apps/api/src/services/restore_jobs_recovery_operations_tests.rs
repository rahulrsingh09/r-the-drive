use super::*;
use crate::services::restore_compatibility::*;

pub(crate) fn config(root: &Path) -> Config {
    Config {
        database_url: "postgresql://unused:synthetic@127.0.0.1:1/unused".into(),
        redis_url: "redis://127.0.0.1:1/".into(),
        jwt_secret: None,
        jwt_public_key: None,
        age_encryption_key: None,
        port: 0,
        allowed_origins: vec![],
        s3_endpoint: None,
        s3_access_key: None,
        s3_secret_key: None,
        backup_artifact_dir: root.to_string_lossy().into_owned(),
        vehicle_image_cache_dir: root.join("images").to_string_lossy().into_owned(),
        backup_driver: "pg_dump".into(),
        backup_poll_interval_seconds: 60,
        restore_agent_url: "http://127.0.0.1:1".into(),
        restore_agent_key_file: root.join("unused-key").to_string_lossy().into_owned(),
        recovery: crate::config::RecoveryConfig {
            min_free_bytes: 0,
            ..Default::default()
        },
        origin_bind: Default::default(),
        security: Default::default(),
        rivian_ws_reconnect_initial_seconds: 10,
        rivian_ws_reconnect_max_seconds: 900,
        rivian_raw_event_retention_days: 7,
        rivian_persist_raw_events: false,
        rivian_suppress_duplicate_telemetry: true,
        panda_env: Some("development".into()),
        cookie_insecure: None,
        allow_insecure_lan_http_auth: false,
        rate_limit: Default::default(),
    }
}

fn plan() -> RestorePlan {
    let profile = DatabaseProfile {
        postgres_major: Some(18),
        timescale_version: Some("2.28.3".into()),
        migration_version: latest_migration_version(),
        migration_ledger: compiled_migration_ledger(),
        migration_ledger_successful: true,
        migration_chain_id: Some(crate::db::migrations::MIGRATION_CHAIN_ID.into()),
        migration_catalog_digest: Some(crate::db::migrations::migration_catalog_digest()),
        schema_contract_version: Some(SCHEMA_CONTRACT_VERSION.into()),
        schema_fingerprint: Some("synthetic".into()),
    };
    RestorePlan {
        plan_id: "synthetic-plan".into(),
        engine_version: RESTORE_ENGINE_VERSION,
        package_checksum_sha256: "synthetic-checksum".into(),
        package_format: RECOVERY_FORMAT_V3.into(),
        compatible: true,
        source: profile.clone(),
        target: profile,
        pending_migrations: vec![],
        transforms: vec![],
        validation_checks: vec![],
        warnings: vec![],
        blocking_errors: vec![],
        planned_at: Utc::now(),
    }
}

#[tokio::test]
async fn journal_characterizes_creation_transitions_tokens_and_public_redaction() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let (job, token) = create(
        &config,
        Uuid::new_v4(),
        "/synthetic/private-package".into(),
        Uuid::new_v4(),
        plan(),
    )
    .await
    .unwrap();
    assert_eq!(job.phase, RestorePhase::Queued);
    assert_eq!(job.progress_percent, 5);
    assert!(job.retryable);
    assert!(token_matches(&job, &token));
    assert!(!token_matches(&job, &(token.clone() + "x")));
    let public = serde_json::to_value(job.public()).unwrap();
    for field in [
        "artifact_path",
        "capability_sha256",
        "catalog_snapshot",
        "candidate_database",
        "previous_database",
    ] {
        assert!(public.get(field).is_none(), "{field}");
    }
    let job = update(&config, job.id, RestorePhase::Planning, 150, "planned")
        .await
        .unwrap();
    assert_eq!(job.progress_percent, 100);
    fail(&config, job.id, "synthetic failure").await.unwrap();
    let mut job = read(&config, job.id).await.unwrap();
    assert_eq!(job.phase, RestorePhase::Failed);
    assert_eq!(job.progress_percent, 99);
    assert_eq!(job.error_message.as_deref(), Some("synthetic failure"));
    job.phase = RestorePhase::Queued;
    job.automatic_retry_count += 1;
    job.rollback_state = RollbackState::Succeeded;
    write(&config, &job).await.unwrap();
    let retried = read(&config, job.id).await.unwrap();
    assert_eq!(retried.automatic_retry_count, 1);
    assert_eq!(retried.rollback_state, RollbackState::Succeeded);
    assert!(matches!(
        read(&config, Uuid::new_v4()).await,
        Err(AppError::NotFound)
    ));
    assert_eq!(
        std::fs::read_dir(jobs_dir(&config))
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "json"))
            .count(),
        1
    );
}

#[test]
fn journal_characterizes_source_availability_and_legacy_progress() {
    let now = Utc::now();
    let mut run = BackupRunSnapshot {
        id: Uuid::new_v4(),
        trigger: "manual".into(),
        status: "succeeded".into(),
        phase: "".into(),
        progress_percent: -1,
        artifact_key: None,
        started_at: None,
        completed_at: None,
        error_message: None,
        created_at: now,
        updated_at: now,
    };
    assert_eq!(normalized_backup_run_progress(&run), ("completed", 100));
    run.status = "failed".into();
    run.progress_percent = 100;
    assert_eq!(normalized_backup_run_progress(&run), ("failed", 99));
    run.phase = "uploading".into();
    assert_eq!(normalized_backup_run_progress(&run), ("uploading", 100));
    let mut source = BackupCatalogSnapshot {
        artifacts: vec![BackupArtifactSnapshot {
            id: Uuid::new_v4(),
            run_id: None,
            storage_type: "s3".into(),
            file_name: "synthetic".into(),
            storage_path: "s3://synthetic/private".into(),
            size_bytes: 1,
            checksum_sha256: "synthetic".into(),
            manifest: Value::Null,
            created_at: now,
        }],
        ..Default::default()
    };
    let target = source.clone();
    mark_source_artifact_availability(&mut source, Some(&target));
    assert_eq!(
        source.artifacts[0].manifest["restore_availability"],
        "unavailable"
    );
    assert_eq!(
        source.artifacts[0].storage_path,
        target.artifacts[0].storage_path
    );
}

#[cfg(unix)]
#[tokio::test]
async fn journal_regression_temporary_symlink_cannot_overwrite_another_file() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let (job, _) = create(
        &config,
        Uuid::new_v4(),
        "/synthetic/package".into(),
        Uuid::new_v4(),
        plan(),
    )
    .await
    .unwrap();
    let outside = root.path().join("untouched");
    std::fs::write(&outside, b"preserve").unwrap();
    std::os::unix::fs::symlink(&outside, jobs_dir(&config).join(format!(".{}.tmp", job.id)))
        .unwrap();
    write(&config, &job).await.unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"preserve");
    assert!(!std::fs::symlink_metadata(job_path(&config, job.id))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[tokio::test]
async fn journal_regression_failed_publication_removes_temporary() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let (job, _) = create(
        &config,
        Uuid::new_v4(),
        "/synthetic/package".into(),
        Uuid::new_v4(),
        plan(),
    )
    .await
    .unwrap();
    std::fs::remove_file(job_path(&config, job.id)).unwrap();
    std::fs::create_dir(job_path(&config, job.id)).unwrap();
    assert!(write(&config, &job).await.is_err());
    assert!(
        !std::fs::read_dir(jobs_dir(&config))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "tmp")),
        "journal publication left temporary bytes behind"
    );
}

fn catalog() -> BackupCatalogSnapshot {
    let now = Utc::now();
    let run_id = Uuid::new_v4();
    BackupCatalogSnapshot {
        runs: vec![BackupRunSnapshot {
            id: run_id,
            trigger: "manual".into(),
            status: "succeeded".into(),
            phase: "completed".into(),
            progress_percent: 100,
            artifact_key: None,
            started_at: Some(now),
            completed_at: Some(now),
            error_message: None,
            created_at: now,
            updated_at: now,
        }],
        artifacts: vec![BackupArtifactSnapshot {
            id: Uuid::new_v4(),
            run_id: Some(run_id),
            storage_type: "s3".into(),
            file_name: "synthetic.rma.tar.gz".into(),
            storage_path: "s3://synthetic/package".into(),
            size_bytes: 42,
            checksum_sha256: "synthetic".into(),
            manifest: serde_json::json!({"restore_availability": "available"}),
            created_at: now,
        }],
        restore_requests: vec![],
    }
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn catalog_regression_merge_is_atomic_on_late_fk_failure() {
    let db =
        crate::services::restore_compatibility::recovery_operations_tests::SyntheticDatabase::new()
            .await;
    let mut snapshot = catalog();
    snapshot
        .restore_requests
        .push(BackupRestoreRequestSnapshot {
            id: Uuid::new_v4(),
            artifact_id: Uuid::new_v4(),
            status: "failed".into(),
            confirmation_phrase: "RESTORE".into(),
            notes: None,
            error_message: None,
            requested_at: Utc::now(),
            updated_at: Utc::now(),
        });
    assert!(merge_catalog_snapshot(&db.pool, &snapshot).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.backup_runs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "failed merge published a partial catalog");
    db.close().await;
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn catalog_characterizes_target_locator_ownership_and_terminal_reconciliation() {
    let db =
        crate::services::restore_compatibility::recovery_operations_tests::SyntheticDatabase::new()
            .await;
    let source = catalog();
    merge_catalog_snapshot(&db.pool, &source).await.unwrap();
    let target = catalog();
    merge_catalog_snapshot(&db.pool, &target).await.unwrap();
    let (path, availability): (String, String) = sqlx::query_as("SELECT storage_path,manifest->>'restore_availability' FROM panda.backup_artifacts WHERE id=$1")
        .bind(source.artifacts[0].id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(path, format!("unavailable:{}", source.artifacts[0].id));
    assert_eq!(availability, "unavailable");
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    for phase in [RestorePhase::Completed, RestorePhase::Failed] {
        let (mut job, _) = create(
            &config,
            target.artifacts[0].id,
            "/synthetic/unused".into(),
            Uuid::new_v4(),
            plan(),
        )
        .await
        .unwrap();
        job.catalog_snapshot = Some(target.clone());
        job.phase = phase.clone();
        write(&config, &job).await.unwrap();
        reconcile_completed_jobs(&db.pool, &config).await.unwrap();
        let reconciled = read(&config, job.id).await.unwrap();
        assert!(reconciled.reconciled_at.is_some());
        let status: String =
            sqlx::query_scalar("SELECT status FROM panda.backup_restore_requests WHERE id=$1")
                .bind(job.restore_request_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(
            status,
            if phase == RestorePhase::Completed {
                "completed"
            } else {
                "failed"
            }
        );
        reconcile_completed_jobs(&db.pool, &config).await.unwrap();
        assert_eq!(
            read(&config, job.id).await.unwrap().reconciled_at,
            reconciled.reconciled_at
        );
    }
    let snapshot = snapshot_catalog(&db.pool).await.unwrap();
    assert_eq!(snapshot.runs.len(), 2);
    assert_eq!(snapshot.artifacts.len(), 2);
    assert_eq!(snapshot.restore_requests.len(), 2);
    db.close().await;
}

#[tokio::test]
async fn journal_budget_failure_preserves_the_last_complete_record() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config(root.path());
    config.recovery.max_history_bytes = 0;
    config.recovery.max_manifest_bytes = 0;
    let (mut job, _) = create(
        &config,
        Uuid::new_v4(),
        "/synthetic/package".into(),
        Uuid::new_v4(),
        plan(),
    )
    .await
    .unwrap();
    let original = std::fs::read(job_path(&config, job.id)).unwrap();
    job.message = "x".repeat(128 * 1024);
    assert!(matches!(
        write(&config, &job).await,
        Err(AppError::RecoveryTooLarge(_))
    ));
    assert_eq!(std::fs::read(job_path(&config, job.id)).unwrap(), original);
    assert!(!std::fs::read_dir(jobs_dir(&config))
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "tmp")));
    config.recovery.restore_deadline_seconds = 0;
    assert!(matches!(
        write(&config, &job).await,
        Err(AppError::RecoveryDeadline(_))
    ));
}

#[tokio::test]
async fn journal_serializes_concurrent_mutations() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let (job, _) = create(
        &config,
        Uuid::new_v4(),
        "/synthetic/package".into(),
        Uuid::new_v4(),
        plan(),
    )
    .await
    .unwrap();
    let mut tasks = Vec::new();
    for _ in 0..24 {
        let config = config.clone();
        tasks.push(tokio::spawn(async move {
            recovery_journal::mutate(&config, job.id, |job| job.automatic_retry_count += 1)
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        read(&config, job.id).await.unwrap().automatic_retry_count,
        24
    );
}

#[cfg(unix)]
#[tokio::test]
async fn journal_rejects_symlinked_roots_and_mismatched_record_identity() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let config = config(root.path());
    std::os::unix::fs::symlink(outside.path(), jobs_dir(&config)).unwrap();
    assert!(create(
        &config,
        Uuid::new_v4(),
        "/synthetic/package".into(),
        Uuid::new_v4(),
        plan()
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    std::fs::remove_file(jobs_dir(&config)).unwrap();
    let (job, _) = create(
        &config,
        Uuid::new_v4(),
        "/synthetic/package".into(),
        Uuid::new_v4(),
        plan(),
    )
    .await
    .unwrap();
    let different = Uuid::new_v4();
    std::fs::copy(job_path(&config, job.id), job_path(&config, different)).unwrap();
    assert!(matches!(
        read(&config, different).await,
        Err(AppError::Validation(_))
    ));
}

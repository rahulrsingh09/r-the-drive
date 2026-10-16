use super::*;

#[test]
fn catalog_recognizes_service_run_names_without_guessing_other_paths() {
    let id = Uuid::new_v4();
    assert_eq!(
        backup_run_id_from_artifact_path(Path::new(&format!(
            "backup-20261016T000000Z-{}.rma.tar.gz",
            id.simple()
        ))),
        Some(id)
    );
    for path in [
        "package.json",
        "unknown.rma.tar.gz",
        "backup-not-a-run.rma.tar.gz",
    ] {
        assert_eq!(backup_run_id_from_artifact_path(Path::new(path)), None);
    }
}

#[tokio::test]
async fn catalog_scan_observes_count_deadline_and_sorted_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("backups");
    std::fs::create_dir(&root).unwrap();
    for name in ["b.rma.tar.gz", "a.rma.tar.gz"] {
        std::fs::write(root.join(name), b"synthetic").unwrap();
    }
    let mut config = crate::services::restore_jobs::recovery_operations_tests::config(&root);
    let paths = scan_local_packages(&root, &config).await.unwrap();
    assert_eq!(
        paths,
        vec![root.join("a.rma.tar.gz"), root.join("b.rma.tar.gz")]
    );
    config.recovery.max_members = 1;
    assert!(matches!(
        scan_local_packages(&root, &config).await,
        Err(AppError::RecoveryTooLarge(_))
    ));
    config.recovery.restore_deadline_seconds = 0;
    assert!(matches!(
        scan_local_packages(&root, &config).await,
        Err(AppError::RecoveryDeadline(_))
    ));
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn catalog_characterizes_discovery_active_runs_and_reverification() {
    use crate::services::recovery_extract::recovery_validation_fixtures::{members, write_archive};
    let db =
        crate::services::restore_compatibility::recovery_operations_tests::SyntheticDatabase::new()
            .await;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("backups");
    std::fs::create_dir(&root).unwrap();
    let config = crate::services::restore_jobs::recovery_operations_tests::config(&root);
    let local = &root.join("package.rma.tar.gz");
    write_archive(local, &members(1));
    let hidden = &root.join(".hidden");
    std::fs::create_dir(hidden).unwrap();
    write_archive(&hidden.join("hidden.rma.tar.gz"), &members(1));
    let run: Uuid = sqlx::query_scalar(
        "INSERT INTO panda.backup_runs (trigger,status) VALUES ('manual','running') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let pending = &root.join(format!(
        "backup-20261016T000000Z-{}.rma.tar.gz",
        run.simple()
    ));
    write_archive(pending, &members(1));
    assert_eq!(reconcile_local_catalog(&db.pool, &config).await.unwrap(), 1);
    assert_eq!(reconcile_local_catalog(&db.pool, &config).await.unwrap(), 0);
    sqlx::query("UPDATE panda.backup_artifacts SET manifest=jsonb_build_object('restore_availability','unavailable'), checksum_sha256='stale'").execute(&db.pool).await.unwrap();
    assert_eq!(reconcile_local_catalog(&db.pool, &config).await.unwrap(), 0);
    let (availability, checksum): (String, String) = sqlx::query_as(
        "SELECT manifest->>'restore_availability',checksum_sha256 FROM panda.backup_artifacts",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(availability, "available");
    assert_ne!(checksum, "stale");
    sqlx::query("UPDATE panda.backup_runs SET status='succeeded' WHERE id=$1")
        .bind(run)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(reconcile_local_catalog(&db.pool, &config).await.unwrap(), 1);
    db.close().await;
}

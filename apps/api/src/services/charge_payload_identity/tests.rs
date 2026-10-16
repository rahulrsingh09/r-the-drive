use super::*;

#[tokio::test]
#[ignore = "requires an isolated PostgreSQL/TimescaleDB DATABASE_URL"]
async fn backfill_index_preserves_migrated_schema_contract_and_detects_real_drift() {
    let (pool, admin, database) =
        crate::services::charge_sessions::charge_history_contract::database().await;
    let fingerprints = async {
        let before = crate::services::restore_compatibility::schema_fingerprint(&pool).await?;
        ensure_pending_index(&pool).await?;
        let after = crate::services::restore_compatibility::schema_fingerprint(&pool).await?;
        sqlx::query("CREATE INDEX unexpected_contract_drift ON panda.users(created_at)")
            .execute(&pool)
            .await?;
        let drift = crate::services::restore_compatibility::schema_fingerprint(&pool).await?;
        Ok::<_, anyhow::Error>((before, after, drift))
    }
    .await;
    crate::services::charge_sessions::charge_history_contract::cleanup(pool, admin, database).await;
    let (before, after, drift) = fingerprints.unwrap();
    assert_eq!(
        before, after,
        "backfill startup must preserve the migrated contract"
    );
    assert_ne!(
        before, drift,
        "unexpected schema changes must remain visible"
    );
}

#[test]
fn default_backfill_config_is_bounded() {
    let config = BackfillConfig::default();
    assert_eq!(config.batch_size, 1_000);
    assert_eq!(config.pause, Duration::from_millis(100));
}

#[test]
fn invalid_batch_size_is_rejected() {
    std::env::set_var("CHARGE_IDENTITY_BACKFILL_BATCH_SIZE", "99");
    let error = BackfillConfig::from_env().expect_err("invalid batch size must fail");
    std::env::remove_var("CHARGE_IDENTITY_BACKFILL_BATCH_SIZE");
    assert!(error.to_string().contains("between 100 and 10000"));
}

#[test]
fn retry_backoff_doubles_and_is_bounded() {
    assert_eq!(
        next_retry_delay(Duration::from_secs(1)),
        Duration::from_secs(2)
    );
    assert_eq!(
        next_retry_delay(Duration::from_secs(32)),
        Duration::from_secs(60)
    );
    assert_eq!(
        next_retry_delay(Duration::from_secs(60)),
        Duration::from_secs(60)
    );
}

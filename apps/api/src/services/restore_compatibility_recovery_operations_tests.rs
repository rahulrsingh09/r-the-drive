use super::*;

pub(crate) struct SyntheticDatabase {
    pub(crate) pool: PgPool,
    admin: PgPool,
    name: String,
    pub(crate) url: String,
}

impl SyntheticDatabase {
    pub(crate) async fn new() -> Self {
        Self::create(true).await
    }

    async fn create(migrate: bool) -> Self {
        let base = std::env::var("RECOVERY_OPERATIONS_TEST_DATABASE_URL")
            .expect("explicit disposable recovery operations database required");
        let mut url = Url::parse(&base).unwrap();
        assert_eq!(url.path(), "/recovery_ops");
        assert_eq!(url.username(), "recovery_ops");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&base)
            .await
            .unwrap();
        let name = format!("recovery_operations_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .execute(&admin)
            .await
            .unwrap();
        url.set_path(&format!("/{name}"));
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(url.as_str())
            .await
            .unwrap();
        if migrate {
            MIGRATOR.run(&pool).await.unwrap();
        } else {
            migrations::apply_baseline_schema(&pool).await.unwrap();
        }
        Self {
            pool,
            admin,
            name,
            url: url.into(),
        }
    }

    pub(crate) async fn close(self) {
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE \"{}\" WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await
        .unwrap();
        self.admin.close().await;
    }
}

fn manifest(profile: &DatabaseProfile) -> Value {
    serde_json::json!({"format": RECOVERY_FORMAT_V3, "source": profile})
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn compatibility_characterizes_plan_matrix_and_digest() {
    let db = SyntheticDatabase::new().await;
    let profile = runtime_database_profile(&db.pool).await.unwrap();
    for case in 0..8 {
        let mut source = manifest(&profile);
        match case {
            0 => {}
            1 => source["format"] = RECOVERY_FORMAT_V1.into(),
            2 => source["format"] = RECOVERY_FORMAT_V2.into(),
            3 => source["format"] = "unknown".into(),
            4 => source["source"]["schema_contract_version"] = "unknown".into(),
            5 => source["source"]["postgres_major"] = 99.into(),
            6 => source["source"]["timescale_version"] = "99.0.0".into(),
            _ => source["source"]["migration_ledger"] = serde_json::json!([]),
        }
        let plan = plan_restore(&source, "synthetic-checksum", &db.pool, None)
            .await
            .unwrap();
        let second = plan_restore(&source, "synthetic-checksum", &db.pool, None)
            .await
            .unwrap();
        assert_eq!(plan.plan_id, second.plan_id);
        assert_eq!(plan.compatible, matches!(case, 0 | 7));
        println!(
            "PLAN_CASE {case} {}",
            serde_json::json!({
                "id": plan.plan_id, "compatible": plan.compatible, "pending": plan.pending_migrations,
                "checks": plan.validation_checks, "errors": plan.blocking_errors
            })
        );
    }
    db.close().await;
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn compatibility_characterizes_candidate_schema_and_sanitization() {
    let db = SyntheticDatabase::new().await;
    let profile = runtime_database_profile(&db.pool).await.unwrap();
    let mut source = manifest(&profile);
    source["format"] = RECOVERY_FORMAT_V2.into();
    assert!(prepare_candidate_schema(&db.pool, &db.url, &source)
        .await
        .is_err());
    source["format"] = RECOVERY_FORMAT_V3.into();
    source["source"]["schema_fingerprint"] = "wrong".into();
    assert!(prepare_candidate_schema(&db.pool, &db.url, &source)
        .await
        .is_err());
    source["source"]["schema_fingerprint"] = profile.schema_fingerprint.unwrap().into();
    sqlx::query("UPDATE panda.authentication_settings SET oidc_enabled=TRUE, password_login_enabled=FALSE, client_secret_encrypted='synthetic'::bytea WHERE id=TRUE").execute(&db.pool).await.unwrap();
    let report = prepare_candidate_schema(&db.pool, &db.url, &source)
        .await
        .unwrap();
    assert!(report.source_schema.required_relations_present);
    assert!(report.target_schema.foreign_keys_validated);
    assert!(report.migrations_applied.is_empty());
    let safe: bool = sqlx::query_scalar("SELECT NOT oidc_enabled AND password_login_enabled AND client_secret_encrypted IS NULL FROM panda.authentication_settings WHERE id=TRUE").fetch_one(&db.pool).await.unwrap();
    assert!(safe);
    db.close().await;
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn candidate_adopts_legacy_ledger_only_with_physical_baseline_proof() {
    for baseline in [true, false] {
        let db = SyntheticDatabase::create(!baseline).await;
        let source = serde_json::json!({
            "format": RECOVERY_FORMAT_V3,
            "source": {
                "migration_version": 3,
                "migration_ledger": [{"version": 3, "description": "synthetic legacy", "checksum_sha384": "00".repeat(48)}],
                "schema_contract_version": SCHEMA_CONTRACT_VERSION,
                "schema_fingerprint": schema_fingerprint(&db.pool).await.unwrap(),
            }
        });
        let result = prepare_candidate_schema(&db.pool, &db.url, &source).await;
        if baseline {
            let report = result.unwrap();
            assert_eq!(
                report.migrations_applied,
                compiled_migration_ledger()
                    .iter()
                    .skip(1)
                    .map(|migration| migration.version)
                    .collect::<Vec<_>>()
            );
            assert!(report.target_schema.required_relations_present);
        } else {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("not the public baseline"));
        }
        db.close().await;
    }
}

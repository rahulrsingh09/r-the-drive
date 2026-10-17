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

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn cancellation_of_baseline_probe_does_not_leak_its_scratch_database() {
    let db = SyntheticDatabase::new().await;
    let url = db.url.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (finish, wait) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(async move {
        baseline_schema_fingerprint_after_create(&url, |name| async move {
            started.send(name).unwrap();
            let _ = wait.await;
        })
        .await
    });
    let name = ready.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let _ = finish.send(());
    let leaked = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname=$1)")
                    .bind(&name)
                    .fetch_one(&db.pool)
                    .await
                    .unwrap();
            if !exists {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .is_err();
    sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
    )))
    .execute(&db.pool)
    .await
    .unwrap();
    db.close().await;
    assert!(
        !leaked,
        "cancelled baseline verification leaked its scratch database"
    );
}

struct PreparedScratch {
    name: String,
    url: String,
    transaction: String,
    connection: sqlx::pool::PoolConnection<Postgres>,
    pool: PgPool,
}

impl PreparedScratch {
    async fn new(db: &SyntheticDatabase) -> Self {
        let name = format!("panda_restore_baseline_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE \"{name}\" TEMPLATE template0"
        )))
        .execute(&db.admin)
        .await
        .unwrap();
        let mut url = Url::parse(&db.url).unwrap();
        url.set_path(&format!("/{name}"));
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(url.as_str())
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::raw_sql("CREATE TABLE marker (id integer); BEGIN; INSERT INTO marker VALUES (1)")
            .execute(&mut *connection)
            .await
            .unwrap();
        let transaction = format!("baseline_cleanup_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!(
            "PREPARE TRANSACTION '{transaction}'"
        )))
        .execute(&mut *connection)
        .await
        .expect("disposable fixture requires max_prepared_transactions > 0");
        Self {
            name,
            url: url.into(),
            transaction,
            connection,
            pool,
        }
    }

    async fn release(self, admin: &PgPool) {
        drop(self.connection);
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "ALTER DATABASE \"{}\" ALLOW_CONNECTIONS true",
            self.name
        )))
        .execute(admin)
        .await
        .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.url)
            .await
            .unwrap();
        sqlx::query(AssertSqlSafe(format!(
            "ROLLBACK PREPARED '{}'",
            self.transaction
        )))
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }
}

#[tokio::test]
#[ignore = "requires disposable recovery operations database with prepared transactions enabled"]
async fn baseline_cleanup_closes_only_scratch_admissions_and_retries_busy_drop() {
    let db = SyntheticDatabase::new().await;
    let scratch = PreparedScratch::new(&db).await;
    let name = scratch.name.clone();
    let pool = db.admin.clone();
    let cleanup_name = name.clone();
    let cleanup =
        tokio::spawn(async move { drop_baseline_scratch_database(&pool, &cleanup_name).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let allowed: bool =
                sqlx::query_scalar("SELECT datallowconn FROM pg_database WHERE datname=$1")
                    .bind(&name)
                    .fetch_one(&db.admin)
                    .await
                    .unwrap();
            if !allowed {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let error = PgPoolOptions::new()
        .max_connections(1)
        .connect(&scratch.url)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55000")
    );
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert!(!cleanup.is_finished(), "busy database must be retried");
    let error = cleanup.await.unwrap().unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55006")
    );
    scratch.release(&db.admin).await;
    drop_baseline_scratch_database(&db.admin, &name)
        .await
        .unwrap();
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname=$1)")
            .bind(&name)
            .fetch_one(&db.admin)
            .await
            .unwrap();
    assert!(!exists);
    let allowed: bool =
        sqlx::query_scalar("SELECT datallowconn FROM pg_database WHERE datname=current_database()")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(
        allowed,
        "candidate database admissions must remain unchanged"
    );
    assert!(schema_fingerprint(&db.pool).await.is_ok());
    db.close().await;
}

#[tokio::test]
#[ignore = "requires disposable recovery operations database with prepared transactions enabled"]
async fn baseline_cleanup_preserves_busy_error_after_bounded_retries() {
    let db = SyntheticDatabase::new().await;
    let scratch = PreparedScratch::new(&db).await;
    let name = scratch.name.clone();
    let started = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        drop_baseline_scratch_database(&db.admin, &name),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55006")
    );
    assert!(started.elapsed() >= std::time::Duration::from_millis(900));
    scratch.release(&db.admin).await;
    drop_baseline_scratch_database(&db.admin, &name)
        .await
        .unwrap();
    db.close().await;
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn baseline_cleanup_preserves_permission_errors_and_accepts_absence() {
    let db = SyntheticDatabase::new().await;
    let name = format!("panda_restore_baseline_{}", Uuid::new_v4().simple());
    drop_baseline_scratch_database(&db.admin, &name)
        .await
        .unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "CREATE DATABASE \"{name}\" TEMPLATE template0"
    )))
    .execute(&db.admin)
    .await
    .unwrap();
    let role = format!("baseline_cleanup_{}", Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!("CREATE ROLE \"{role}\"")))
        .execute(&db.admin)
        .await
        .unwrap();
    let restricted = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db.url)
        .await
        .unwrap();
    sqlx::query(AssertSqlSafe(format!("SET ROLE \"{role}\"")))
        .execute(&restricted)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        drop_baseline_scratch_database(&restricted, &name),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    let allowed: bool = sqlx::query_scalar("SELECT datallowconn FROM pg_database WHERE datname=$1")
        .bind(&name)
        .fetch_one(&db.admin)
        .await
        .unwrap();
    assert!(allowed);
    restricted.close().await;
    sqlx::query(AssertSqlSafe(format!("DROP ROLE \"{role}\"")))
        .execute(&db.admin)
        .await
        .unwrap();
    drop_baseline_scratch_database(&db.admin, &name)
        .await
        .unwrap();
    db.close().await;
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn baseline_probe_disables_telemetry_only_in_its_scratch_database() {
    let db = SyntheticDatabase::create(false).await;
    let before: String =
        sqlx::query_scalar("SELECT current_setting('timescaledb.telemetry_level')")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let expected = schema_fingerprint(&db.pool).await.unwrap();
    let admin = db.admin.clone();
    let fingerprint = baseline_schema_fingerprint_after_create(&db.url, move |name| async move {
        let configured: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_db_role_setting s \
             JOIN pg_database d ON d.oid=s.setdatabase \
             WHERE d.datname=$1 AND s.setrole=0 \
             AND 'timescaledb.telemetry_level=off'=ANY(s.setconfig))",
        )
        .bind(&name)
        .fetch_one(&admin)
        .await
        .unwrap();
        assert!(
            configured,
            "telemetry must be off before the baseline connection"
        );
    })
    .await
    .unwrap();
    assert_eq!(fingerprint, expected);
    let after: String = sqlx::query_scalar("SELECT current_setting('timescaledb.telemetry_level')")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    db.close().await;
}


#[test]
fn full_replacement_version_suffix_and_missing_component_contract() {
    for (version, expected) in [
        ("2.28", Some((2, 28, 0))),
        ("2.28.3-dev.1", Some((2, 28, 3))),
        ("2.28.03suffix", Some((2, 28, 3))),
        ("2.28.", None),
        ("2.28.preview", None),
        ("2", None),
        (" 2.28.3", None),
        ("2.28.3.ignored", Some((2, 28, 3))),
    ] {
        assert_eq!(super::parse_version_triplet(version), expected, "{version}");
    }
    let empty = super::source_profile_from_manifest(&serde_json::json!({})).unwrap();
    assert_eq!(empty.postgres_major, None);
    assert_eq!(empty.migration_version, 0);
    assert!(empty.migration_ledger.is_empty());
    assert!(empty.migration_ledger_successful);
    assert!(super::source_profile_from_manifest(
        &serde_json::json!({"source":{"migration_ledger":null}})
    )
    .is_err());
}

#[tokio::test]
#[ignore = "requires the uniquely named disposable recovery operations database"]
async fn full_replacement_cancellation_does_not_wait_for_the_proof_callback() {
    let db = SyntheticDatabase::new().await;
    let url = db.url.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (finish, wait) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(async move {
        baseline_schema_fingerprint_after_create(&url, |name| async move {
            started.send(name).unwrap();
            let _ = wait.await;
        })
        .await
    });
    let name = ready.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let removed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname=$1)")
                    .bind(&name)
                    .fetch_one(&db.pool)
                    .await
                    .unwrap();
            if !exists {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok();
    let _ = finish.send(());
    sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
    )))
    .execute(&db.pool)
    .await
    .unwrap();
    db.close().await;
    assert!(
        removed,
        "cancelled proof waited for its blocked proof callback"
    );
}

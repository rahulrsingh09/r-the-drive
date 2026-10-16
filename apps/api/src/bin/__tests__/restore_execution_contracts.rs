use super::*;
use axum_test::TestServer;
use serde_json::Value;
use sqlx::{Executor, PgPool};

#[path = "restore_execution_durable.rs"]
mod durable;

fn config(directory: &Path) -> Config {
    let mut config: Config = serde_json::from_value(json!({
        "database_url": "postgres://restore_fixture:synthetic_restore_only@db:5432/restore_fixture",
        "redis_url": "redis://unused",
        "panda_env": "production",
        "backup_artifact_dir": directory.join("packages"),
        "vehicle_image_cache_dir": directory.join("artwork"),
        "backup_driver": "pg_dump"
    }))
    .unwrap();
    config.recovery.min_free_bytes = 0;
    config
}

fn fixed_job() -> restore_jobs::RestoreJob {
    serde_json::from_value(json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "artifact_id": "00000000-0000-0000-0000-000000000002",
        "artifact_path": "/synthetic/package",
        "safety_artifact_path": null,
        "restore_request_id": "00000000-0000-0000-0000-000000000003",
        "phase": "preparing_candidate", "progress_percent": 25,
        "message": "Synthetic candidate", "error_message": null,
        "capability_sha256": "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b",
        "created_at": "2020-01-01T00:00:00Z", "updated_at": "2020-01-01T00:00:00Z",
        "retryable": true
    }))
    .unwrap()
}

fn evidence(name: &str, value: &Value) {
    let root = std::env::var("RESTORE_CONTRACT_ARTIFACT_DIR").unwrap();
    std::fs::write(
        Path::new(&root).join(format!("{name}-actual.json")),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn authentication_and_full_public_wire_contract() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let job = fixed_job();
    restore_jobs::write(&config, &job).await.unwrap();
    let state = AgentState {
        config,
        agent_key: "synthetic-agent-key".into(),
    };
    let server = TestServer::new(
        Router::new()
            .route("/v1/restore-runtime/jobs/{job_id}", get(get_job))
            .route("/internal/jobs/{job_id}/execute", post(execute_job))
            .with_state(state.clone()),
    );
    let mut results = Vec::new();
    for (path, token) in [
        (job.id.to_string(), None),
        (job.id.to_string(), Some("")),
        (job.id.to_string(), Some("wrong")),
        ("00000000-0000-0000-0000-000000000099".into(), Some("wrong")),
        (job.id.to_string(), Some("secret")),
        ("malformed".into(), Some("secret")),
    ] {
        let mut request = server.get(&format!("/v1/restore-runtime/jobs/{path}"));
        if let Some(token) = token {
            request = request.add_header("x-panda-restore-token", token);
        }
        let response = request.await;
        results.push(json!({"status":response.status_code().as_u16(),"body":response.text()}));
    }
    let oversized = "s".repeat(MAX_AUTH_HEADER_BYTES + 1);
    let response = server
        .get(&format!("/v1/restore-runtime/jobs/{}", job.id))
        .add_header("x-panda-restore-token", &oversized)
        .await;
    results.push(json!({"status":response.status_code().as_u16(),"body":response.text()}));
    let mut terminal = job.clone();
    terminal.phase = RestorePhase::Completed;
    restore_jobs::write(&state.config, &terminal).await.unwrap();
    for (id, key) in [
        (job.id, ""),
        (job.id, "wrong"),
        (Uuid::from_u128(99), "synthetic-agent-key"),
        (job.id, "synthetic-agent-key"),
    ] {
        let response = server
            .post(&format!("/internal/jobs/{id}/execute"))
            .add_header("x-panda-agent-key", key)
            .await;
        results.push(json!({"status":response.status_code().as_u16(),"body":response.text()}));
    }
    let actual = json!(results);
    evidence("authentication", &actual);
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(include_str!("restore_execution/authentication.json"))
            .unwrap()
    );
}

#[tokio::test]
async fn artwork_activation_rollback_and_finalize_preserve_complete_files() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path());
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested")).await.unwrap();
    fs::write(source.join("nested/car.bin"), [0, 255, 7])
        .await
        .unwrap();
    fs::write(source.join("empty"), []).await.unwrap();
    fs::create_dir_all(&config.vehicle_image_cache_dir)
        .await
        .unwrap();
    fs::write(
        Path::new(&config.vehicle_image_cache_dir).join("old"),
        b"before",
    )
    .await
    .unwrap();
    let id = Uuid::from_u128(20);
    let activation = activate_artwork(&config, &source, id).await.unwrap();
    assert_eq!(
        fs::read(activation.destination.join("nested/car.bin"))
            .await
            .unwrap(),
        [0, 255, 7]
    );
    assert_eq!(
        fs::read(activation.previous.join("old")).await.unwrap(),
        b"before"
    );
    assert!(!activation.destination.join("old").exists());
    rollback_artwork(&activation).await.unwrap();
    assert_eq!(
        fs::read(activation.destination.join("old")).await.unwrap(),
        b"before"
    );
    assert!(!activation.destination.join("nested").exists());
    let activation = activate_artwork(&config, &source, id).await.unwrap();
    finalize_artwork(&activation).await;
    assert!(!activation.previous.exists());
    assert_eq!(
        fs::read(activation.destination.join("nested/car.bin"))
            .await
            .unwrap(),
        [0, 255, 7]
    );
    let missing = activate_artwork(&config, &temp.path().join("absent"), Uuid::from_u128(21)).await;
    assert!(missing.is_err());
    assert_eq!(
        fs::read(activation.destination.join("nested/car.bin"))
            .await
            .unwrap(),
        [0, 255, 7]
    );
}

struct DatabaseFixture {
    admin: PgPool,
    pool: PgPool,
    config: Config,
    directory: tempfile::TempDir,
    role: String,
}

impl DatabaseFixture {
    async fn new() -> Self {
        let explicit = std::env::var("RESTORE_CONTRACT_DATABASE_URL").unwrap();
        let mut url = url::Url::parse(&explicit).unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "restore_fixture");
        assert_eq!(url.path(), "/restore_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_database WHERE datname='panda')")
                .fetch_one(&admin)
                .await
                .unwrap();
        assert!(
            !exists,
            "fixture requires a fresh disposable server or prior fixture cleanup"
        );
        let role = format!("restore_app_{}", Uuid::new_v4().simple());
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_app_only'"
        ))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE panda OWNER {role} TEMPLATE template0"
            )))
            .await
            .unwrap();
        url.set_path("/panda");
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE SCHEMA panda AUTHORIZATION {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SCHEMA panda FROM PUBLIC;
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        let directory =
            tempfile::tempdir_in(std::env::var("RESTORE_CONTRACT_ARTIFACT_DIR").unwrap()).unwrap();
        let mut config = config(directory.path());
        config.database_url = url.to_string();
        let keys = panda_api::keys::generate_keys().unwrap();
        config.jwt_secret = Some(keys.jwt_private_pem);
        config.jwt_public_key = Some(keys.jwt_public_pem);
        config.age_encryption_key = Some(keys.age_key);
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_app_only")).unwrap();
        let pool = panda_api::db::pool::application_pool_options()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
        panda_api::db::migrations::run_current_migrations(&pool)
            .await
            .unwrap();
        panda_api::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        let protected: bool = sqlx::query_scalar("SELECT NOT has_table_privilege(current_user,'panda.vehicles','DELETE') AND NOT has_table_privilege(current_user,'panda.trips','TRUNCATE')")
            .fetch_one(&pool).await.unwrap();
        assert!(protected);
        let last: i64 = sqlx::query_scalar("SELECT max(version) FROM public._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(last, 1_000_031);
        Self {
            admin,
            pool,
            config,
            directory,
            role,
        }
    }

    async fn cleanup(self) {
        self.pool.close().await;
        self.admin
            .execute("DROP DATABASE panda WITH (FORCE)")
            .await
            .unwrap();
        cleanup_restore_roles(&self.config).await.unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
        self.directory.close().unwrap();
    }
}

#[tokio::test]
#[ignore = "requires the explicitly isolated restore fixture server"]
async fn database_cutover_rollback_role_cleanup_and_lease_contract() {
    let fixture = DatabaseFixture::new().await;
    let id = Uuid::from_u128(30);
    let candidate = format!("panda_restore_{}", id.simple());
    let previous = format!("panda_previous_{}", id.simple());
    create_database(&fixture.config, &candidate).await.unwrap();
    let candidate_config = config_with_database(&fixture.config, &candidate).unwrap();
    run_psql(&candidate_config, "CREATE TABLE public.synthetic_origin(value text); INSERT INTO public.synthetic_origin VALUES ('candidate');").await.unwrap();
    fixture.pool.execute("CREATE TABLE panda.synthetic_origin(value text); INSERT INTO panda.synthetic_origin VALUES ('original')").await.unwrap();
    let lock = backups::acquire_recovery_mutation_lock(&fixture.pool)
        .await
        .unwrap();
    assert!(backups::acquire_recovery_mutation_lock(&fixture.pool)
        .await
        .is_err());
    lock.release().await;
    let lock = backups::acquire_recovery_mutation_lock(&fixture.pool)
        .await
        .unwrap();
    lock.release().await;
    let lock = backups::acquire_recovery_mutation_lock(&fixture.pool)
        .await
        .unwrap();
    drop(lock);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(lock) = backups::acquire_recovery_mutation_lock(&fixture.pool).await {
                lock.release().await;
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a cancelled owner must release the recovery lease");
    fixture.pool.close().await;
    swap_databases(&fixture.config, &candidate, &previous)
        .await
        .unwrap();
    let active = PgPool::connect(&fixture.config.database_url).await.unwrap();
    let actual: String = sqlx::query_scalar("SELECT value FROM public.synthetic_origin")
        .fetch_one(&active)
        .await
        .unwrap();
    assert_eq!(actual, "candidate");
    active.close().await;
    assert!(!database_exists(&fixture.config, &candidate).await.unwrap());
    assert!(database_exists(&fixture.config, &previous).await.unwrap());
    rollback_databases(&fixture.config, &previous, &candidate)
        .await
        .unwrap();
    let active = PgPool::connect(&fixture.config.database_url).await.unwrap();
    let actual: String = sqlx::query_scalar("SELECT value FROM panda.synthetic_origin")
        .fetch_one(&active)
        .await
        .unwrap();
    assert_eq!(actual, "original");
    active.close().await;
    drop_database(&fixture.config, &candidate).await.unwrap();
    assert!(!database_exists(&fixture.config, &candidate).await.unwrap());
    assert!(!database_exists(&fixture.config, &previous).await.unwrap());
    fixture.cleanup().await;
}

fn catalog() -> Value {
    json!({
        "restore_requests": [{
            "id": Uuid::from_u128(103), "artifact_id": Uuid::from_u128(102),
            "status": "failed", "confirmation_phrase": "RESTORE", "notes": "",
            "error_message": null, "requested_at": "2020-01-01T00:00:00.123456Z",
            "updated_at": "2020-01-01T00:01:00Z"
        }],
        "artifacts": [{
            "id": Uuid::from_u128(102), "run_id": Uuid::from_u128(101),
            "storage_type": "s3", "file_name": "synthetic.tar.gz", "storage_path": "synthetic/key",
            "size_bytes": 0, "checksum_sha256": "fixed-synthetic-checksum",
            "manifest": {"restore_availability":"available","nested":[null,0,""]},
            "created_at": "2020-01-01T00:00:00Z"
        }],
        "runs": [{
            "id": Uuid::from_u128(101), "trigger": "manual", "status": "failed",
            "progress_percent": 120, "artifact_key": null, "started_at": null,
            "completed_at": null, "error_message": "fixture failure",
            "created_at": "2020-01-01T00:00:00Z", "updated_at": "2020-01-01T00:00:00Z"
        }],
        "unknown": [null, {"runs": []}]
    })
}

async fn catalog_rows(pool: &PgPool) -> Value {
    let mut result = serde_json::Map::new();
    for table in ["backup_runs", "backup_artifacts", "backup_restore_requests"] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY id"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        result.insert(table.into(), json!(rows));
    }
    json!(result)
}

#[tokio::test]
#[ignore = "requires the explicitly isolated restore fixture server"]
async fn history_replay_complete_rows_and_ordered_failures() {
    let fixture = DatabaseFixture::new().await;
    let path = fixture.directory.path().join("history.json");
    let input = serde_json::to_vec(&catalog()).unwrap();
    fs::write(&path, &input).await.unwrap();
    restore_history::merge(&fixture.pool, &path, input.len() as u64)
        .await
        .unwrap();
    let first = catalog_rows(&fixture.pool).await;
    restore_history::merge(&fixture.pool, &path, input.len() as u64)
        .await
        .unwrap();
    assert_eq!(
        first,
        catalog_rows(&fixture.pool).await,
        "replay idempotency"
    );
    let mut results = vec![json!({"case":"complete","rows":first})];
    for (name, bytes, limit) in [
        ("empty", b"{}".to_vec(), 2),
        ("null", b"null".to_vec(), 4),
        ("array", b"[]".to_vec(), 2),
        ("oversized", input.clone(), input.len() as u64 - 1),
        ("null-runs", br#"{"runs":null}"#.to_vec(), 100),
        ("malformed", br#"{"runs":["#.to_vec(), 100),
        ("duplicate", br#"{"runs":[],"runs":[]}"#.to_vec(), 100),
        ("trailing", br#"{"runs":[]} false"#.to_vec(), 100),
        ("unknown", br#"{"new":[1,null,{}]}"#.to_vec(), 100),
    ] {
        fs::write(&path, &bytes).await.unwrap();
        let error = restore_history::merge(&fixture.pool, &path, limit)
            .await
            .err()
            .map(|e| e.to_string());
        results.push(json!({"case":name,"error":error,"rows":catalog_rows(&fixture.pool).await}));
    }
    let mut late = catalog();
    late["runs"][0]["id"] = json!(Uuid::from_u128(201));
    late["artifacts"][0]["id"] = json!(Uuid::from_u128(202));
    late["artifacts"][0]["run_id"] = json!(Uuid::from_u128(999));
    fs::write(&path, serde_json::to_vec(&late).unwrap())
        .await
        .unwrap();
    let error = restore_history::merge(&fixture.pool, &path, 10000)
        .await
        .unwrap_err();
    results.push(json!({"case":"late-foreign-key","error":error.to_string(),"rows":catalog_rows(&fixture.pool).await}));
    let actual = json!(results);
    evidence("history", &actual);
    fixture.cleanup().await;
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(include_str!("restore_execution/history.json")).unwrap()
    );
}

#[test]
fn bounded_settings_json_exact_bytes_null_and_trailing_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    for (bytes, maximum, expected) in [
        (b"null".as_slice(), 4, Some(Value::Null)),
        (b"{}".as_slice(), 2, Some(json!({}))),
        (
            br#"{"present":false}"#.as_slice(),
            17,
            Some(json!({"present":false})),
        ),
        (b"null ".as_slice(), 4, None),
        (b"null true".as_slice(), 9, None),
        (b"".as_slice(), 0, None),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let result = restore_history::read_bounded_json::<Value>(&path, maximum);
        assert_eq!(result.ok(), expected);
    }
}

#[tokio::test]
#[ignore = "requires the explicitly isolated restore fixture server and PostgreSQL clients"]
async fn candidate_package_and_preparation_fault_contract() {
    let fixture = DatabaseFixture::new().await;
    fixture.pool.execute(
        "INSERT INTO panda.users(id,email,password_hash,created_at,updated_at) VALUES
         ('00000000-0000-0000-0000-000000000301','synthetic@example.invalid','fixture-only','2020-01-01','2020-01-01');
         INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at)
         VALUES ('00000000-0000-0000-0000-000000000302','00000000-0000-0000-0000-000000000301','synthetic-vehicle','R1T','2020-01-01','2020-01-01')"
    ).await.unwrap();
    let backup = backups::run_backup_now(
        &fixture.pool,
        &fixture.config,
        None,
        backups::BackupRunTrigger::Manual,
    )
    .await
    .unwrap();
    let package: String =
        sqlx::query_scalar("SELECT storage_path FROM panda.backup_artifacts WHERE run_id=$1")
            .bind(backup.run_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let validated = backups::validate_recovery_package_with_limits(
        Path::new(&package),
        &fixture.config.recovery,
    )
    .await
    .unwrap();
    fs::copy(
        &package,
        Path::new(&std::env::var("RESTORE_CONTRACT_ARTIFACT_DIR").unwrap())
            .join("historical-fixture.rma.tar.gz"),
    )
    .await
    .unwrap();
    let staging = fixture.directory.path().join("extracted");
    extract_package(&fixture.config, Path::new(&package), &staging)
        .await
        .unwrap();
    let mut outcomes = Vec::new();
    for (index, phase) in [Some("timescale_pre_restore"), Some("dump_restored"), None]
        .into_iter()
        .enumerate()
    {
        let id = Uuid::from_u128(400 + index as u128);
        if let Some(phase) = phase {
            std::env::set_var("PANDA_RESTORE_FAULT_PHASE", phase);
        }
        let result = prepare_restore_database(
            &fixture.config,
            id,
            &staging.join("database.dump"),
            &validated.manifest,
            &staging.join("backup-settings.json"),
            Some(&staging.join("operational-history.json")),
            None,
        )
        .await;
        std::env::remove_var("PANDA_RESTORE_FAULT_PHASE");
        match result {
            Err(error) => {
                outcomes.push(json!({"phase":phase,"error":error.to_string()}));
                assert!(phase.is_some(), "valid candidate preparation: {error:#}");
                assert!(!database_exists(
                    &fixture.config,
                    &format!("panda_restore_{}", id.simple())
                )
                .await
                .unwrap());
            }
            Ok(prepared) => {
                assert!(phase.is_none());
                let candidate_config =
                    config_with_database(&fixture.config, &prepared.candidate_database).unwrap();
                let candidate = PgPool::connect(&candidate_config.database_url)
                    .await
                    .unwrap();
                let mut rows = serde_json::Map::new();
                for table in [
                    "users",
                    "vehicles",
                    "backup_settings",
                    "authentication_settings",
                ] {
                    let data: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                        "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY id"
                    )))
                    .fetch_all(&candidate)
                    .await
                    .unwrap();
                    rows.insert(table.into(), json!(data));
                }
                let attributes: (bool, bool, bool, bool, bool, Option<String>) = sqlx::query_as(
                    "SELECT rolsuper,rolcreatedb,rolcreaterole,rolbypassrls,rolcanlogin,rolpassword FROM pg_authid WHERE rolname=$1"
                ).bind(format!("panda_restore_owner_{}", id.simple())).fetch_one(&fixture.admin).await.unwrap();
                assert_eq!(attributes, (false, false, false, false, false, None));
                outcomes.push(json!({"phase":phase,"rows":rows,"validation":prepared.validation_report,"role":attributes}));
                candidate.close().await;
                cleanup_candidate(&fixture.config, &prepared.candidate_database).await;
            }
        }
    }
    let mut actual = json!(outcomes);
    evidence("candidate-raw", &actual);
    actual[2]["rows"]["authentication_settings"][0]["updated_at"] = json!("<clock>");
    evidence("candidate", &actual);
    fixture.cleanup().await;
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(include_str!("restore_execution/candidate.json")).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires the explicitly isolated restore fixture server and PostgreSQL clients"]
async fn runtime_faults_capture_complete_journal_and_database_outcomes() {
    let fixture = DatabaseFixture::new().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture.config.clone();
    config.port = listener.local_addr().unwrap().port();
    let health = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/health", get(|| async { StatusCode::OK }))
                .route(
                    "/v1/auth/setup",
                    get(|| async { Json(json!({"setup_required":false})) }),
                ),
        )
        .await
        .unwrap();
    });
    fs::create_dir_all(&config.vehicle_image_cache_dir)
        .await
        .unwrap();
    fs::write(
        Path::new(&config.vehicle_image_cache_dir).join("car"),
        b"synthetic-artwork",
    )
    .await
    .unwrap();
    let backup = backups::run_backup_now(
        &fixture.pool,
        &config,
        None,
        backups::BackupRunTrigger::Manual,
    )
    .await
    .unwrap();
    let package: String =
        sqlx::query_scalar("SELECT storage_path FROM panda.backup_artifacts WHERE run_id=$1")
            .bind(backup.run_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let validated =
        backups::validate_recovery_package_with_limits(Path::new(&package), &config.recovery)
            .await
            .unwrap();
    let inspection = restore_compatibility::inspect_recovery_dump(
        Path::new(&package),
        Path::new(&config.backup_artifact_dir),
        &config.recovery,
    )
    .await
    .unwrap();
    let plan = restore_compatibility::plan_restore(
        &validated.manifest,
        &validated.checksum_sha256,
        &fixture.pool,
        Some(&inspection),
    )
    .await
    .unwrap();
    assert!(plan.compatible, "{plan:?}");
    fixture.pool.close().await;
    let mut outcomes = Vec::new();
    let mut raw = Vec::new();
    for (index, fault) in [
        Some("package_validated"),
        Some("timescale_pre_restore"),
        Some("dump_restored"),
        Some("candidate_validated"),
        Some("safety_backup"),
        Some("history_merged"),
        Some("database_swapped"),
        Some("artwork_activated"),
        Some("health_verification"),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        let id = Uuid::from_u128(500 + index as u128);
        let mut job = fixed_job();
        job.id = id;
        job.artifact_path = package.clone();
        job.plan = Some(plan.clone());
        restore_jobs::write(&config, &job).await.unwrap();
        if let Some(fault) = fault {
            std::env::set_var("PANDA_RESTORE_FAULT_PHASE", fault);
        }
        let result = perform_restore_inner(&config, id).await;
        std::env::remove_var("PANDA_RESTORE_FAULT_PHASE");
        let error = result.err().map(|error| error.to_string());
        let journal = restore_jobs::read(&config, id).await.unwrap();
        let active = PgPool::connect(&config.database_url).await.unwrap();
        let catalog = catalog_rows(&active).await;
        let restored: Vec<Value> =
            sqlx::query_scalar("SELECT to_jsonb(t) FROM panda.vehicles t ORDER BY id")
                .fetch_all(&active)
                .await
                .unwrap();
        active.close().await;
        let candidate_exists = database_exists(&config, &format!("panda_restore_{}", id.simple()))
            .await
            .unwrap();
        let previous_exists = database_exists(&config, &format!("panda_previous_{}", id.simple()))
            .await
            .unwrap();
        raw.push(json!({"fault":fault,"error":error,"journal":journal,"catalog":catalog,"vehicles":restored}));
        outcomes.push(json!({
            "fault":fault, "error":error, "phase":journal.phase,
            "progress_percent":journal.progress_percent, "message":journal.message,
            "retryable":journal.retryable, "rollback_state":journal.rollback_state,
            "candidate_exists":candidate_exists,"previous_exists":previous_exists,
            "marker_exists":restore_marker_path().exists(),
            "artwork":fs::read(Path::new(&config.vehicle_image_cache_dir).join("car")).await.unwrap(),
            "vehicles":restored
        }));
        if fault.is_some() {
            assert!(error.is_some(), "fault {fault:?} must fail");
        } else {
            assert!(error.is_none(), "successful restore: {error:?}");
        }
        cleanup_candidate(&config, &format!("panda_restore_{}", id.simple())).await;
        assert!(!previous_exists);
        let _ = fs::remove_file(restore_marker_path()).await;
    }
    evidence("runtime-raw", &json!(raw));
    let actual = json!(outcomes);
    evidence("runtime", &actual);
    health.abort();
    let _ = health.await;
    fixture.cleanup().await;
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(include_str!("restore_execution/runtime.json")).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires the explicitly isolated restore fixture server"]
async fn restart_retry_admission_limit_and_full_journal_contract() {
    let fixture = DatabaseFixture::new().await;
    let mut records = Vec::new();
    for (index, phase, retryable, count, staged) in [
        (0, RestorePhase::Queued, true, 0, true),
        (1, RestorePhase::Completed, true, 0, true),
        (2, RestorePhase::Failed, false, 0, true),
        (3, RestorePhase::Failed, true, 2, true),
        (4, RestorePhase::Failed, true, 0, false),
        (5, RestorePhase::Failed, true, 0, true),
        (6, RestorePhase::Failed, true, 0, true),
    ] {
        let mut config = fixture.config.clone();
        if index == 6 {
            config.jwt_secret = None;
            config.jwt_public_key = None;
            config.age_encryption_key = None;
        }
        config.backup_artifact_dir = fixture
            .directory
            .path()
            .join(format!("retry-{index}"))
            .to_string_lossy()
            .into();
        let mut job = fixed_job();
        job.phase = phase;
        job.retryable = retryable;
        job.automatic_retry_count = count;
        restore_jobs::write(&config, &job).await.unwrap();
        if staged {
            let staging = Path::new(&config.backup_artifact_dir)
                .join(".restore-staging")
                .join(job.id.to_string());
            fs::create_dir_all(&staging).await.unwrap();
            fs::write(staging.join("manifest.json"), b"{}")
                .await
                .unwrap();
            fs::write(staging.join("database.dump"), b"synthetic-incomplete-dump")
                .await
                .unwrap();
        }
        let state = AgentState {
            config: config.clone(),
            agent_key: "synthetic-agent-key".into(),
        };
        retry_interrupted_restore(&state).await.unwrap();
        let after = restore_jobs::read(&config, job.id).await.unwrap();
        let mut raw = serde_json::to_value(&after).unwrap();
        evidence(&format!("retry-{index}-raw"), &raw);
        if index >= 5 {
            assert_eq!(after.automatic_retry_count, 1);
            assert_eq!(after.phase, RestorePhase::Failed);
            raw["updated_at"] = json!("<clock>");
            retry_interrupted_restore(&state).await.unwrap();
            let retried = restore_jobs::read(&config, job.id).await.unwrap();
            if index == 5 {
                assert_eq!(
                    serde_json::to_value(&after).unwrap(),
                    serde_json::to_value(&retried).unwrap(),
                    "missing staging after failed re-admission prevents another retry"
                );
            } else {
                assert_eq!(retried.automatic_retry_count, 2);
                retry_interrupted_restore(&state).await.unwrap();
                assert_eq!(
                    serde_json::to_value(&retried).unwrap(),
                    serde_json::to_value(restore_jobs::read(&config, job.id).await.unwrap())
                        .unwrap()
                );
            }
        } else {
            assert_eq!(raw, serde_json::to_value(job).unwrap());
        }
        records.push(raw);
    }
    let actual = json!(records);
    evidence("retry", &actual);
    fixture.cleanup().await;
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(include_str!("restore_execution/retry.json")).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires the explicitly isolated restore fixture server and PostgreSQL clients"]
async fn host_prepare_activate_retry_rollback_and_finalize_contract() {
    let fixture = DatabaseFixture::new().await;
    let config = &fixture.config;
    fs::create_dir_all(&config.vehicle_image_cache_dir)
        .await
        .unwrap();
    fs::write(
        Path::new(&config.vehicle_image_cache_dir).join("car"),
        b"host-artwork",
    )
    .await
    .unwrap();
    let backup = backups::run_backup_now(
        &fixture.pool,
        config,
        None,
        backups::BackupRunTrigger::Manual,
    )
    .await
    .unwrap();
    let package: String =
        sqlx::query_scalar("SELECT storage_path FROM panda.backup_artifacts WHERE run_id=$1")
            .bind(backup.run_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    fixture.pool.close().await;
    let mut outcomes = Vec::new();
    for finalize in [false, true] {
        host_restore(config, Path::new(&package), false)
            .await
            .unwrap();
        let prepared = read_host_restore_state(config).await.unwrap();
        assert!(!prepared.activated);
        assert!(!prepared.activation_started);
        let pending = host_restore(config, Path::new(&package), false)
            .await
            .unwrap_err();
        assert_eq!(
            pending.to_string(),
            "a host restore is already awaiting finalize or rollback"
        );
        let premature = finalize_host_restore(config).await.unwrap_err();
        assert_eq!(
            premature.to_string(),
            "host restore candidate has not been activated"
        );
        activate_host_restore(config).await.unwrap();
        let activated = read_host_restore_state(config).await.unwrap();
        assert!(activated.activated);
        assert!(activated.activation_started);
        let state_before = fs::read(host_restore_state_path(config)).await.unwrap();
        activate_host_restore(config).await.unwrap();
        assert_eq!(
            state_before,
            fs::read(host_restore_state_path(config)).await.unwrap()
        );
        if finalize {
            finalize_host_restore(config).await.unwrap();
        } else {
            rollback_host_restore(config).await.unwrap();
            cleanup_candidate(config, &activated.candidate_database).await;
        }
        assert!(!host_restore_state_path(config).exists());
        let pool = PgPool::connect(&config.database_url).await.unwrap();
        let run: Value =
            sqlx::query_scalar("SELECT to_jsonb(t) FROM panda.backup_runs t WHERE id=$1")
                .bind(prepared.job_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let artifact: Value =
            sqlx::query_scalar("SELECT to_jsonb(t) FROM panda.backup_artifacts t WHERE id=$1")
                .bind(prepared.artifact_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let request: Value = sqlx::query_scalar(
            "SELECT to_jsonb(t) FROM panda.backup_restore_requests t WHERE id=$1",
        )
        .bind(prepared.request_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let report: Value = serde_json::from_slice(
            &fs::read(
                Path::new(&config.backup_artifact_dir)
                    .join(".restore-reports")
                    .join(format!("{}.json", prepared.job_id)),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        evidence(
            if finalize {
                "host-finalize-raw"
            } else {
                "host-rollback-raw"
            },
            &json!({"state":activated,"run":run,"artifact":artifact,"request":request,"report":report}),
        );
        assert_eq!(artifact["run_id"], json!(prepared.job_id));
        assert_eq!(
            artifact["checksum_sha256"],
            json!(prepared.plan.package_checksum_sha256)
        );
        assert_eq!(request["artifact_id"], json!(prepared.artifact_id));
        assert_eq!(
            report["plan"],
            serde_json::to_value(&prepared.plan).unwrap()
        );
        assert_eq!(
            report["validation_report"],
            serde_json::to_value(&prepared.validation_report).unwrap()
        );
        outcomes.push(json!({
            "finalize":finalize,"run_status":run["status"],"run_phase":run["phase"],
            "progress":run["progress_percent"],"request_status":request["status"],
            "request_error":request["error_message"],"report_status":report["status"],
            "report_error":report["error"],"artifact_manifest":artifact["manifest"]["restore_availability"],
            "artwork":fs::read(Path::new(&config.vehicle_image_cache_dir).join("car")).await.unwrap()
        }));
        pool.close().await;
    }
    let actual = json!(outcomes);
    evidence("host", &actual);
    fixture.cleanup().await;
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(include_str!("restore_execution/host.json")).unwrap()
    );
}

async fn cleanup_candidate(config: &Config, database: &str) {
    for attempt in 0..5 {
        match drop_database(config, database).await {
            Ok(()) => return,
            Err(error) if attempt < 4 => {
                eprintln!("synthetic fixture cleanup retry: {error}");
                sleep(Duration::from_millis(200)).await;
            }
            Err(error) => panic!("synthetic candidate cleanup: {error}"),
        }
    }
}

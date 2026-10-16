use super::*;
use axum::response::IntoResponse;
use serde_json::{json, Value};
use sqlx::{Acquire, Executor, PgPool};
use std::{collections::BTreeMap, io::Read};

struct DatabaseFixture {
    url: String,
    admin: PgPool,
    pool: PgPool,
    database: String,
    role: String,
}

impl DatabaseFixture {
    async fn new() -> Self {
        use sqlx::Executor;
        let mut url = url::Url::parse(
            &std::env::var("DEMO_CONTRACT_DATABASE_URL")
                .expect("explicit disposable DEMO_CONTRACT_DATABASE_URL required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "demo_fixture");
        assert_eq!(url.path(), "/demo_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("demo_role_{suffix}");
        let database = format!("demo_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_demo_app'"
        ))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA panda AUTHORIZATION {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SCHEMA panda FROM PUBLIC;
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_demo_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(5)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({
            "database_url":url.as_str(), "redis_url":"redis://unused", "panda_env":"production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        Self {
            url: url.to_string(),
            admin,
            pool,
            database,
            role,
        }
    }

    async fn cleanup(self) {
        use sqlx::Executor;
        self.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH (FORCE)",
                self.database
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
    }
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn anchor() -> DateTime<Utc> {
    "2024-03-01T00:00:00.999999999Z".parse().unwrap()
}

const TABLES: &[(&str, &str)] = &[
    ("panda.users", "email"),
    ("panda.vehicles", "rivian_vehicle_id"),
    ("panda.vehicle_memberships", "user_id,role"),
    ("panda.addresses", "osm_id"),
    ("panda.trips", "started_at"),
    ("panda.charge_sessions", "started_at"),
    ("panda.trip_weather_samples", "sampled_at,elapsed_seconds"),
    ("panda.weather_enrichment_jobs", "trip_id"),
    ("panda.rivian_charge_curve_points", "ts"),
    ("timeseries.telemetry", "ts"),
    ("panda.vehicle_state_periods", "started_at,state"),
    ("panda.software_versions", "installed_at"),
    ("panda.battery_capacity_snapshots", "snapshotted_at"),
    ("panda.vehicle_images", "id"),
    ("panda.vehicle_latest_status", "vehicle_id"),
    ("panda.vehicle_runtime_state", "vehicle_id"),
];

async fn snapshot(tx: &mut Transaction<'_, Postgres>) -> Value {
    let mut tables = serde_json::Map::new();
    for (table, order) in TABLES {
        let rows = sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM {table} t ORDER BY {order}"
        )))
        .fetch_all(&mut **tx)
        .await
        .unwrap();
        tables.insert((*table).into(), json!(rows));
    }
    Value::Object(tables)
}

fn normalize(mut data: Value, clock: &Value) -> Value {
    let mut identities = BTreeMap::new();
    for (table, rows) in data.as_object().unwrap() {
        for (index, row) in rows.as_array().unwrap().iter().enumerate() {
            if let Some(Value::String(value)) = row.get("id") {
                if Uuid::parse_str(value)
                    .is_ok_and(|uuid| ![id(1), id(2), id(10), id(20)].contains(&uuid))
                {
                    identities.insert(value.clone(), format!("{table}:{index}"));
                }
            }
        }
    }
    fn visit(value: &mut Value, identities: &BTreeMap<String, String>, clock: &Value) {
        if value == clock {
            *value = json!("<transaction-clock>");
            return;
        }
        match value {
            Value::String(s) => {
                if let Some(label) = identities.get(s) {
                    *s = label.clone();
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, identities, clock);
                }
            }
            Value::Object(m) => {
                for v in m.values_mut() {
                    visit(v, identities, clock);
                }
            }
            _ => {}
        }
    }
    visit(&mut data, &identities, clock);
    data["panda.weather_enrichment_jobs"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|v| v["trip_id"].as_str().unwrap().to_owned());
    data
}

async fn seed_identity(tx: &mut Transaction<'_, Postgres>, model: &str) {
    sqlx::query("INSERT INTO panda.users(id,email,password_hash,role) VALUES ($1,'demo-contract@example.invalid','inert synthetic','admin')")
        .bind(id(1)).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,name) VALUES ($1,$2,'demo-contract',$3,'Keep demo name')")
        .bind(id(10)).bind(id(1)).bind(if matches!(model,"R2S"|"R2-S") {"R2"} else {model}).execute(&mut **tx).await.unwrap();
}

async fn error_output(error: AppError) -> Value {
    let diagnostic = match &error {
        AppError::Database(e) => {
            json!({"sqlstate":e.as_database_error().and_then(|e| e.code()).map(|s|s.into_owned()),"message":e.as_database_error().map(|e|e.message())})
        }
        _ => json!({"display":error.to_string()}),
    };
    let response = error.into_response();
    let status = response.status().as_u16();
    let headers: BTreeMap<_, _> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap().to_owned()))
        .collect();
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    json!({"status":status,"headers":headers,"body":serde_json::from_slice::<Value>(&body).unwrap(),"diagnostic":diagnostic})
}

fn evidence_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("DEMO_CONTRACT_ARTIFACT_DIR")
        .map(|root| std::path::PathBuf::from(root).join(name))
}

fn evidence(name: &str, value: &Value) {
    if let Some(path) = evidence_path(&format!("{name}.json")) {
        std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_complete_rows_and_guard_contracts() {
    let fixture = DatabaseFixture::new().await;
    let mut output = serde_json::Map::new();
    for model in ["R1T", "R1S", "R2", "R2S", "R2-S"] {
        let mut tx = fixture.pool.begin().await.unwrap();
        seed_identity(&mut tx, model).await;
        let clock: Value = sqlx::query_scalar("SELECT to_jsonb(now())")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let summary = seed_new_demo_vehicle(&mut tx, id(10), model, anchor())
            .await
            .unwrap();
        let raw = snapshot(&mut tx).await;
        evidence(&format!("raw-{model}"), &raw);
        let normalized = normalize(raw.clone(), &clock);
        output.insert(model.into(), json!({"summary":summary,"tables":normalized}));
        let denied = seed_new_demo_vehicle(&mut tx, id(10), "invalid", anchor())
            .await
            .unwrap_err();
        output.insert(format!("{model}-repeat"), error_output(denied).await);
        assert_eq!(raw, snapshot(&mut tx).await, "denial changed history");
        let mut savepoint = tx.begin().await.unwrap();
        let denied = seed_demo_vehicle(&mut savepoint, id(10), model, anchor())
            .await
            .unwrap_err();
        output.insert(format!("{model}-refresh"), error_output(denied).await);
        savepoint.rollback().await.unwrap();
        assert_eq!(
            raw,
            snapshot(&mut tx).await,
            "protected refresh changed history"
        );
        tx.rollback().await.unwrap();
    }
    let mut tx = fixture.pool.begin().await.unwrap();
    seed_identity(&mut tx, "R1S").await;
    let raw = snapshot(&mut tx).await;
    for (name, vehicle, model, fresh) in [
        ("fresh-missing-invalid", id(999), "bad", true),
        ("refresh-missing-invalid", id(999), "bad", false),
        ("fresh-invalid", id(10), "bad", true),
        ("refresh-invalid", id(10), "bad", false),
    ] {
        let result = if fresh {
            seed_new_demo_vehicle(&mut tx, vehicle, model, anchor()).await
        } else {
            seed_demo_vehicle(&mut tx, vehicle, model, anchor()).await
        };
        output.insert(name.into(), error_output(result.unwrap_err()).await);
        assert_eq!(raw, snapshot(&mut tx).await);
    }
    sqlx::query("UPDATE panda.vehicles SET rivian_vehicle_id='ordinary-synthetic' WHERE id=$1")
        .bind(id(10))
        .execute(&mut *tx)
        .await
        .unwrap();
    for fresh in [true, false] {
        let error = if fresh {
            seed_new_demo_vehicle(&mut tx, id(10), "R1S", anchor()).await
        } else {
            seed_demo_vehicle(&mut tx, id(10), "R1S", anchor()).await
        }
        .unwrap_err();
        output.insert(format!("non-demo-{fresh}"), error_output(error).await);
    }
    for role in ["user", "super_user", "admin"] {
        for disabled in [false, true] {
            sqlx::query("UPDATE panda.users SET role=$2,is_disabled=$3 WHERE id=$1")
                .bind(id(1))
                .bind(role)
                .bind(disabled)
                .execute(&mut *tx)
                .await
                .unwrap();
            let result = lock_demo_creator(&mut tx, id(1)).await;
            output.insert(
                format!("creator-{role}-{disabled}"),
                match result {
                    Ok(()) => json!({"ok":true}),
                    Err(e) => error_output(e).await,
                },
            );
        }
    }
    output.insert(
        "creator-missing".into(),
        error_output(lock_demo_creator(&mut tx, id(999)).await.unwrap_err()).await,
    );
    tx.rollback().await.unwrap();
    for table in [
        "timeseries.telemetry",
        "panda.trips",
        "panda.vehicles",
        "panda.vehicle_state_periods",
        "panda.software_versions",
        "panda.battery_capacity_snapshots",
    ] {
        for verb in ["DELETE FROM", "TRUNCATE"] {
            let error = fixture
                .pool
                .execute(sqlx::AssertSqlSafe(format!("{verb} {table}")))
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("42501")
            );
        }
    }
    let value = Value::Object(output);
    evidence("normalized", &value);
    fixture.cleanup().await;
    let mut expected = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("demo_generation/expected.json.gz")[..])
        .read_to_string(&mut expected)
        .unwrap();
    let expected: Value = serde_json::from_str(&expected).unwrap();
    assert_eq!(
        value.as_object().unwrap().keys().collect::<Vec<_>>(),
        expected.as_object().unwrap().keys().collect::<Vec<_>>(),
        "case inventory"
    );
    for (case, actual) in value.as_object().unwrap() {
        assert!(
            actual == &expected[case],
            "complete output differs for {case}; inspect normalized.json"
        );
    }
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_stage_failure_retry_and_concurrency() {
    let fixture = DatabaseFixture::new().await;
    let mut results = Vec::new();
    for table in [
        "panda.trips",
        "panda.trip_weather_samples",
        "panda.charge_sessions",
        "timeseries.telemetry",
        "panda.vehicle_state_periods",
        "panda.software_versions",
        "panda.vehicle_latest_status",
        "panda.vehicle_runtime_state",
    ] {
        let mut tx = fixture.pool.begin().await.unwrap();
        seed_identity(&mut tx, "R1S").await;
        let before = snapshot(&mut tx).await;
        let mut attempt = tx.begin().await.unwrap();
        attempt.execute("CREATE OR REPLACE FUNCTION pg_temp.demo_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic demo stage failure'; END $$").await.unwrap();
        attempt.execute(sqlx::AssertSqlSafe(format!("CREATE TRIGGER demo_stage_failure BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION pg_temp.demo_fail()"))).await.unwrap();
        let failure = seed_new_demo_vehicle(&mut attempt, id(10), "R1S", anchor())
            .await
            .unwrap_err();
        let failure = error_output(failure).await;
        assert_eq!(failure["diagnostic"]["sqlstate"], "P0001");
        attempt.rollback().await.unwrap();
        assert_eq!(
            before,
            snapshot(&mut tx).await,
            "late failure rollback at {table}"
        );
        let retry = seed_new_demo_vehicle(&mut tx, id(10), "R1S", anchor())
            .await
            .unwrap();
        let clock: Value = sqlx::query_scalar("SELECT to_jsonb(now())")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let raw = snapshot(&mut tx).await;
        evidence(&format!("raw-fault-{}", table.replace('.', "-")), &raw);
        results.push(
            json!({"stage":table,"error":failure,"retry":retry,"tables":normalize(raw,&clock)}),
        );
        tx.rollback().await.unwrap();
    }
    let mut tx = fixture.pool.begin().await.unwrap();
    seed_identity(&mut tx, "R1S").await;
    tx.commit().await.unwrap();
    let mut first = fixture.pool.begin().await.unwrap();
    seed_new_demo_vehicle(&mut first, id(10), "R1S", anchor())
        .await
        .unwrap();
    let before = snapshot(&mut first).await;
    let pool = fixture.pool.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    let contender = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        send.send(pid).unwrap();
        let result = seed_new_demo_vehicle(&mut tx, id(10), "R1T", anchor()).await;
        tx.rollback().await.unwrap();
        error_output(result.unwrap_err()).await
    });
    let pid = receive.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT cardinality(pg_blocking_pids($1)) > 0")
                .bind(pid)
                .fetch_one(&fixture.pool)
                .await
                .unwrap();
            if blocked {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("second seeder must wait for vehicle row lock");
    first.commit().await.unwrap();
    let result = contender.await.unwrap();
    assert_eq!(result["status"], 409);
    let mut reconnected = fixture.pool.begin().await.unwrap();
    assert_eq!(
        before,
        snapshot(&mut reconnected).await,
        "concurrent denial changed committed history"
    );
    let retry = seed_new_demo_vehicle(&mut reconnected, id(10), "R1S", anchor())
        .await
        .unwrap_err();
    assert_eq!(error_output(retry).await, result);
    reconnected.rollback().await.unwrap();
    evidence(
        "concurrency",
        &json!({"blocked_on_vehicle_row":true,"contender":result,"restart_same_denial":true}),
    );
    let results = json!(results);
    evidence("faults-normalized", &results);
    fixture.cleanup().await;
    let mut expected = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("demo_generation/faults-expected.json.gz")[..])
        .read_to_string(&mut expected)
        .unwrap();
    let expected: Value = serde_json::from_str(&expected).unwrap();
    assert!(
        results == expected,
        "fault/retry full rows differ; inspect faults-normalized.json"
    );
}

#[tokio::test]
async fn demo_generation_validation_boundaries() {
    let mut cases = Vec::new();
    for (name, patch) in [
        (
            "schema-first",
            json!({"schema_version":0,"telemetry_rows":0}),
        ),
        (
            "source-first",
            json!({"source_model":"R1T","telemetry_rows":0}),
        ),
        ("window", json!({"window_days":15})),
        ("rows-low", json!({"telemetry_rows":999})),
        ("rows-high", json!({"telemetry_rows":20001})),
        ("rows-min", json!({"telemetry_rows":1000})),
        ("rows-max", json!({"telemetry_rows":20000})),
        ("active-low", json!({"active_days":6})),
        ("active-high", json!({"active_days":15})),
        ("trips-low", json!({"trip_count":9})),
        ("trips-high", json!({"trip_count":101})),
        ("charges-low", json!({"charge_count":1})),
        ("charges-high", json!({"charge_count":21})),
        ("weather-low", json!({"weather_sample_count":30})),
        ("null-rows", json!({"telemetry_rows":null})),
        ("unknown-field", json!({"unexpected":"synthetic"})),
    ] {
        let mut input: Value = serde_json::from_str(FIXTURE_JSON).unwrap();
        input
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let result = match serde_json::from_value::<DemoFixtureSpec>(input.clone()) {
            Ok(spec) => match validate_fixture(&spec) {
                Ok(()) => json!({"ok":true}),
                Err(e) => error_output(e).await,
            },
            Err(e) => json!({"decode_error":e.to_string()}),
        };
        cases.push(json!({"name":name,"input":input,"result":result}));
    }
    for value in [0.0, 1.0, -0.01, 1.01, f64::NAN, f64::INFINITY] {
        let mut spec = load_fixture().unwrap();
        spec.coverage.location = value;
        let result = match validate_fixture(&spec) {
            Ok(()) => json!({"ok":true}),
            Err(e) => error_output(e).await,
        };
        cases.push(json!({"coverage_bits":value.to_bits(),"result":result}));
    }
    for model in ["R1T", "R1S", "R2", "R2S", "R2-S", "", "r1s", " R2", "R3"] {
        let result = match model_profile(model) {
            Ok(p) => {
                json!({"capacity_wh":p.capacity_wh,"max_range_mi":p.max_range_mi,"liftgate":p.has_liftgate,"truck_closures":p.has_truck_closures})
            }
            Err(e) => error_output(e).await,
        };
        cases.push(json!({"model":model,"result":result}));
    }
    let value = json!(cases);
    evidence("validation", &value);
    let expected: Value =
        serde_json::from_str(include_str!("demo_generation/validation.json")).unwrap();
    assert!(
        value == expected,
        "validation output differs; inspect validation.json"
    );
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_sparse_timestamp_and_batch_boundaries() {
    let fixture = DatabaseFixture::new().await;
    let mut output = Vec::new();
    for (rows, coverage) in [(0, 0.0), (24, 0.0), (24, 1.0)] {
        let mut tx = fixture.pool.begin().await.unwrap();
        seed_identity(&mut tx, "R1T").await;
        let mut spec = load_fixture().unwrap();
        spec.telemetry_rows = rows;
        spec.coverage.location = coverage;
        spec.coverage.battery = coverage;
        spec.coverage.tires = coverage;
        spec.coverage.doors = coverage;
        seed_telemetry(
            &mut tx,
            id(10),
            anchor().with_nanosecond(0).unwrap(),
            &spec,
            model_profile("R1T").unwrap(),
        )
        .await
        .unwrap();
        let clock: Value = sqlx::query_scalar("SELECT to_jsonb(now())")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let raw = snapshot(&mut tx).await;
        evidence(&format!("raw-sparse-{rows}-{coverage}"), &raw);
        output.push(json!({"rows":rows,"coverage":coverage,"tables":normalize(raw,&clock)}));
        tx.rollback().await.unwrap();
    }
    let mut tx = fixture.pool.begin().await.unwrap();
    seed_identity(&mut tx, "R1S").await;
    let mut spec = load_fixture().unwrap();
    spec.telemetry_rows = 25;
    let error = seed_telemetry(
        &mut tx,
        id(10),
        anchor(),
        &spec,
        model_profile("R1S").unwrap(),
    )
    .await
    .unwrap_err();
    output.push(error_output(error).await);
    tx.rollback().await.unwrap();
    let value = json!(output);
    evidence("sparse-normalized", &value);
    fixture.cleanup().await;
    let mut expected = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("demo_generation/sparse-expected.json.gz")[..])
        .read_to_string(&mut expected)
        .unwrap();
    let expected: Value = serde_json::from_str(&expected).unwrap();
    assert!(value == expected, "sparse complete rows differ");
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_interrupted_connection_rolls_back_and_restarts() {
    let fixture = DatabaseFixture::new().await;
    let mut tx = fixture.pool.begin().await.unwrap();
    seed_identity(&mut tx, "R1S").await;
    let before = snapshot(&mut tx).await;
    tx.commit().await.unwrap();
    fixture.pool.execute("CREATE FUNCTION panda.demo_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(30); RETURN NEW; END $$; CREATE TRIGGER demo_pause BEFORE INSERT ON panda.vehicle_runtime_state FOR EACH ROW EXECUTE FUNCTION panda.demo_pause()").await.unwrap();
    let pool = fixture.pool.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    let writer = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        send.send(pid).unwrap();
        seed_new_demo_vehicle(&mut tx, id(10), "R1S", anchor()).await
    });
    let pid = receive.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30),async {
        loop {
            let paused:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event='PgSleep')").bind(pid).fetch_one(&fixture.pool).await.unwrap();
            if paused { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("reach final-stage interruption point");
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert!(killed);
    let result = writer.await.unwrap();
    assert!(result.is_err());
    fixture.pool.execute("DROP TRIGGER demo_pause ON panda.vehicle_runtime_state; DROP FUNCTION panda.demo_pause()").await.unwrap();
    let mut restarted = fixture.pool.begin().await.unwrap();
    assert_eq!(
        before,
        snapshot(&mut restarted).await,
        "terminated writer left partial history"
    );
    let summary = seed_new_demo_vehicle(&mut restarted, id(10), "R1S", anchor())
        .await
        .unwrap();
    assert_eq!(summary.counts.telemetry, 5664);
    evidence(
        "interruption",
        &json!({"terminated_at":"vehicle_runtime_state","complete_rollback":true,"retry":summary,"error":error_output(result.unwrap_err()).await}),
    );
    restarted.rollback().await.unwrap();
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_legacy_refresh_test_remains_incompatible_with_protected_history() {
    let fixture = DatabaseFixture::new().await;
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "services::demo_seed::tests::seeds_every_model_with_stable_relational_counts",
            "--ignored",
            "--nocapture",
        ])
        .env("DATABASE_URL", &fixture.url)
        .output()
        .unwrap();
    let mut log = result.stdout;
    log.extend(result.stderr);
    if let Some(path) = evidence_path("legacy-regression.log") {
        std::fs::write(path, &log).unwrap();
    }
    assert!(
        !result.status.success(),
        "legacy test assumes history deletion is allowed"
    );
    let log = String::from_utf8(log).unwrap();
    assert!(
        log.contains("42501") && log.contains("permission denied for table telemetry"),
        "unexpected legacy failure: {log}"
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_vehicle_isolation_and_single_weather_samples() {
    let fixture = DatabaseFixture::new().await;
    let mut tx = fixture.pool.begin().await.unwrap();
    seed_identity(&mut tx, "R1S").await;
    sqlx::query("INSERT INTO panda.users(id,email,password_hash,role) VALUES ($1,'foreign-demo@example.invalid','inert synthetic','user')").bind(id(2)).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES ($1,$2,'demo-foreign','R1T')").bind(id(20)).bind(id(2)).execute(&mut *tx).await.unwrap();
    seed_new_demo_vehicle(&mut tx, id(10), "R1S", anchor())
        .await
        .unwrap();
    let original = snapshot(&mut tx).await;
    seed_new_demo_vehicle(&mut tx, id(20), "R1T", anchor())
        .await
        .unwrap();
    let both = snapshot(&mut tx).await;
    for (table, rows) in original.as_object().unwrap() {
        for row in rows.as_array().unwrap() {
            assert!(
                both[table].as_array().unwrap().contains(row),
                "second vehicle changed a complete existing row in {table}"
            );
        }
    }
    let bad_links:i64=sqlx::query_scalar("SELECT count(*) FROM timeseries.telemetry t LEFT JOIN panda.trips trip ON trip.id=t.trip_id LEFT JOIN panda.charge_sessions charge ON charge.id=t.charge_session_id WHERE (trip.id IS NOT NULL AND t.vehicle_id<>trip.vehicle_id) OR (charge.id IS NOT NULL AND t.vehicle_id<>charge.vehicle_id)").fetch_one(&mut *tx).await.unwrap();
    assert_eq!(bad_links, 0);
    evidence("isolation-raw", &both);
    tx.rollback().await.unwrap();
    let mut tx = fixture.pool.begin().await.unwrap();
    seed_identity(&mut tx, "R1S").await;
    let addresses = ensure_demo_addresses(&mut tx).await.unwrap();
    seed_trips(&mut tx, id(10), anchor(), 10, 10, &addresses)
        .await
        .unwrap();
    let samples:Vec<(i32,i32,f64,f64)>=sqlx::query_as("SELECT t.duration_seconds,w.elapsed_seconds,w.temperature_c,t.outside_temp_c FROM panda.trips t JOIN panda.trip_weather_samples w ON w.trip_id=t.id ORDER BY t.started_at").fetch_all(&mut *tx).await.unwrap();
    assert_eq!(samples.len(), 10);
    for (duration, elapsed, temp, outside) in &samples {
        assert_eq!(*elapsed, *duration / 2);
        assert_eq!(*temp, *outside + 0.9);
    }
    evidence("single-weather", &json!(samples));
    tx.rollback().await.unwrap();
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_exact_float_storage_bits() {
    let fixture = DatabaseFixture::new().await;
    let mut output = serde_json::Map::new();
    for model in ["R1T", "R1S", "R2"] {
        let mut tx = fixture.pool.begin().await.unwrap();
        seed_identity(&mut tx, model).await;
        seed_new_demo_vehicle(&mut tx, id(10), model, anchor())
            .await
            .unwrap();
        let mut tables = serde_json::Map::new();
        for (table, order) in TABLES {
            let columns:Vec<String>=sqlx::query_scalar("SELECT attname::text FROM pg_attribute WHERE attrelid=$1::regclass AND atttypid='float8'::regtype AND attnum>0 AND NOT attisdropped ORDER BY attname")
                .bind(table).fetch_all(&mut *tx).await.unwrap();
            if columns.is_empty() {
                continue;
            }
            let fields = columns
                .iter()
                .map(|column| {
                    assert!(column
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_'));
                    format!("'{column}',encode(float8send({column}),'hex')")
                })
                .collect::<Vec<_>>()
                .join(",");
            let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT jsonb_build_object({fields}) FROM {table} ORDER BY {order}"
            )))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
            tables.insert((*table).to_owned(), json!(rows));
        }
        output.insert(model.into(), Value::Object(tables));
        tx.rollback().await.unwrap();
    }
    let value = Value::Object(output);
    evidence("float-bits", &value);
    fixture.cleanup().await;
    let mut expected = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("demo_generation/float-bits.json.gz")[..])
        .read_to_string(&mut expected)
        .unwrap();
    let expected: Value = serde_json::from_str(&expected).unwrap();
    assert!(
        value == expected,
        "stored IEEE-754 values differ; inspect float-bits.json"
    );
}

#[tokio::test]
#[ignore = "requires a new disposable DEMO_CONTRACT_DATABASE_URL"]
async fn demo_generation_database_calendar_timezone_boundaries() {
    let fixture = DatabaseFixture::new().await;
    let mut output = Vec::new();
    for instant in ["2024-03-15T00:00:00Z", "2024-11-08T00:00:00Z"] {
        let mut tx = fixture.pool.begin().await.unwrap();
        tx.execute("SET LOCAL TIME ZONE 'America/New_York'")
            .await
            .unwrap();
        seed_identity(&mut tx, "R1S").await;
        let mut spec = load_fixture().unwrap();
        spec.telemetry_rows = 24;
        let clock: Value = sqlx::query_scalar("SELECT to_jsonb(now())")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        seed_telemetry(
            &mut tx,
            id(10),
            instant.parse().unwrap(),
            &spec,
            model_profile("R1S").unwrap(),
        )
        .await
        .unwrap();
        output.push(json!({"anchor":instant,"tables":normalize(snapshot(&mut tx).await,&clock)}));
        tx.rollback().await.unwrap();
    }
    let value = json!(output);
    evidence("timezone-normalized", &value);
    fixture.cleanup().await;
    let mut expected = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("demo_generation/timezone-expected.json.gz")[..])
        .read_to_string(&mut expected)
        .unwrap();
    assert!(
        value == serde_json::from_str::<Value>(&expected).unwrap(),
        "database calendar semantics changed"
    );
}

#[test]
fn demo_generation_identity_normalization_preserves_fixed_values() {
    let generated = "f0000000-0000-4000-8000-000000000001";
    let clock = json!("2026-10-16T00:00:00Z");
    let actual = normalize(
        json!({
            "panda.users":[{"id":id(1),"created_at":clock}],
            "panda.vehicles":[{"id":id(10),"user_id":id(1)}],
            "panda.trips":[{"id":generated,"vehicle_id":id(10),"started_at":"2024-03-01T00:00:00Z"}],
            "panda.weather_enrichment_jobs":[{"trip_id":generated}],
            "panda.vehicle_state_periods":[{"id":17,"vehicle_id":id(10)}]
        }),
        &clock,
    );
    assert_eq!(actual["panda.users"][0]["id"], json!(id(1)));
    assert_eq!(actual["panda.vehicles"][0]["id"], json!(id(10)));
    assert_eq!(actual["panda.vehicles"][0]["user_id"], json!(id(1)));
    assert_eq!(actual["panda.trips"][0]["vehicle_id"], json!(id(10)));
    assert_eq!(actual["panda.trips"][0]["id"], "panda.trips:0");
    assert_eq!(
        actual["panda.weather_enrichment_jobs"][0]["trip_id"],
        "panda.trips:0"
    );
    assert_eq!(actual["panda.vehicle_state_periods"][0]["id"], 17);
    assert_eq!(
        actual["panda.trips"][0]["started_at"],
        "2024-03-01T00:00:00Z"
    );
    assert_eq!(
        actual["panda.users"][0]["created_at"],
        "<transaction-clock>"
    );
}

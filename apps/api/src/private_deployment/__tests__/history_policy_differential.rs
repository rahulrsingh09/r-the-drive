use super::{protect_history, HISTORY_POLICY};
use crate::config::Config;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, Executor, PgConnection, PgPool};
use std::{collections::BTreeMap, time::Duration};

const PROTECTED: [&str; 6] = [
    "panda.vehicles",
    "panda.trips",
    "timeseries.telemetry",
    "panda.vehicle_state_periods",
    "panda.software_versions",
    "panda.battery_capacity_snapshots",
];
const RETENTION: [&str; 4] = [
    "timeseries.telemetry",
    "panda.trips",
    "panda.charge_sessions",
    "panda.rivian_charge_payloads",
];
const UNRELATED: [&str; 9] = [
    "history_policy_other.telemetry",
    "history_policy_other.trips",
    "history_policy_other.charge_sessions",
    "history_policy_other.rivian_charge_payloads",
    "panda.telemetry",
    "timeseries.trips",
    "timeseries.charge_sessions",
    "timeseries.rivian_charge_payloads",
    "panda.unrelated",
];
const PRIVILEGES: [&str; 8] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "TRUNCATE",
    "REFERENCES",
    "TRIGGER",
    "MAINTAIN",
];

fn config(environment: &str) -> Config {
    serde_json::from_value(json!({
        "database_url": "postgresql://synthetic",
        "redis_url": "redis://synthetic",
        "panda_env": environment
    }))
    .unwrap()
}

fn evidence(case: &str, phase: &str, value: impl Serialize) {
    println!(
        "HISTORY_POLICY_EVIDENCE {}",
        json!({"case": case, "phase": phase, "value": value})
    );
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct TableState {
    privileges: BTreeMap<String, bool>,
    acl: String,
    rows: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Snapshot {
    tables: BTreeMap<String, TableState>,
    jobs: Vec<Value>,
}

struct Fixture {
    admin: PgPool,
    bootstrap: PgPool,
    runtime: PgPool,
    name: String,
}

impl Fixture {
    async fn new(jobs: bool) -> Self {
        let mut url = url::Url::parse(
            &std::env::var("HISTORY_TEST_DATABASE_URL")
                .expect("explicit disposable TimescaleDB fixture URL"),
        )
        .unwrap();
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let name = format!("history_policy_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {name} LOGIN PASSWORD 'synthetic-history-policy-only'
                 NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
                 CREATE ROLE {name}_inherited NOLOGIN NOSUPERUSER NOCREATEDB
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS;"
            )))
            .await
            .unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .await
            .unwrap();
        url.set_path(&format!("/{name}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE EXTENSION IF NOT EXISTS timescaledb;
                 CREATE SCHEMA panda AUTHORIZATION {name};
                 CREATE SCHEMA timeseries AUTHORIZATION {name};
                 CREATE SCHEMA history_policy_other AUTHORIZATION {name};"
            )))
            .await
            .unwrap();
        url.set_username(&name).unwrap();
        url.set_password(Some("synthetic-history-policy-only"))
            .unwrap();
        let runtime = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .connect(url.as_str())
            .await
            .unwrap();
        let identity: Value = sqlx::query_scalar(
            "SELECT jsonb_build_object(
               'same_session_user', current_user = session_user,
               'superuser', rolsuper, 'createdb', rolcreatedb, 'createrole', rolcreaterole,
               'replication', rolreplication, 'bypassrls', rolbypassrls,
               'memberships', (SELECT count(*) FROM pg_auth_members WHERE member = r.oid),
               'database_owner', (SELECT datdba = r.oid FROM pg_database
                                  WHERE datname = current_database()))
             FROM pg_roles r WHERE rolname = current_user",
        )
        .fetch_one(&runtime)
        .await
        .unwrap();
        assert_eq!(
            identity,
            json!({"same_session_user": true, "superuser": false, "createdb": false,
                   "createrole": false, "replication": false, "bypassrls": false,
                   "memberships": 0, "database_owner": false})
        );
        evidence("fixture", "restricted_login", identity);
        let mut tables = PROTECTED.to_vec();
        tables.extend(["panda.charge_sessions", "panda.rivian_charge_payloads"]);
        tables.extend(UNRELATED);
        for table in tables {
            runtime
                .execute(sqlx::AssertSqlSafe(format!(
                    "CREATE TABLE {table} (id integer NOT NULL, time timestamptz NOT NULL,
                                         efficiency numeric NOT NULL);"
                )))
                .await
                .unwrap();
            if RETENTION.contains(&table) || UNRELATED.contains(&table) {
                sqlx::query("SELECT create_hypertable($1::regclass, 'time')")
                    .bind(table)
                    .execute(&runtime)
                    .await
                    .unwrap();
            }
            runtime
                .execute(sqlx::AssertSqlSafe(format!(
                    "INSERT INTO {table} VALUES (1, '2026-01-01T00:00:00Z', 2.73)"
                )))
                .await
                .unwrap();
        }
        if jobs {
            for table in RETENTION.into_iter().chain(UNRELATED) {
                sqlx::query(
                    "SELECT add_retention_policy($1::regclass, INTERVAL '90 days',
                                                 initial_start => '2099-01-01T00:00:00Z')",
                )
                .bind(table)
                .execute(&runtime)
                .await
                .unwrap();
            }
            for table in RETENTION {
                runtime
                    .execute(sqlx::AssertSqlSafe(format!(
                        "ALTER TABLE {table} SET (timescaledb.compress,
                         timescaledb.compress_segmentby='id', timescaledb.compress_orderby='time');
                         SELECT add_compression_policy('{table}', INTERVAL '7 days',
                                              initial_start => '2099-01-01T00:00:00Z');"
                    )))
                    .await
                    .unwrap();
            }
            runtime
                .execute(
                    "CREATE PROCEDURE history_policy_other.policy_retention(integer, jsonb)
                     LANGUAGE plpgsql AS $$ BEGIN NULL; END $$;
                     SELECT add_job('history_policy_other.policy_retention', INTERVAL '1 day',
                       config => '{\"hypertable_schema\":\"panda\",\"hypertable_name\":\"trips\"}',
                       initial_start => '2099-01-01T00:00:00Z');",
                )
                .await
                .unwrap();
        }
        Self {
            admin,
            bootstrap,
            runtime,
            name,
        }
    }

    async fn snapshot_on(&self, connection: &mut PgConnection) -> Snapshot {
        let relations: Vec<String> = sqlx::query_scalar(
            "SELECT n.nspname || '.' || c.relname FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname IN ('panda', 'timeseries', 'history_policy_other')
               AND c.relkind = 'r' ORDER BY 1",
        )
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        let mut tables = BTreeMap::new();
        for table in relations {
            let mut privileges = BTreeMap::new();
            for privilege in PRIVILEGES {
                let granted: bool =
                    sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, $2)")
                        .bind(&table)
                        .bind(privilege)
                        .fetch_one(&mut *connection)
                        .await
                        .unwrap();
                privileges.insert(privilege.to_owned(), granted);
            }
            let acl: String = sqlx::query_scalar(
                "SELECT coalesce(relacl, acldefault('r', relowner))::text FROM pg_class
                 WHERE oid = $1::regclass",
            )
            .bind(&table)
            .fetch_one(&mut *connection)
            .await
            .unwrap();
            let rows = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY id, time), '[]'::jsonb)
                 FROM {table} r"
            )))
            .fetch_one(&mut *connection)
            .await
            .unwrap();
            tables.insert(
                table,
                TableState {
                    privileges,
                    acl: acl
                        .replace(&format!("{}_inherited", self.name), "fixture_inherited")
                        .replace(&self.name, "fixture_runtime"),
                    rows,
                },
            );
        }
        let jobs = sqlx::query_scalar(
            "SELECT to_jsonb(j) - 'owner' - 'application_name'
             FROM timescaledb_information.jobs j
             WHERE owner::text = current_user ORDER BY job_id",
        )
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        Snapshot { tables, jobs }
    }

    async fn snapshot(&self, case: &str, phase: &str) -> Snapshot {
        let snapshot = self
            .snapshot_on(&mut self.runtime.acquire().await.unwrap())
            .await;
        evidence(case, phase, &snapshot);
        snapshot
    }

    async fn cleanup(self) {
        self.runtime.close().await;
        self.bootstrap.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH (FORCE)",
                self.name
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP ROLE {}, {}_inherited",
                self.name, self.name
            )))
            .await
            .unwrap();
        let remaining: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM pg_database WHERE datname = $1)
                  + (SELECT count(*) FROM pg_roles WHERE rolname IN ($1, $1 || '_inherited'))",
        )
        .bind(&self.name)
        .fetch_one(&self.admin)
        .await
        .unwrap();
        assert_eq!(remaining, 0);
        evidence("fixture", "cleanup_remaining", remaining);
        self.admin.close().await;
    }
}

fn target_job(job: &Value) -> bool {
    let relation = format!(
        "{}.{}",
        job["hypertable_schema"].as_str().unwrap_or(""),
        job["hypertable_name"].as_str().unwrap_or("")
    );
    job["proc_name"] == "policy_retention" && RETENTION.contains(&relation.as_str())
}

fn assert_policy_effects(before: &Snapshot, after: &Snapshot) {
    assert_eq!(
        before.tables.keys().collect::<Vec<_>>(),
        after.tables.keys().collect::<Vec<_>>()
    );
    for (table, original) in &before.tables {
        let actual = &after.tables[table];
        assert_eq!(actual.rows, original.rows, "{table} data");
        for privilege in PRIVILEGES {
            let denied = (PROTECTED.contains(&table.as_str())
                && ["DELETE", "TRUNCATE"].contains(&privilege))
                || (table == "panda.charge_sessions" && privilege == "TRUNCATE");
            assert_eq!(
                actual.privileges[privilege],
                !denied && original.privileges[privilege],
                "{table} {privilege}"
            );
        }
        if !PROTECTED.contains(&table.as_str()) && table != "panda.charge_sessions" {
            assert_eq!(actual.acl, original.acl, "{table} unrelated ACL");
        }
    }
    let mut expected_jobs = before.jobs.clone();
    for job in &mut expected_jobs {
        if target_job(job) {
            job["scheduled"] = json!(false);
        }
    }
    assert_eq!(
        after.jobs, expected_jobs,
        "only target retention scheduling may change"
    );
}

async fn denied(pool: &PgPool, case: &str, statement: String) {
    let error = pool
        .execute(sqlx::AssertSqlSafe(statement.clone()))
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501"),
        "{statement}"
    );
    evidence(
        case,
        "denied",
        json!({"statement": statement, "sqlstate": "42501"}),
    );
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_all_privileges_jobs_recording_and_idempotence() {
    let case = "all_privileges_jobs";
    let fixture = Fixture::new(true).await;
    let before = fixture.snapshot(case, "before").await;
    assert_eq!(before.tables.len(), 17);
    assert!(before
        .tables
        .values()
        .all(|table| table.privileges.values().all(|value| *value)));
    assert_eq!(before.jobs.iter().filter(|job| target_job(job)).count(), 4);
    assert_eq!(before.jobs.len(), 18);
    assert!(before.jobs.iter().all(|job| job["scheduled"] == true));
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    let after = fixture.snapshot(case, "after").await;
    assert_policy_effects(&before, &after);
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    assert_eq!(after, fixture.snapshot(case, "idempotent").await);
    for table in PROTECTED {
        for operation in ["DELETE FROM", "TRUNCATE"] {
            denied(&fixture.runtime, case, format!("{operation} {table}")).await;
        }
    }
    denied(
        &fixture.runtime,
        case,
        "TRUNCATE panda.charge_sessions".to_owned(),
    )
    .await;
    assert_eq!(after, fixture.snapshot(case, "denied_data_unchanged").await);
    for table in PROTECTED.into_iter().chain(["panda.charge_sessions"]) {
        fixture
            .runtime
            .execute(sqlx::AssertSqlSafe(format!(
                "INSERT INTO {table} VALUES (2, '2026-01-02T00:00:00Z', 3.01);
             UPDATE {table} SET efficiency = 2.91 WHERE id = 1;"
            )))
            .await
            .unwrap();
        let values: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT efficiency::text FROM {table} ORDER BY id"
        )))
        .fetch_all(&fixture.runtime)
        .await
        .unwrap();
        assert_eq!(values, ["2.91", "3.01"], "{table} recording/update");
    }
    let repaired = fixture
        .runtime
        .execute("DELETE FROM panda.charge_sessions WHERE id = 2")
        .await
        .unwrap();
    assert_eq!(repaired.rows_affected(), 1);
    let recorded = fixture.snapshot(case, "recorded_and_repaired").await;
    assert_eq!(
        recorded.tables["panda.charge_sessions"]
            .rows
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(recorded.jobs, after.jobs);
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_empty_jobs_and_development_noop() {
    let case = "empty_jobs_development";
    let fixture = Fixture::new(false).await;
    let before = fixture.snapshot(case, "before").await;
    assert!(before.jobs.is_empty());
    protect_history(&fixture.runtime, &config("development"))
        .await
        .unwrap();
    assert_eq!(before, fixture.snapshot(case, "development").await);
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    let after = fixture.snapshot(case, "production").await;
    assert_policy_effects(&before, &after);
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    assert_eq!(after, fixture.snapshot(case, "idempotent").await);
    fixture.cleanup().await;
}

async fn late_audit_failure(inherited: bool) {
    let case = if inherited {
        "inherited_late_audit"
    } else {
        "public_late_audit"
    };
    let fixture = Fixture::new(true).await;
    if inherited {
        fixture
            .bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "GRANT {}_inherited TO {};
             GRANT DELETE ON panda.battery_capacity_snapshots TO {}_inherited",
                fixture.name, fixture.name, fixture.name
            )))
            .await
            .unwrap();
    } else {
        fixture
            .bootstrap
            .execute("GRANT DELETE ON panda.battery_capacity_snapshots TO PUBLIC")
            .await
            .unwrap();
    }
    let before = fixture.snapshot(case, "before").await;
    let mut transaction = fixture.runtime.begin().await.unwrap();
    sqlx::raw_sql(HISTORY_POLICY)
        .execute(&mut *transaction)
        .await
        .unwrap();
    let uncommitted = fixture.snapshot_on(&mut transaction).await;
    evidence(case, "policy_uncommitted", &uncommitted);
    assert!(!uncommitted.tables["panda.vehicles"].privileges["DELETE"]);
    assert!(uncommitted.tables["panda.battery_capacity_snapshots"].privileges["DELETE"]);
    assert!(uncommitted
        .jobs
        .iter()
        .filter(|job| target_job(job))
        .all(|job| job["scheduled"] == false));
    transaction.rollback().await.unwrap();
    assert_eq!(before, fixture.snapshot(case, "probe_rolled_back").await);
    let error = protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "vehicle history protection requires a database role without overriding delete privileges"
    );
    evidence(case, "late_audit_error", error.to_string());
    assert_eq!(
        before,
        fixture.snapshot(case, "failed_install_rolled_back").await
    );
    if inherited {
        fixture
            .bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "REVOKE DELETE ON panda.battery_capacity_snapshots FROM {}_inherited;
             REVOKE {}_inherited FROM {}",
                fixture.name, fixture.name, fixture.name
            )))
            .await
            .unwrap();
    } else {
        fixture
            .bootstrap
            .execute("REVOKE DELETE ON panda.battery_capacity_snapshots FROM PUBLIC")
            .await
            .unwrap();
    }
    let retry_before = fixture.snapshot(case, "retry_before").await;
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    assert_policy_effects(&retry_before, &fixture.snapshot(case, "retry_after").await);
    denied(
        &fixture.runtime,
        case,
        "DELETE FROM panda.battery_capacity_snapshots".to_owned(),
    )
    .await;
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_public_delete_late_audit_rolls_back_every_effect() {
    late_audit_failure(false).await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_inherited_delete_late_audit_rolls_back_every_effect() {
    late_audit_failure(true).await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_missing_later_relation_rolls_back() {
    let case = "missing_later_relation";
    let fixture = Fixture::new(true).await;
    fixture
        .runtime
        .execute("ALTER TABLE panda.charge_sessions RENAME TO charge_sessions_missing")
        .await
        .unwrap();
    let before = fixture.snapshot(case, "before").await;
    let error = protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap_err();
    let sql_error = error.downcast_ref::<sqlx::Error>().unwrap();
    assert_eq!(
        sql_error.as_database_error().unwrap().code().as_deref(),
        Some("42P01")
    );
    evidence(
        case,
        "sql_error",
        json!({"sqlstate": "42P01", "error": format!("{error:#}")}),
    );
    assert_eq!(before, fixture.snapshot(case, "rolled_back").await);
    fixture
        .runtime
        .execute("ALTER TABLE panda.charge_sessions_missing RENAME TO charge_sessions")
        .await
        .unwrap();
    let retry_before = fixture.snapshot(case, "retry_before").await;
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    assert_policy_effects(&retry_before, &fixture.snapshot(case, "retry_after").await);
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_job_permission_sql_failure_rolls_back() {
    let case = "job_permission_failure";
    let fixture = Fixture::new(true).await;
    fixture
        .bootstrap
        .execute(
            "REVOKE EXECUTE ON FUNCTION alter_job(integer, interval, interval, integer, interval,
         boolean, jsonb, timestamptz, boolean, regproc, boolean, timestamptz, text, text)
         FROM PUBLIC",
        )
        .await
        .unwrap();
    let before = fixture.snapshot(case, "before").await;
    let error = protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap_err();
    let sql_error = error.downcast_ref::<sqlx::Error>().unwrap();
    assert_eq!(
        sql_error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    evidence(
        case,
        "sql_error",
        json!({"sqlstate": "42501", "error": format!("{error:#}")}),
    );
    assert_eq!(before, fixture.snapshot(case, "rolled_back").await);
    fixture
        .bootstrap
        .execute(
            "GRANT EXECUTE ON FUNCTION alter_job(integer, interval, interval, integer, interval,
         boolean, jsonb, timestamptz, boolean, regproc, boolean, timestamptz, text, text)
         TO PUBLIC",
        )
        .await
        .unwrap();
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    assert_policy_effects(&before, &fixture.snapshot(case, "retry_after").await);
    fixture.cleanup().await;
}

async fn blocked_policy(cancel_backend: bool) {
    let case = if cancel_backend {
        "server_cancel"
    } else {
        "dropped_future"
    };
    let fixture = Fixture::new(true).await;
    let before = fixture.snapshot(case, "before").await;
    let backend: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&fixture.runtime)
        .await
        .unwrap();
    let mut blocker = fixture.bootstrap.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query(
        "SELECT id FROM _timescaledb_config.bgw_job WHERE id =
         (SELECT max(job_id) FROM timescaledb_information.jobs
          WHERE proc_name = 'policy_retention' AND hypertable_schema = 'panda'
            AND hypertable_name = 'rivian_charge_payloads') FOR UPDATE",
    )
    .fetch_one(&mut *blocker)
    .await
    .unwrap();
    let runtime = fixture.runtime.clone();
    let worker =
        tokio::spawn(async move { protect_history(&runtime, &config("production")).await });
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT $2 = ANY(pg_blocking_pids($1))")
                .bind(backend)
                .bind(blocker_pid)
                .fetch_one(&fixture.bootstrap)
                .await
                .unwrap();
            if waiting {
                break;
            }
            assert!(
                !worker.is_finished(),
                "policy must block on the real job catalog row"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("policy reached locked target job");
    evidence(case, "observed_job_lock_wait", true);
    let mut observer = fixture.bootstrap.acquire().await.unwrap();
    observer
        .execute(sqlx::AssertSqlSafe(format!("SET ROLE {}", fixture.name)))
        .await
        .unwrap();
    let during = fixture.snapshot_on(&mut observer).await;
    evidence(case, "while_blocked", &during);
    assert_eq!(
        before, during,
        "uncommitted ACL/job changes must be invisible"
    );
    observer.execute("RESET ROLE").await.unwrap();
    drop(observer);
    if cancel_backend {
        let cancelled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
            .bind(backend)
            .fetch_one(&fixture.bootstrap)
            .await
            .unwrap();
        assert!(cancelled);
        let error = tokio::time::timeout(Duration::from_secs(15), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        let sql_error = error.downcast_ref::<sqlx::Error>().unwrap();
        assert_eq!(
            sql_error.as_database_error().unwrap().code().as_deref(),
            Some("57014")
        );
        evidence(
            case,
            "cancel_error",
            json!({"sqlstate": "57014", "error": format!("{error:#}")}),
        );
    } else {
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        evidence(case, "future_cancelled", true);
    }
    blocker.rollback().await.unwrap();
    assert_eq!(before, fixture.snapshot(case, "rolled_back").await);
    let idle: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT FROM pg_stat_activity
         WHERE usename = $1 AND xact_start IS NOT NULL)",
    )
    .bind(&fixture.name)
    .fetch_one(&fixture.bootstrap)
    .await
    .unwrap();
    assert!(
        idle,
        "no runtime transaction may remain open after cancellation"
    );
    evidence(case, "no_open_runtime_transaction", idle);
    protect_history(&fixture.runtime, &config("production"))
        .await
        .unwrap();
    assert_policy_effects(&before, &fixture.snapshot(case, "retry_after").await);
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_server_cancelled_job_transaction_rolls_back() {
    blocked_policy(true).await;
}

#[tokio::test]
#[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
async fn history_policy_differential_dropped_blocked_future_rolls_back() {
    blocked_policy(false).await;
}

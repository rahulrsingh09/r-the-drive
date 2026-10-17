use chrono::DateTime;
use serde_json::{json, Value};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

pub fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub struct Fixture {
    pub pool: PgPool,
    pub url: String,
    admin: PgPool,
    database: String,
    role: String,
}
impl Fixture {
    pub async fn new() -> Self {
        let mut url = url::Url::parse(
            &std::env::var("MAINTENANCE_DATABASE_URL")
                .expect("explicit disposable fixture required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "maintenance_bootstrap");
        assert_eq!(url.path(), "/maintenance_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("maintenance_role_{suffix}");
        let database = format!("maintenance_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!("CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_maintenance_app'"))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap.execute(sqlx::AssertSqlSafe(format!("CREATE SCHEMA panda AUTHORIZATION {role}; REVOKE CREATE ON SCHEMA public FROM PUBLIC; REVOKE ALL ON SCHEMA panda FROM PUBLIC; CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda; CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"))).await.unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_maintenance_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
        panda_api::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = panda_api::db::pool::application_pool_options()
            .max_connections(5)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({"database_url":url.as_str(),"redis_url":"redis://unused","panda_env":"production"})).unwrap();
        panda_api::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        Self {
            pool,
            url: url.into(),
            admin,
            database,
            role,
        }
    }
    pub async fn seed(&self) {
        for n in 1..=5 {
            sqlx::query("INSERT INTO panda.users(id,email,password_hash,created_at,updated_at) VALUES($1,$2,'unused','2025-01-01Z','2025-01-01Z')").bind(id(n)).bind(format!("synthetic-{n}@example.invalid")).execute(&self.pool).await.unwrap();
        }
        for (n, user, model) in [(10, 1, "R1T"), (11, 2, "R2"), (12, 3, "R1S")] {
            sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,battery_capacity_wh,created_at,updated_at) VALUES($1,$2,$3,$4,100000,'2025-01-01Z','2025-01-01Z')").bind(id(n)).bind(id(user)).bind(format!("synthetic-{n}")).bind(model).execute(&self.pool).await.unwrap();
        }
        for (n, vehicle, user, role) in [
            (20, 10, 1, "owner"),
            (21, 10, 2, "manager"),
            (22, 10, 3, "viewer"),
            (23, 11, 4, "owner"),
        ] {
            sqlx::query("INSERT INTO panda.vehicle_memberships(id,vehicle_id,user_id,role,created_at,updated_at) VALUES($1,$2,$3,$4,'2025-01-01Z','2025-01-01Z')").bind(id(n)).bind(id(vehicle)).bind(id(user)).bind(role).execute(&self.pool).await.unwrap();
        }
    }
    pub async fn sql(&self, sql: &str) {
        self.pool.execute(sqlx::AssertSqlSafe(sql)).await.unwrap();
    }
    pub async fn state(&self) -> Value {
        let mut state = serde_json::Map::new();
        for (table, order) in [
            ("panda.users", "id"),
            ("panda.vehicles", "id"),
            ("panda.vehicle_memberships", "id"),
            ("panda.trips", "vehicle_id,started_at,id"),
            ("panda.trip_user_annotations", "trip_id,user_id"),
            ("panda.charge_sessions", "vehicle_id,started_at,id"),
            (
                "panda.charge_session_user_annotations",
                "charge_session_id,user_id",
            ),
            ("panda.vehicle_state_periods", "vehicle_id,started_at,id"),
            ("timeseries.telemetry", "vehicle_id,ts"),
            ("panda.rivian_charge_payloads", "vehicle_id,captured_at,id"),
            (
                "panda.charge_session_external_aliases",
                "charge_session_id,external_id",
            ),
            ("panda.rivian_charge_payload_identities", "identity_key"),
            ("panda.active_charge_checkpoints", "vehicle_id"),
            ("panda.pending_charge_completions", "vehicle_id,session_id"),
        ] {
            let query=format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY {order}), '[]'::jsonb) FROM {table} t");
            state.insert(
                table.into(),
                sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(query))
                    .fetch_one(&self.pool)
                    .await
                    .unwrap(),
            );
        }
        Value::Object(state)
    }
    pub async fn process(&self, binary: &str, args: &[&str]) -> Value {
        let target = std::env::var("CARGO_TARGET_DIR").unwrap();
        let out = tokio::process::Command::new(format!("{target}/debug/{binary}"))
            .args(args)
            .env("DATABASE_URL", &self.url)
            .env("NO_COLOR", "1")
            .env_remove("RUST_LOG")
            .output()
            .await
            .unwrap();
        json!({"exit":out.status.code(),"stdout":String::from_utf8(out.stdout).unwrap(),"stderr":String::from_utf8(out.stderr).unwrap()})
    }
    pub async fn cleanup(self) {
        self.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER DATABASE {} ALLOW_CONNECTIONS false",
                self.database
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH(FORCE)",
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

pub fn freeze(name: &str, actual: Value, expected: &[u8]) {
    let directory = std::env::var_os("MAINTENANCE_ARTIFACT_DIR").map(std::path::PathBuf::from);
    if let Some(directory) = &directory {
        std::fs::write(
            directory.join(format!("{name}-raw.json")),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }
    let mut actual = actual;
    let mut ids = std::collections::BTreeMap::new();
    collect_generated_ids(&actual, &mut ids);
    normalize(&mut actual, &mut ids);
    if let Some(directory) = &directory {
        std::fs::write(
            directory.join(format!("{name}-actual.json")),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }
    let expected: Value = serde_json::from_reader(flate2::read::GzDecoder::new(expected)).unwrap();
    assert!(actual == expected, "frozen {name} contract differs");
}
fn collect_generated_ids(value: &Value, ids: &mut std::collections::BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            for table in ["panda.charge_sessions", "panda.trips"] {
                if let Some(Value::Array(rows)) = map.get(table) {
                    for row in rows {
                        if let Some(Value::String(id)) = row.get("id") {
                            if Uuid::parse_str(id).is_ok_and(|id| id.as_u128() > 1000000) {
                                let next = format!("<generated-id-{}>", ids.len() + 1);
                                ids.entry(id.clone()).or_insert(next);
                            }
                        }
                    }
                }
            }
            for child in map.values() {
                collect_generated_ids(child, ids);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_generated_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn normalize(value: &mut Value, ids: &mut std::collections::BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                normalize(v, ids);
            }
            for table in [
                "panda.charge_session_user_annotations",
                "panda.trip_user_annotations",
                "panda.charge_session_external_aliases",
            ] {
                if let Some(Value::Array(rows)) = map.get_mut(table) {
                    rows.sort_by_cached_key(|row| row.to_string());
                }
            }
        }
        Value::Array(array) => {
            for v in array {
                normalize(v, ids)
            }
        }
        Value::String(text) => {
            if let Ok(uuid) = Uuid::parse_str(text) {
                if uuid.as_u128() > 1000000 {
                    let next = format!("<generated-id-{}>", ids.len() + 1);
                    *text = ids.entry(text.clone()).or_insert(next).clone();
                }
            } else if text.starts_with("2026-")
                && (text.contains('T') || text.contains(' '))
                && DateTime::parse_from_rfc3339(text).is_ok()
            {
                *text = "<clock>".into();
            }
            if text.contains(" INFO ") || text.contains(" WARN ") {
                *text = text
                    .lines()
                    .map(|line| {
                        if line.starts_with("2026-") {
                            line.split_once(' ')
                                .map(|(_, tail)| format!("<clock> {tail}\n"))
                                .unwrap_or_else(|| format!("{line}\n"))
                        } else {
                            format!("{line}\n")
                        }
                    })
                    .collect();
            }
        }
        _ => {}
    }
}

#[allow(dead_code)]
pub fn full_replacement_normalized(mut value: Value) -> Value {
    let mut ids = std::collections::BTreeMap::new();
    collect_generated_ids(&value, &mut ids);
    normalize(&mut value, &mut ids);
    value
}

use super::super::*;
use axum::{body::to_bytes, http::Request, Extension};
use serde_json::{json, Value};
use sqlx::{Executor, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

pub(crate) struct Fixture {
    pub state: AppState,
    admin: PgPool,
    database: String,
    role: String,
}

impl Fixture {
    pub async fn new() -> Self {
        let mut url = Url::parse(&std::env::var("EXTERNAL_CONTRACT_DATABASE_URL").unwrap())
            .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "external_fixture");
        assert_eq!(url.path(), "/external_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("external_role_{suffix}");
        let database = format!("external_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_external_app'"
        ))).await.unwrap();
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {database} OWNER {role}"
        ))).await.unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap.execute(sqlx::AssertSqlSafe(format!(
            "CREATE SCHEMA panda AUTHORIZATION {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SCHEMA panda FROM PUBLIC;
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
        ))).await.unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_external_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool).await.unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(8).connect(url.as_str()).await.unwrap();
        let redis_url = std::env::var("REDIS_URL").unwrap();
        assert_eq!(redis_url, "redis://redis:6379/");
        let config = serde_json::from_value(json!({
            "database_url":url.as_str(), "redis_url":redis_url, "panda_env":"production"
        })).unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await.unwrap();
        let generated = crate::keys::generate_keys().unwrap();
        let jwt_keys = std::sync::Arc::new(
            crate::middleware::auth::JwtKeys::new(
                &generated.jwt_private_pem, &generated.jwt_public_pem
            ).unwrap()
        );
        Self {
            state: AppState {
                pool, redis: redis::Client::open(redis_url).unwrap(), jwt_keys,
                age_key: generated.age_key, config, resources: Default::default(),
                nominatim_cache: Default::default(),
                supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            },
            admin, database, role,
        }
    }

    pub async fn user(&self, role: &str) -> Uuid {
        let user = Uuid::new_v4();
        self.insert_user(user, role).await;
        user
    }

    pub async fn insert_user(&self, user: Uuid, role: &str) {
        sqlx::query("INSERT INTO panda.users (id,email,role,created_at,updated_at) VALUES ($1,$2,$3,'2026-01-01Z','2026-01-01Z')")
            .bind(user).bind(format!("{user}@example.invalid")).bind(role)
            .execute(&self.state.pool).await.unwrap();
    }

    pub fn auth(&self, user_id: Uuid) -> AuthUser {
        AuthUser { user_id, default_vehicle_id: None, api_access_level: None, api_vehicle_id: None }
    }

    pub fn router(&self) -> Router {
        Router::new().nest("/v1", router())
            .layer(Extension(self.state.jwt_keys.decoding.clone()))
            .with_state(self.state.clone())
    }

    pub async fn request(&self, user: Option<Uuid>, method: &str, path: &str, body: &str) -> Value {
        let mut builder = Request::builder().method(method).uri(path)
            .header("content-type", "application/json");
        if let Some(user) = user {
            let token = crate::middleware::auth::issue_access_token(user, None, &self.state.jwt_keys).unwrap();
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = self.router().oneshot(builder.body(Body::from(body.to_owned())).unwrap())
            .await.unwrap();
        let status = response.status().as_u16();
        let headers: std::collections::BTreeMap<_, _> = response.headers().iter()
            .map(|(k,v)| (k.as_str().to_owned(), v.to_str().unwrap().to_owned())).collect();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024).await.unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let body: Value = serde_json::from_str(&text).unwrap_or(json!(text));
        json!({"status":status,"headers":headers,"body":body})
    }

    pub async fn rows(&self) -> Value {
        let mut result = serde_json::Map::new();
        for (table, order) in [
            ("external_connection_settings","id"), ("external_connection_activity","connection_id"),
            ("users","id"), ("vehicles","id"), ("vehicle_credentials","vehicle_id"),
            ("vehicle_runtime_state","vehicle_id"), ("vehicle_memberships","id"),
            ("backup_settings","id"), ("addresses","id"), ("geofences","id"),
        ] {
            let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY {order}"
            ))).fetch_all(&self.state.pool).await.unwrap();
            result.insert(table.into(), json!(rows));
        }
        Value::Object(result)
    }

    pub async fn execute(&self, sql: &'static str) {
        sqlx::raw_sql(sql).execute(&self.state.pool).await.unwrap();
    }

    pub async fn cleanup(self) {
        self.state.pool.close().await;
        self.admin.execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {} WITH (FORCE)", self.database
        ))).await.unwrap();
        self.admin.execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await.unwrap();
        self.admin.close().await;
    }
}

pub(super) fn id(n: u128) -> Uuid { Uuid::from_u128(n) }

pub(super) fn freeze(name: &str, raw: &Value, expected: &str) {
    let dir = std::env::var("EXTERNAL_CONTRACT_ARTIFACT_DIR").unwrap();
    std::fs::write(format!("{dir}/{name}-raw.json"), serde_json::to_vec_pretty(raw).unwrap()).unwrap();
    let mut normalized = raw.clone();
    normalize_clocks(&mut normalized);
    std::fs::write(format!("{dir}/{name}.json"), serde_json::to_vec_pretty(&normalized).unwrap()).unwrap();
    let expected: Value = serde_json::from_str(expected).unwrap();
    assert!(normalized == expected, "frozen {name} contract differs; full outputs saved outside the repository");
}

fn normalize_clocks(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if ["updated_at", "created_at", "last_attempt_at", "last_success_at", "last_test_at",
                    "tested_at", "usage_date", "revision"].contains(&name.as_str())
                    && value.is_string()
                    && !value.as_str().unwrap().starts_with("2026-01-")
                {
                    *value = json!("<clock>");
                } else { normalize_clocks(value); }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(normalize_clocks),
        _ => {}
    }
}

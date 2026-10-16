use crate::middleware::auth::{issue_access_token, AppState, JwtKeys};
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

pub(super) fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub(super) struct Fixture {
    admin: PgPool,
    pub state: AppState,
    database: String,
    role: String,
    ids: std::collections::HashMap<String, String>,
}
impl Fixture {
    pub async fn new() -> Self {
        let configured = std::env::var("TRIP_CONTRACT_TEST_DATABASE_URL")
            .expect("explicit disposable TRIP_CONTRACT_TEST_DATABASE_URL is required");
        let mut url = url::Url::parse(&configured).unwrap();
        assert_eq!(
            url.host_str(),
            Some("db"),
            "use the isolated fixture Docker network"
        );
        assert_eq!(url.path(), "/trip_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let database = format!("trip_contract_{suffix}");
        let role = format!("trip_role_{suffix}");
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {role} LOGIN PASSWORD 'synthetic_trip_role_only'
             NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"
            )))
            .await
            .unwrap();
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
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_trip_role_only")).unwrap();
        let migration_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({
            "database_url": url.as_str(), "redis_url": "redis://synthetic.invalid",
            "panda_env": "production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        let protected: bool = sqlx::query_scalar(
            "SELECT NOT rolsuper AND NOT rolbypassrls
             AND NOT has_table_privilege(current_user,'panda.trips','DELETE')
             AND NOT has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE')
             FROM pg_roles WHERE rolname=current_user",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(protected);
        let keys = crate::keys::generate_keys().unwrap();
        let state = AppState {
            pool,
            redis: redis::Client::open("redis://redis:6379/0").unwrap(),
            jwt_keys: std::sync::Arc::new(
                JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).unwrap(),
            ),
            age_key: "synthetic-local-only-unused".into(),
            config,
            resources: Default::default(),
            nominatim_cache: Default::default(),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
        };
        let mut conn = state
            .redis
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
        Self {
            admin,
            state,
            database,
            role,
            ids: std::collections::HashMap::new(),
        }
    }

    pub async fn seed(&self) {
        sqlx::raw_sql(include_str!("seed.sql"))
            .execute(&self.state.pool)
            .await
            .unwrap();
    }
    pub fn register(&mut self, actual: &Value, label: &str) -> Uuid {
        let value = actual["body"]["id"].as_str().expect("created tag id");
        self.ids.insert(value.into(), label.into());
        value.parse().unwrap()
    }
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        user: Option<u128>,
        body: Value,
    ) -> Value {
        self.raw(
            method,
            path,
            user,
            &serde_json::to_string(&body).unwrap(),
            None,
        )
        .await
    }
    pub async fn raw(
        &self,
        method: &str,
        path: &str,
        user: Option<u128>,
        body: &str,
        key: Option<&str>,
    ) -> Value {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(user) = user {
            let token = issue_access_token(id(user), None, &self.state.jwt_keys).unwrap();
            req = req.header("authorization", format!("Bearer {token}"));
        } else if let Some(key) = key {
            req = req.header("authorization", format!("Bearer {key}"));
        }
        let response = crate::routes::build_router(self.state.clone())
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_str().unwrap().into())))
            .collect::<serde_json::Map<_, _>>();
        let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        if let Some(length) = headers.get("content-length").and_then(Value::as_str) {
            assert_eq!(length.parse::<usize>().unwrap(), bytes.len());
        }
        let body = serde_json::from_slice::<Value>(&bytes)
            .unwrap_or_else(|_| json!(String::from_utf8(bytes.to_vec()).unwrap()));
        json!({"status":status,"headers":headers,"body":body})
    }
    pub async fn snapshot(&self) -> Value {
        let mut result = serde_json::Map::new();
        for table in [
            "panda.trips",
            "panda.trip_tags",
            "panda.trip_tag_assignments",
            "panda.trip_user_annotations",
            "panda.trip_weather_samples",
            "timeseries.telemetry",
            "panda.geofences",
            "panda.addresses",
            "panda.vehicle_memberships",
        ] {
            let sql = format!("SELECT COALESCE(jsonb_agg(j ORDER BY j::text), '[]'::jsonb) FROM (SELECT to_jsonb(t) AS j FROM {table} t) s");
            let value: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(&self.state.pool)
                .await
                .unwrap();
            result.insert(table.into(), value);
        }
        Value::Object(result)
    }
    pub fn normalize(&self, v: &mut Value) {
        match v {
            Value::Object(map) => {
                for (k, v) in map {
                    if [
                        "x-request-id",
                        "x-ratelimit-after",
                        "x-panda-ratelimit-reset",
                        "ratelimit-reset",
                        "x-ratelimit-reset",
                        "content-length",
                    ]
                    .contains(&k.as_str())
                    {
                        *v = json!("<transport-generated>");
                    } else if [
                        "created_at",
                        "updated_at",
                        "assigned_at",
                        "matched_at",
                        "fetched_at",
                    ]
                    .contains(&k.as_str())
                        && v.is_string()
                        && !v.as_str().unwrap().starts_with("2026-01-01T00:00:00")
                    {
                        *v = json!("<database-clock>");
                    } else {
                        self.normalize(v);
                    }
                }
            }
            Value::Array(items) => {
                for v in items {
                    self.normalize(v);
                }
            }
            Value::String(s) => {
                if let Some(label) = self.ids.get(s) {
                    *s = label.clone();
                }
            }
            _ => (),
        }
    }
    pub async fn cleanup(self) {
        self.state.pool.close().await;
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

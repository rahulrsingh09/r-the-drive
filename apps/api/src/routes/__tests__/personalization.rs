use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use std::{collections::BTreeMap, sync::Arc};
use tower::ServiceExt;

#[path = "personalization/dashboard_chart.rs"]
mod dashboard_chart;
#[path = "personalization/theme_settings.rs"]
mod theme_settings;

struct Fixture {
    admin: PgPool,
    pool: PgPool,
    database: String,
    role: String,
    url: String,
}

impl Fixture {
    async fn new() -> Self {
        let configured = std::env::var("PERSONALIZATION_TEST_DATABASE_URL")
            .expect("explicit disposable PERSONALIZATION_TEST_DATABASE_URL is required");
        let mut url = url::Url::parse(&configured).unwrap();
        assert_eq!(
            url.host_str(),
            Some("db"),
            "use the isolated fixture Docker network"
        );
        assert_eq!(url.path(), "/personalization_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let database = format!("personalization_contract_{suffix}");
        let role = format!("personalization_role_{suffix}");
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {role} LOGIN PASSWORD 'synthetic_personalization_role_only'
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
        url.set_password(Some("synthetic_personalization_role_only"))
            .unwrap();
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
        assert!(
            sqlx::query("DELETE FROM panda.trips WHERE id='00000000-0000-0000-0000-000000000000'")
                .execute(&pool)
                .await
                .is_err(),
            "production history protection must deny direct deletion"
        );
        Self {
            admin,
            pool,
            database,
            role,
            url: url.to_string(),
        }
    }

    async fn cleanup(self) {
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

struct Harness {
    fixture: Fixture,
    app: Router,
    tokens: Vec<String>,
    observations: Vec<Value>,
    aliases: BTreeMap<String, String>,
}

impl Harness {
    async fn new() -> Self {
        let fixture = Fixture::new().await;
        for (index, role) in [
            (101, "super_user"),
            (102, "admin"),
            (103, "user"),
            (104, "user"),
        ] {
            sqlx::query("INSERT INTO panda.users(id,email,password_hash,role) VALUES($1,$2,'synthetic-not-a-real-password-hash',$3)")
                .bind(Uuid::from_u128(index)).bind(format!("synthetic-{index}@example.invalid")).bind(role).execute(&fixture.pool).await.unwrap();
        }
        let keys = crate::keys::generate_keys().unwrap();
        let jwt_keys = Arc::new(
            crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem)
                .unwrap(),
        );
        let tokens = (101..105)
            .map(|id| {
                crate::middleware::auth::issue_access_token(Uuid::from_u128(id), None, &jwt_keys)
                    .unwrap()
            })
            .collect();
        let mut config: crate::config::Config = serde_json::from_value(json!({
            "database_url":"postgres://synthetic.invalid/local_only", "redis_url":"redis://127.0.0.1:1/",
            "panda_env":"test", "cookie_insecure":true
        })).unwrap();
        config.rate_limit.auth_read_burst = 10000;
        config.rate_limit.auth_write_burst = 10000;
        config.rate_limit.auth_metadata_burst = 10000;
        let state = AppState {
            pool: fixture.pool.clone(),
            redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
            jwt_keys,
            age_key: keys.age_key,
            config,
            nominatim_cache: Default::default(),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            resources: Default::default(),
        };
        Self {
            fixture,
            app: crate::routes::build_router(state),
            tokens,
            observations: vec![],
            aliases: BTreeMap::new(),
        }
    }

    async fn state(&self) -> Value {
        let mut state = serde_json::Map::new();
        for (table, order) in [
            ("dashboards", "slug,owner_id NULLS FIRST,id"),
            ("charts", "slug,owner_id NULLS FIRST,id"),
            ("user_themes", "name,id"),
            ("user_theme_revisions", "theme_id,revision"),
            ("user_theme_publications", "theme_id,revision"),
            ("user_preferences", "user_id"),
            ("system_config", "key"),
            ("backup_settings", "id"),
            ("update_check_settings", "id"),
            ("security_events", "created_at,id"),
        ] {
            let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY {order}"
            )))
            .fetch_all(&self.fixture.pool)
            .await
            .unwrap();
            state.insert(table.into(), json!(rows));
        }
        Value::Object(state)
    }

    fn label(&mut self, id: &str, label: &str) {
        self.aliases.insert(id.into(), label.into());
    }

    async fn request(
        &self,
        actor: Option<usize>,
        method: &str,
        path: &str,
        raw: &str,
        headers: &[(&str, String)],
    ) -> Value {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("x-request-id", "synthetic-personalization-request")
            .header("content-type", "application/json");
        if let Some(actor) = actor {
            request = request.header("authorization", format!("Bearer {}", self.tokens[actor]));
        }
        for (key, value) in headers {
            request = request.header(*key, value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(raw.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let header_values: BTreeMap<String, Vec<String>> = response
            .headers()
            .keys()
            .map(|key| {
                (
                    key.to_string(),
                    response
                        .headers()
                        .get_all(key)
                        .iter()
                        .map(|value| value.to_str().unwrap().to_owned())
                        .collect(),
                )
            })
            .collect();
        let headers: BTreeMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_owned()))
            .collect();
        let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        if let Some(length) = headers.get("content-length") {
            assert_eq!(
                length.parse::<usize>().unwrap(),
                bytes.len(),
                "declared body length"
            );
        }
        let body = serde_json::from_slice::<Value>(&bytes)
            .unwrap_or_else(|_| json!(String::from_utf8(bytes.to_vec()).unwrap()));
        json!({"status":status, "headers":headers, "headerValues":header_values, "body":body})
    }

    async fn step(
        &mut self,
        name: &str,
        actor: Option<usize>,
        method: &str,
        path: &str,
        body: Value,
        headers: &[(&str, String)],
        expected: u16,
    ) -> Value {
        self.raw_step(
            name,
            actor,
            method,
            path,
            &body.to_string(),
            headers,
            expected,
        )
        .await
    }

    async fn raw_step(
        &mut self,
        name: &str,
        actor: Option<usize>,
        method: &str,
        path: &str,
        raw: &str,
        headers: &[(&str, String)],
        expected: u16,
    ) -> Value {
        let before = self.state().await;
        let response = self.request(actor, method, path, raw, headers).await;
        assert_eq!(response["status"], expected, "{name}: {response}");
        let after = self.state().await;
        for (table, identity) in [
            ("dashboards", "id"),
            ("charts", "id"),
            ("user_themes", "id"),
            ("user_preferences", "user_id"),
        ] {
            for previous in before[table].as_array().unwrap() {
                if let Some(current) = after[table]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|current| current[identity] == previous[identity])
                {
                    assert_eq!(
                        previous.get("created_at"),
                        current.get("created_at"),
                        "{name}: existing creation time changed"
                    );
                }
            }
        }
        if expected >= 400 || method == "GET" {
            assert_eq!(
                before, after,
                "{name}: rejected/read request changed persistence"
            );
        }
        self.observations
            .push(json!({"name":name,"response":response,"persisted":after}));
        response
    }

    async fn checkpoint(&mut self, name: &str) {
        self.observations
            .push(json!({"name":name,"persisted":self.state().await}));
    }

    fn normalize(&self, value: &mut Value, key: &str) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    self.normalize(value, key);
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.normalize(value, key);
                }
            }
            Value::String(text) => {
                if [
                    "created_at",
                    "updated_at",
                    "retired_at",
                    "published_at",
                    "createdAt",
                    "updatedAt",
                    "retiredAt",
                    "publishedAt",
                ]
                .contains(&key)
                {
                    assert!(
                        chrono::DateTime::parse_from_rfc3339(text).is_ok(),
                        "invalid timestamp {text}"
                    );
                    *text = "<database-time>".into();
                } else if key == "content-length" {
                    *text = "<body-byte-length>".into();
                } else if key.starts_with("x-ratelimit-") || key == "x-panda-ratelimit-reset" {
                    *text = "<rate-counter>".into();
                } else {
                    for (id, label) in &self.aliases {
                        *text = text.replace(id, label);
                    }
                }
            }
            _ => {}
        }
    }

    async fn finish(mut self) {
        let state = self.state().await;
        for (i, row) in state["security_events"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            if let Some(id) = row["id"].as_str() {
                self.label(id, &format!("audit-{i}"));
            }
        }
        let raw = json!(self.observations);
        let mut normalized = raw.clone();
        self.normalize(&mut normalized, "");
        if let Ok(directory) = std::env::var("PERSONALIZATION_CAPTURE") {
            std::fs::write(
                format!("{directory}/expected.json"),
                serde_json::to_string_pretty(&normalized).unwrap(),
            )
            .unwrap();
            std::fs::write(
                format!("{directory}/raw.json"),
                serde_json::to_string_pretty(&raw).unwrap(),
            )
            .unwrap();
        } else {
            let expected: Value =
                serde_json::from_str(include_str!("personalization/expected.json")).unwrap();
            if expected != normalized {
                std::fs::write(
                    "/review/actual.json",
                    serde_json::to_string_pretty(&normalized).unwrap(),
                )
                .unwrap();
                for (i, (a, b)) in normalized
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(expected.as_array().unwrap())
                    .enumerate()
                {
                    assert_eq!(a, b, "observation {i}");
                }
                assert_eq!(
                    normalized.as_array().unwrap().len(),
                    expected.as_array().unwrap().len()
                );
            }
        }
        self.fixture.cleanup().await;
    }
}

#[tokio::test]
#[ignore = "requires the unique disposable TimescaleDB fixture"]
async fn full_http_and_persistence_contract() {
    let mut h = Harness::new().await;
    dashboard_chart::exercise(&mut h).await;
    theme_settings::exercise(&mut h).await;
    h.finish().await;
}

#[test]
fn managed_widget_boundary_contract() {
    let mut cases = Vec::new();
    let canonical = json!({"id":"managed-id","managedKey":"example.chart","managed":true,"componentType":"chart","definitionId":"catalog","layout":{"x":0,"y":2,"w":12,"h":7},"options":{"page":"example","showPicker":false,"chartId":"example"}});
    for widgets in [
        Value::Null,
        json!([]),
        json!([null, 7]),
        json!([{"id":"other","layout":{"y":40,"h":9}}]),
        json!([{"id":"managed-id","options":{"showPicker":true,"custom":7},"layout":{"x":3,"y":90,"w":6,"h":3}}]),
        json!([{"managedKey":"example.chart","options":null}]),
        json!([{"id":"managed-id"},{"managedKey":"example.chart"}]),
    ] {
        for bundled in [
            canonical.clone(),
            json!({"id":"managed-id"}),
            json!({"managedKey":"example.chart","options":null}),
        ] {
            for retired in [false, true] {
                let config = json!({"widgets":widgets,"unrelated":{"saved":true}});
                let retirement = if retired {
                    vec![("example.chart", "managed-id")]
                } else {
                    vec![]
                };
                let result = merge_managed_widgets(config.clone(), &[&bundled], &retirement);
                cases.push(
                    json!({"config":config,"bundled":bundled,"retired":retired,"result":result}),
                );
            }
        }
    }
    for x in [0, 1] {
        for y in [8, 9, 19, 40] {
            for width in [11, 12] {
                for height in [10, 14] {
                    let config = json!({"widgets":[{"managedKey":"trips.tire-pressure-timeline"},{"id":"d5000005-0000-0000-0000-000000000005","layout":{"x":x,"y":y,"w":width,"h":height}},{"id":"d5000005-0000-0000-0000-000000000006","layout":{"x":x,"y":y,"w":width,"h":height}}]});
                    let result = merge_managed_widgets(
                        config.clone(),
                        &[],
                        retired_managed_widgets("trips"),
                    );
                    cases.push(json!({"config":config,"result":result}));
                }
            }
        }
    }
    let current =
        json!({"widgets":[{"id":"one","managedKey":"old"},{"id":"two","managedKey":"old"}]});
    let templates = [
        json!({"id":"one","managedKey":"new"}),
        json!({"managedKey":"old"}),
    ];
    cases.push(json!({"config":current,"bundled":templates,"result":merge_managed_widgets(current.clone(),&templates.iter().collect::<Vec<_>>(),&[])}));
    let actual = json!(cases);
    if let Ok(dir) = std::env::var("PERSONALIZATION_CAPTURE") {
        std::fs::write(
            format!("{dir}/managed-expected.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    } else {
        assert_eq!(
            actual,
            serde_json::from_str::<Value>(include_str!("personalization/managed-expected.json"))
                .unwrap()
        );
    }
}

#[test]
#[ignore = "runs the unchanged original admin regression on its own restricted database"]
fn original_admin_regression_on_disposable_db() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = runtime.block_on(Fixture::new());
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "routes::dashboards::tests::require_admin_returns_403_not_500_for_non_admin",
            "--ignored",
            "--nocapture",
        ])
        .env("DATABASE_URL", &fixture.url)
        .env("REDIS_URL", "redis://127.0.0.1:1/")
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&result.stdout));
    println!("{}", String::from_utf8_lossy(&result.stderr));
    runtime.block_on(fixture.cleanup());
    assert!(
        result.status.success(),
        "unchanged original admin regression failed"
    );
}

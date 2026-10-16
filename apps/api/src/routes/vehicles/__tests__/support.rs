use super::*;
use axum::{body::to_bytes, http::Request, Extension};
use sqlx::{Executor, PgPool};
use tower::ServiceExt;

pub(super) fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub(super) struct Fixture {
    pub state: AppState,
    admin: PgPool,
    database: String,
    role: String,
}

impl Fixture {
    pub async fn new() -> Self {
        let mut url = url::Url::parse(
            &std::env::var("VEHICLE_CONTRACT_DATABASE_URL")
                .expect("explicit disposable fixture URL"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.path(), "/vehicle_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("vehicle_role_{suffix}");
        let database = format!("vehicle_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!("CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS"))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap.execute(sqlx::AssertSqlSafe(format!("CREATE SCHEMA panda AUTHORIZATION {role}; CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda; CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"))).await.unwrap();
        bootstrap.close().await;
        let set_role = role.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(6)
            .after_connect(move |c, _| {
                let role = set_role.clone();
                Box::pin(async move {
                    c.execute(sqlx::AssertSqlSafe(format!("SET ROLE {role}")))
                        .await?;
                    c.execute("SET search_path=panda,timeseries,public").await?;
                    Ok(())
                })
            })
            .connect(url.as_str())
            .await
            .unwrap();
        crate::db::migrations::run_current_migrations(&pool)
            .await
            .unwrap();
        let mut state = super::super::tests::make_helper_state(std::env::var("REDIS_URL").unwrap());
        state.pool = pool;
        state.config.database_url = url.to_string();
        state.config.panda_env = Some("production".into());
        state.config.vehicle_image_cache_dir = format!("/review/assets-{suffix}");
        crate::private_deployment::history::protect_history(&state.pool, &state.config)
            .await
            .unwrap();
        let restricted: bool = sqlx::query_scalar("SELECT NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolbypassrls AND NOT has_table_privilege(current_user,'timeseries.telemetry','DELETE') AND NOT has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE') FROM pg_roles WHERE rolname=current_user").fetch_one(&state.pool).await.unwrap();
        assert!(restricted);
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT to_regclass('panda.active_charge_checkpoints') IS NOT NULL"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap());
        Self {
            state,
            admin,
            database,
            role,
        }
    }

    pub async fn seed(&self) {
        for (n, email, role) in [
            (1, "owner@example.invalid", "user"),
            (2, "manager@example.invalid", "user"),
            (3, "viewer@example.invalid", "user"),
            (4, "foreign@example.invalid", "user"),
            (5, "admin@example.invalid", "admin"),
            (6, "recipient@example.invalid", "user"),
        ] {
            sqlx::query("INSERT INTO panda.users(id,email,role,created_at,updated_at) VALUES($1,$2,$3,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(email).bind(role).execute(&self.state.pool).await.unwrap();
        }
        for (n, owner, remote, model) in [
            (101, 1, "demo-workflow-one", "R1T"),
            (102, 1, "demo-workflow-two", "R2"),
            (103, 4, "demo-foreign", "R1S"),
        ] {
            sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at) VALUES($1,$2,$3,$4,'2026-01-01Z')").bind(id(n)).bind(id(owner)).bind(remote).bind(model).execute(&self.state.pool).await.unwrap();
        }
        for (user, vehicle, role) in [
            (1, 101, "owner"),
            (1, 102, "owner"),
            (2, 101, "manager"),
            (3, 101, "viewer"),
            (4, 103, "owner"),
        ] {
            sqlx::query("INSERT INTO panda.vehicle_memberships(user_id,vehicle_id,role,created_at,updated_at) VALUES($1,$2,$3,'2026-01-01Z','2026-01-01Z')").bind(id(user)).bind(id(vehicle)).bind(role).execute(&self.state.pool).await.unwrap();
        }
    }

    pub fn auth(&self, n: u128) -> AuthUser {
        AuthUser {
            user_id: id(n),
            default_vehicle_id: None,
            api_access_level: None,
            api_vehicle_id: None,
        }
    }

    pub async fn request(
        &self,
        user: Option<u128>,
        method: &str,
        path: &str,
        body: Value,
    ) -> Value {
        self.raw(user, method, path, serde_json::to_vec(&body).unwrap())
            .await
    }

    pub async fn raw(&self, user: Option<u128>, method: &str, path: &str, body: Vec<u8>) -> Value {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(n) = user {
            request = request.header(
                "authorization",
                format!(
                    "Bearer {}",
                    crate::middleware::auth::issue_access_token(id(n), None, &self.state.jwt_keys)
                        .unwrap()
                ),
            );
        }
        let router = Router::new()
            .nest(
                "/v1",
                super::super::router().merge(crate::routes::schedules::router()),
            )
            .layer(Extension(self.state.jwt_keys.decoding.clone()))
            .with_state(self.state.clone());
        response(
            router
                .oneshot(request.body(Body::from(body)).unwrap())
                .await
                .unwrap(),
        )
        .await
    }

    pub async fn snapshot(&self) -> Value {
        let mut rows = serde_json::Map::new();
        for table in [
            "users",
            "vehicles",
            "vehicle_memberships",
            "vehicle_user_settings",
            "vehicle_invites",
            "vehicle_runtime_state",
            "vehicle_images",
            "vehicle_artwork_cache_state",
            "charging_schedules",
            "departure_schedules",
            "wallboxes",
        ] {
            let values: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY to_jsonb(t)::text"
            )))
            .fetch_all(&self.state.pool)
            .await
            .unwrap();
            rows.insert(table.into(), json!(values));
        }
        Value::Object(rows)
    }

    pub async fn stage(&self, n: u128) {
        let mut c = self
            .state
            .redis
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        store_encrypted_redis(
            &self.state,
            &mut c,
            &format!("rivian:connect:{}", id(n)),
            &tokens(),
            300,
        )
        .await
        .unwrap();
    }

    pub async fn cleanup(self) {
        let mut c = self
            .state
            .redis
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        for n in 1..=6 {
            let _: () = redis::AsyncCommands::del(&mut c, format!("rivian:connect:{}", id(n)))
                .await
                .unwrap();
        }
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
        if std::path::Path::new(&self.state.config.vehicle_image_cache_dir).exists() {
            std::fs::remove_dir_all(&self.state.config.vehicle_image_cache_dir).unwrap();
        }
    }
}

pub(super) fn tokens() -> crate::ingestion::session_store::RivianTokenBundle {
    crate::ingestion::session_store::RivianTokenBundle {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: "synthetic-user".into(),
        csrf_token: "synthetic-csrf".into(),
        created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
    }
}

pub(super) async fn response(r: Response) -> Value {
    let status = r.status().as_u16();
    let content_type = r
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_owned());
    let cache = r
        .headers()
        .get("cache-control")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(r.into_body(), 8 * 1024 * 1024).await.unwrap();
    let body = serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| json!(String::from_utf8(bytes.to_vec()).unwrap()));
    json!({"status":status,"content_type":content_type,"cache_control":cache,"body":body})
}

pub(super) async fn result(r: Result<Json<Value>, AppError>) -> Value {
    response(r.into_response()).await
}

#[derive(Default)]
pub(super) struct Evidence {
    pub events: Vec<Value>,
    pub aliases: BTreeMap<String, String>,
}
impl Evidence {
    pub fn identity(&mut self, raw: impl ToString, name: &str) {
        self.aliases.insert(raw.to_string(), name.into());
    }
    pub fn push(&mut self, name: &str, value: Value) {
        self.events.push(json!({"case":name,"value":value}));
    }
    pub async fn record(&mut self, f: &Fixture, name: &str, response: Value) {
        let rows = f.snapshot().await;
        for row in rows["vehicle_memberships"].as_array().unwrap() {
            let vehicle = row["vehicle_id"].as_str().unwrap();
            let vehicle = self
                .aliases
                .get(vehicle)
                .map(String::as_str)
                .unwrap_or(vehicle);
            let alias = format!(
                "<membership-{}-{vehicle}>",
                row["user_id"].as_str().unwrap()
            );
            self.identity(row["id"].as_str().unwrap(), &alias);
        }
        self.push(name, json!({"response":response,"rows":rows}));
    }
    pub fn finish(self, name: &str, expected: &str) {
        let run = std::env::var("VEHICLE_CONTRACT_RUN").unwrap_or_else(|_| "actual".into());
        let raw = json!(self.events);
        std::fs::write(
            format!("/review/{run}-{name}-raw.json"),
            serde_json::to_vec_pretty(&raw).unwrap(),
        )
        .unwrap();
        let mut normalized = raw;
        normalize(&mut normalized, "", &self.aliases);
        std::fs::write(
            format!("/review/{run}-{name}.json"),
            serde_json::to_vec_pretty(&normalized).unwrap(),
        )
        .unwrap();
        if std::env::var_os("VEHICLE_CONTRACT_CAPTURE").is_none() {
            assert!(normalized == serde_json::from_str::<Value>(expected).unwrap(),"frozen {name} contract differs; complete actual output is in /review/{run}-{name}.json");
        }
    }
}
fn normalize(value: &mut Value, key: &str, aliases: &BTreeMap<String, String>) {
    match value {
        Value::String(s) => {
            if let Some(alias) = aliases.get(s) {
                *s = alias.clone();
            } else if [
                "created_at",
                "updated_at",
                "accepted_at",
                "revoked_at",
                "last_repair_attempt_at",
                "last_repair_success_at",
                "token_created_at",
                "last_refreshed_at",
                "next_attempt_at",
            ]
            .contains(&key)
                && (s.starts_with("2026-") && !s.starts_with("2026-01-01"))
            {
                *s = "<generated-clock>".into();
            } else {
                for (raw, alias) in aliases {
                    if s.contains(raw) {
                        *s = s.replace(raw, alias);
                    }
                }
            }
        }
        Value::Array(a) => {
            for v in a.iter_mut() {
                normalize(v, key, aliases);
            }
            if [
                "users",
                "vehicles",
                "vehicle_memberships",
                "vehicle_user_settings",
                "vehicle_invites",
                "vehicle_runtime_state",
                "vehicle_images",
                "vehicle_artwork_cache_state",
            ]
            .contains(&key)
            {
                a.sort_by_key(Value::to_string);
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                normalize(v, k, aliases);
            }
        }
        _ => {}
    }
}

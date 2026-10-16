use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

pub struct Fixture {
    admin: PgPool,
    pub state: State,
    database: String,
    role: String,
}

pub struct State {
    pub pool: PgPool,
}

impl Fixture {
    pub async fn new() -> Self {
        use sqlx::Executor;
        let mut url = url::Url::parse(
            &std::env::var("JOBS_DATABASE_URL")
                .expect("explicit disposable JOBS_DATABASE_URL required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "jobs_fixture");
        assert_eq!(url.path(), "/jobs_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("jobs_role_{suffix}");
        let database = format!("jobs_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_jobs_app'"
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
        url.set_password(Some("synthetic_jobs_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
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
            "database_url":url.as_str(), "redis_url":"redis://unused", "panda_env":"production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        let installation: Value = sqlx::query_scalar(
            "SELECT jsonb_build_object(
                'database',current_database(),'role',current_user,
                'superuser',(SELECT rolsuper FROM pg_roles WHERE rolname=current_user),
                'migrations',(SELECT jsonb_agg(jsonb_build_object('version',version,'success',success,'checksum',encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations),
                'trip_delete',has_table_privilege(current_user,'panda.trips','DELETE'),
                'telemetry_truncate',has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE'))",
        ).fetch_one(&pool).await.unwrap();
        if let Some(directory) = evidence_directory() {
            let directory = directory.join("installations");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join(format!("{database}.json")),
                serde_json::to_string_pretty(&installation).unwrap(),
            )
            .unwrap();
        }
        Self {
            admin,
            state: State { pool },
            database,
            role,
        }
    }

    pub async fn user(&self, role: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO panda.users (email, role) VALUES ($1, $2) RETURNING id")
            .bind(format!("{}@example.invalid", Uuid::new_v4()))
            .bind(role)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    pub async fn vehicle(&self, owner: Uuid, remote_id: &str) -> Uuid {
        let vehicle_id = sqlx::query_scalar(
            "INSERT INTO panda.vehicles (user_id, rivian_vehicle_id, model)
             VALUES ($1, $2, 'R1T') RETURNING id",
        )
        .bind(owner)
        .bind(remote_id)
        .fetch_one(&self.state.pool)
        .await
        .unwrap();
        self.member(owner, vehicle_id, "owner").await;
        vehicle_id
    }

    pub async fn member(&self, user_id: Uuid, vehicle_id: Uuid, role: &str) {
        sqlx::query(
            "INSERT INTO panda.vehicle_memberships (user_id, vehicle_id, role)
             VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(vehicle_id)
        .bind(role)
        .execute(&self.state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO panda.vehicle_user_settings (user_id, vehicle_id) VALUES ($1, $2)",
        )
        .bind(user_id)
        .bind(vehicle_id)
        .execute(&self.state.pool)
        .await
        .unwrap();
    }

    pub async fn cleanup(self) {
        use sqlx::Executor;
        self.state.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER DATABASE {} ALLOW_CONNECTIONS false",
                self.database
            )))
            .await
            .unwrap();
        for attempt in 0..10 {
            let result = self
                .admin
                .execute(sqlx::AssertSqlSafe(format!(
                    "DROP DATABASE {} WITH (FORCE)",
                    self.database
                )))
                .await;
            match result {
                Ok(_) => break,
                Err(error)
                    if attempt < 9
                        && error
                            .as_database_error()
                            .and_then(|error| error.code())
                            .as_deref()
                            == Some("55006") =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(error) => panic!("disposable enrichment database cleanup failed: {error}"),
            }
        }
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
    }
}

pub fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub async fn database() -> (PgPool, PgPool, String) {
    let fixture = Fixture::new().await;
    (fixture.state.pool, fixture.admin, fixture.database)
}
pub async fn cleanup(pool: PgPool, admin: PgPool, name: String) {
    Fixture {
        state: State { pool },
        admin,
        role: name.replace("jobs_contract_", "jobs_role_"),
        database: name,
    }
    .cleanup()
    .await;
}
pub async fn vehicle(pool: &PgPool, n: u128) -> Uuid {
    sqlx::query("INSERT INTO panda.users(id,email,password_hash,created_at,updated_at) VALUES($1,$2,'synthetic','2026-01-01Z','2026-01-01Z')")
        .bind(id(n+10000)).bind(format!("history-{n}@example.invalid")).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,name,created_at,updated_at) VALUES($1,$2,$3,'R1T','Synthetic','2026-01-01Z','2026-01-01Z')")
        .bind(id(n)).bind(id(n+10000)).bind(format!("history-{n}")).execute(pool).await.unwrap();
    id(n)
}
pub async fn trip(pool: &PgPool, n: u128, vehicle: Uuid) -> Uuid {
    sqlx::query("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at,start_lat,start_lng,distance_miles,energy_wh,route_preview,created_at) VALUES($1,$2,'2026-01-01T12:00Z','2026-01-01T12:05Z',30,-97,12.5,5000,'[[1,2],[3,4]]','2026-01-01Z')")
        .bind(id(n)).bind(vehicle).execute(pool).await.unwrap();
    id(n)
}
pub async fn credentials(pool: &PgPool, v: Uuid) {
    sqlx::query("INSERT INTO panda.vehicle_credentials(vehicle_id,encrypted_tokens,token_created_at) VALUES($1,$2,'2026-01-01Z')")
        .bind(v).bind(vec![0u8,1,2]).execute(pool).await.unwrap();
}
pub async fn snapshot(pool: &PgPool, extra: Value) -> Value {
    let mut result = json!({"observations":extra});
    result["usage_days"] = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object('connection_id',connection_id,'age_days',CURRENT_DATE-usage_date) ORDER BY connection_id),'[]'::jsonb) FROM panda.external_connection_activity",
    ).fetch_one(pool).await.unwrap();
    for (key, table, order) in [
        ("vehicles", "panda.vehicles", "id"),
        ("trips", "panda.trips", "id"),
        ("jobs", "panda.weather_enrichment_jobs", "trip_id"),
        (
            "weather",
            "panda.trip_weather_samples",
            "trip_id,sampled_at",
        ),
        (
            "activity",
            "panda.external_connection_activity",
            "connection_id",
        ),
        ("settings", "panda.external_connection_settings", "id"),
        ("sessions", "panda.charge_sessions", "id"),
        ("telemetry", "timeseries.telemetry", "vehicle_id,ts"),
        (
            "annotations",
            "panda.charge_session_user_annotations",
            "charge_session_id,user_id",
        ),
        (
            "trip_annotations",
            "panda.trip_user_annotations",
            "trip_id,user_id",
        ),
        ("credentials", "panda.vehicle_credentials", "vehicle_id"),
        ("addresses", "panda.addresses", "id"),
        ("geofences", "panda.geofences", "id"),
    ] {
        result[key] = sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(format!(
            "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY {order}), '[]'::jsonb) FROM {table} t"
        )))
        .fetch_one(pool)
        .await
        .unwrap();
    }
    result
}
pub fn evidence(name: &str, raw: &Value) {
    let mut normalized = raw.clone();
    fn normalize(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if matches!(
                        key.as_str(),
                        "created_at"
                            | "updated_at"
                            | "completed_at"
                            | "next_attempt_at"
                            | "last_attempt_at"
                            | "last_success_at"
                            | "history_backfilled_at"
                            | "usage_date"
                    ) && !value.is_null()
                        && !value.as_str().is_some_and(|value| {
                            ["2020-01-", "2026-01-", "2100-01-"]
                                .iter()
                                .any(|prefix| value.starts_with(prefix))
                        })
                    {
                        *value = json!("<clock>");
                    } else {
                        normalize(value);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    normalize(value);
                }
            }
            _ => {}
        }
    }
    normalize(&mut normalized);
    let text = serde_json::to_string_pretty(&normalized).unwrap() + "\n";
    if let Some(directory) = evidence_directory() {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{name}-raw.json")),
            serde_json::to_string_pretty(raw).unwrap() + "\n",
        )
        .unwrap();
        std::fs::write(directory.join(format!("{name}.json")), &text).unwrap();
    }
    let expected = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/services/__tests__/enrichment_jobs")
        .join(format!("{name}.json"));
    assert_eq!(
        text,
        std::fs::read_to_string(expected).unwrap(),
        "{name} full state changed"
    );
}

fn evidence_directory() -> Option<std::path::PathBuf> {
    std::env::var("JOBS_EVIDENCE")
        .ok()
        .map(|run| std::path::Path::new("/review").join(run))
}

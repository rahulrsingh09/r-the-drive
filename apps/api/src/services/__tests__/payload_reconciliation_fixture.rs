use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{Executor, PgPool};
use uuid::Uuid;

pub(crate) struct Fixture {
    pub pool: PgPool,
    pub admin: PgPool,
    pub name: String,
    role: String,
}

impl Fixture {
    pub async fn new() -> Self {
        let mut url = url::Url::parse(
            &std::env::var("PAYLOAD_DATABASE_URL").expect("explicit disposable fixture URL"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "payload_fixture");
        assert_eq!(url.path(), "/payload_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let name = format!("payload_{}", Uuid::new_v4().simple());
        let role = format!("{name}_role");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_payload_app'"
        ))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {name} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{name}"));
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
        url.set_password(Some("synthetic_payload_app")).unwrap();
        let migrations = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migrations)
            .await
            .unwrap();
        migrations.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(10)
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
            pool,
            admin,
            name,
            role,
        }
    }

    pub async fn cleanup(self) {
        self.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER DATABASE {} ALLOW_CONNECTIONS false",
                self.name
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH (FORCE)",
                self.name
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

pub fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
pub fn instant(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

pub async fn vehicle(pool: &PgPool, n: u128) -> Uuid {
    sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,$2,'synthetic')")
        .bind(id(n + 10000))
        .bind(format!("payload-{n}@example.invalid"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,name) VALUES($1,$2,$3,'R1T','Synthetic')")
        .bind(id(n)).bind(id(n+10000)).bind(format!("payload-{n}")).execute(pool).await.unwrap();
    id(n)
}

pub async fn session(
    pool: &PgPool,
    n: u128,
    vehicle: Uuid,
    source: Option<&str>,
    start: &str,
    end: Option<&str>,
) -> Uuid {
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,source,started_at,ended_at) VALUES($1,$2,$3,$4,$5)")
        .bind(id(n)).bind(vehicle).bind(source).bind(instant(start)).bind(end.map(instant))
        .execute(pool).await.unwrap();
    id(n)
}

pub async fn snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar(r#"SELECT jsonb_build_object(
        'sessions', COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY s.id) FROM panda.charge_sessions s),'[]'::jsonb),
        'payloads', COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY p.vehicle_id,p.operation,p.rivian_transaction_id,p.captured_at,p.id) FROM panda.rivian_charge_payloads p),'[]'::jsonb),
        'aliases', COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.charge_session_id,a.external_id) FROM panda.charge_session_external_aliases a),'[]'::jsonb),
        'identities', COALESCE((SELECT jsonb_agg(to_jsonb(i) ORDER BY i.identity_key) FROM panda.rivian_charge_payload_identities i),'[]'::jsonb),
        'checkpoints', COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY c.vehicle_id) FROM panda.active_charge_checkpoints c),'[]'::jsonb),
        'pending', COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY c.session_id) FROM panda.pending_charge_completions c),'[]'::jsonb),
        'telemetry', COALESCE((SELECT jsonb_agg(to_jsonb(t) ORDER BY t.vehicle_id,t.ts) FROM timeseries.telemetry t),'[]'::jsonb))"#)
        .fetch_one(pool).await.unwrap()
}

pub fn evidence(name: &str, mut value: Value, ids: &[(Uuid, &str)]) {
    let dir = std::env::var("PAYLOAD_EVIDENCE_DIR").expect("evidence directory");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        format!("{dir}/{name}-raw.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    fn normalize(v: &mut Value, ids: &[(Uuid, &str)]) {
        match v {
            Value::Object(o) => {
                for (k, v) in o {
                    if (matches!(
                        k.as_str(),
                        "created_at" | "updated_at" | "first_seen_at" | "last_seen_at"
                    ) || (matches!(k.as_str(), "captured_at" | "latest_payload_captured_at")
                        && !v.as_str().is_some_and(|s| s.starts_with("2026-01-01"))))
                        && !v.is_null()
                    {
                        *v = json!("<clock>");
                    } else {
                        normalize(v, ids);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    normalize(v, ids);
                }
            }
            Value::String(s) => {
                if let Some((_, label)) = ids.iter().find(|(id, _)| id.to_string() == *s) {
                    *s = label.to_string();
                }
            }
            _ => {}
        }
    }
    normalize(&mut value, ids);
    std::fs::write(
        format!("{dir}/{name}.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    if std::env::var("PAYLOAD_CAPTURE").as_deref() != Ok("baseline") {
        let path = format!(
            "{}/src/services/__tests__/payload_reconciliation_goldens/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let expected: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value, expected, "complete output/state parity: {name}");
    }
}

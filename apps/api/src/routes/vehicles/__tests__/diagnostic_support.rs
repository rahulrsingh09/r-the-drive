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
            &std::env::var("VEHICLE_DIAGNOSTIC_DATABASE_URL")
                .expect("explicit disposable fixture URL"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.path(), "/diagnostic_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("diagnostic_role_{suffix}");
        let database = format!("diagnostic_contract_{suffix}");
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
        let ledger: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM public._sqlx_migrations ORDER BY version")
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(ledger.len(), 33);
        assert!(ledger.contains(&1000031));
        assert!(ledger.contains(&1000032));
        assert!(!crate::private_deployment::outbound::optional_traffic_allowed());
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
            sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES($1,$2,$3,$4,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(id(owner)).bind(remote).bind(model).execute(&self.state.pool).await.unwrap();
        }
        for (user, vehicle, role) in [
            (1, 101, "owner"),
            (1, 102, "owner"),
            (2, 101, "manager"),
            (3, 101, "viewer"),
            (4, 103, "owner"),
        ] {
            sqlx::query("INSERT INTO panda.vehicle_memberships(id,user_id,vehicle_id,role,created_at,updated_at) VALUES($4,$1,$2,$3,'2026-01-01Z','2026-01-01Z')").bind(id(user)).bind(id(vehicle)).bind(role).bind(id(user*1000+vehicle)).execute(&self.state.pool).await.unwrap();
        }
    }

    pub async fn telemetry(&self, dense: bool) {
        if dense {
            sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,speed_mph,battery_level) SELECT '2026-10-01Z'::timestamptz + n * interval '1 minute',$1,n::float8,CASE WHEN n%3=0 THEN NULL ELSE 50 END FROM generate_series(0,599) n")
                .bind(id(101)).execute(&self.state.pool).await.unwrap();
        } else {
            sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id) VALUES('2026-10-01Z',$1)")
                .bind(id(101))
                .execute(&self.state.pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,latitude,longitude,altitude_m,speed_mph,battery_level,battery_capacity_wh,distance_to_empty_mi,battery_limit,time_to_end_of_charge_min,cabin_temp_c,driver_temp_c,outside_temp_c,power_kw,regen_power_kw,heading_deg,odometer_miles,tire_fl_psi,tire_fr_psi,tire_rl_psi,tire_rr_psi,door_front_left_locked,ota_status,is_online,hvac_active,charger_status,tire_fl_status,tire_fl_valid) SELECT '2026-10-01Z'::timestamptz+n*interval '30 seconds',$1,CASE WHEN n%2=0 THEN 47.5 END,-122.25,n*10.0,n*2.5,CASE WHEN n%2=0 THEN 60+n END,135000.0,200-n,80,n,21.5,22.5,-2.5,n*1.5,n*0.5,90+n,12000+n,CASE WHEN n%2=0 THEN 45.5 END,46,47,48,n%2=0,CASE WHEN n=3 THEN 'sparse-needle' END,n%2=0,false,'connected',CASE WHEN n%2=1 THEN 'OK' END,true FROM generate_series(1,8) n")
                .bind(id(101)).execute(&self.state.pool).await.unwrap();
            sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,battery_level) VALUES('2026-09-30T23:59:59Z',$1,10),('2026-10-01T00:04:01Z',$1,90),('2026-10-01Z',$2,99)")
                .bind(id(101)).bind(id(103)).execute(&self.state.pool).await.unwrap();
        }
        sqlx::query("INSERT INTO panda.api_keys(id,user_id,vehicle_id,key_hash,access_level,name,created_at,updated_at) VALUES($1,$2,$3,$4,'read','synthetic diagnostics','2026-01-01Z','2026-01-01Z')")
            .bind(id(501)).bind(id(1)).bind(id(101)).bind(crate::routes::api_keys::hash_api_key("panda_synthetic_diagnostics_only").as_slice()).execute(&self.state.pool).await.unwrap();
    }

    pub async fn request(&self, actor: &str, vehicle: u128, suffix: &str, query: &str) -> Value {
        let mut request =
            Request::builder().uri(format!("/v1/vehicles/{}/{suffix}?{query}", id(vehicle)));
        let user = match actor {
            "owner" => Some(1),
            "manager" => Some(2),
            "viewer" => Some(3),
            "foreign" => Some(4),
            "admin" => Some(5),
            _ => None,
        };
        if let Some(user) = user {
            request = request.header(
                "authorization",
                format!(
                    "Bearer {}",
                    crate::middleware::auth::issue_access_token(
                        id(user),
                        None,
                        &self.state.jwt_keys
                    )
                    .unwrap()
                ),
            );
        } else if actor == "key" {
            request = request.header("authorization", "Bearer panda_synthetic_diagnostics_only");
        }
        let router = Router::new()
            .nest("/v1", super::super::router())
            .layer(Extension(self.state.jwt_keys.decoding.clone()))
            .with_state(self.state.clone());
        let response = router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers: BTreeMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
            .collect();
        let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        if let Some(length) = headers.get("content-length") {
            assert_eq!(length.parse::<usize>().unwrap(), bytes.len());
        }
        let body = serde_json::from_slice::<Value>(&bytes)
            .unwrap_or_else(|_| json!(String::from_utf8(bytes.to_vec()).unwrap()));
        json!({"status":status,"headers":headers,"body":body})
    }

    pub async fn snapshot(&self) -> Value {
        let mut rows = serde_json::Map::new();
        for table in [
            "panda.users",
            "panda.vehicles",
            "panda.vehicle_memberships",
            "panda.vehicle_user_settings",
            "panda.api_keys",
            "panda.vehicle_invites",
            "panda.active_charge_checkpoints",
            "panda.charge_sessions",
            "timeseries.telemetry",
        ] {
            let values: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT to_jsonb(t) FROM {table} t ORDER BY to_jsonb(t)::text"
            )))
            .fetch_all(&self.state.pool)
            .await
            .unwrap();
            rows.insert(table.into(), json!(values));
        }
        Value::Object(rows)
    }

    pub async fn protection(&self) -> Value {
        let role: Value = sqlx::query_scalar("SELECT jsonb_build_object('superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'inherit',rolinherit,'replication',rolreplication,'bypassrls',rolbypassrls,'telemetry_delete',has_table_privilege(current_user,'timeseries.telemetry','DELETE'),'charge_truncate',has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE')) FROM pg_roles WHERE rolname=current_user").fetch_one(&self.state.pool).await.unwrap();
        assert_eq!(
            role,
            json!({"superuser":false,"createdb":false,"createrole":false,"inherit":false,"replication":false,"bypassrls":false,"telemetry_delete":false,"charge_truncate":false})
        );
        let ledger: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(t)-'installed_on'-'execution_time' FROM public._sqlx_migrations t ORDER BY version").fetch_all(&self.state.pool).await.unwrap();
        if let Ok(directory) = std::env::var("VEHICLE_DIAGNOSTIC_EVIDENCE") {
            let run = std::env::var("VEHICLE_DIAGNOSTIC_RUN").unwrap_or_else(|_| "actual".into());
            let raw: Vec<Value> = sqlx::query_scalar(
                "SELECT to_jsonb(t) FROM public._sqlx_migrations t ORDER BY version",
            )
            .fetch_all(&self.state.pool)
            .await
            .unwrap();
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("{run}-{}-ledger-raw.json", self.database)),
                serde_json::to_vec_pretty(&raw).unwrap(),
            )
            .unwrap();
        }
        json!({"role":role,"ledger":ledger,"optional_traffic":crate::private_deployment::outbound::optional_traffic_allowed()})
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

#[derive(Default)]
pub(super) struct Evidence {
    pub events: Vec<Value>,
}
impl Evidence {
    pub async fn record(
        &mut self,
        f: &Fixture,
        name: &str,
        actor: &str,
        vehicle: u128,
        suffix: &str,
        query: &str,
    ) {
        let before = f.snapshot().await;
        let start = Utc::now();
        let response = f.request(actor, vehicle, suffix, query).await;
        let end = Utc::now();
        let after = f.snapshot().await;
        let mut compared = after.clone();
        if actor == "key" && response["status"] != 401 {
            for key in ["last_used_at", "updated_at"] {
                if compared["panda.api_keys"][0][key] != before["panda.api_keys"][0][key] {
                    let clock: chrono::DateTime<Utc> = compared["panda.api_keys"][0][key]
                        .as_str()
                        .unwrap()
                        .parse()
                        .unwrap();
                    assert!(clock >= start && clock <= end);
                    compared["panda.api_keys"][0][key] = before["panda.api_keys"][0][key].clone();
                }
            }
        }
        assert_eq!(before, compared, "unexpected write: {name}");
        if before != after {
            if let Ok(directory) = std::env::var("VEHICLE_DIAGNOSTIC_EVIDENCE") {
                let run =
                    std::env::var("VEHICLE_DIAGNOSTIC_RUN").unwrap_or_else(|_| "actual".into());
                std::fs::write(
                    std::path::Path::new(&directory).join(format!(
                        "{run}-{}-{name}-db-write.json",
                        suffix.replace('/', "-")
                    )),
                    serde_json::to_vec_pretty(
                        &json!({"start":start,"end":end,"before":before,"after":after}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
        self.events.push(json!({"case":name,"actor":actor,"vehicle":vehicle,"suffix":suffix,"query":query,"response":response}));
    }

    pub fn finish(self, name: &str, expected: &str) {
        let value = json!(self.events);
        if let Ok(directory) = std::env::var("VEHICLE_DIAGNOSTIC_EVIDENCE") {
            let run = std::env::var("VEHICLE_DIAGNOSTIC_RUN").unwrap_or_else(|_| "actual".into());
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{run}-{name}.json")),
                serde_json::to_vec_pretty(&value).unwrap(),
            )
            .unwrap();
        }
        if std::env::var_os("VEHICLE_DIAGNOSTIC_CAPTURE").is_none() {
            assert_eq!(
                value,
                serde_json::from_str::<Value>(expected).unwrap(),
                "frozen diagnostic {name} contract"
            );
        }
    }
}

pub(super) fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

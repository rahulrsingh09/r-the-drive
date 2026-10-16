use super::*;
use std::io::Write;

pub fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
pub struct Fixture {
    pub state: AppState,
    admin: PgPool,
    database: String,
    role: String,
}
impl Fixture {
    pub async fn new() -> Self {
        let configured = std::env::var("OPERATIONAL_DATABASE_URL")
            .expect("explicit disposable operational fixture required");
        let mut url = url::Url::parse(&configured).unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.path(), "/operational_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let database = format!("operational_{suffix}");
        let role = format!("operational_role_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!("CREATE ROLE {role} LOGIN PASSWORD 'synthetic_operational_role_only' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"))).await.unwrap();
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
        url.set_password(Some("synthetic_operational_role_only"))
            .unwrap();
        let migration = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migration)
            .await
            .unwrap();
        migration.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(12)
            .connect(url.as_str())
            .await
            .unwrap();
        let config=serde_json::from_value(json!({"database_url":url.as_str(),"redis_url":"redis://127.0.0.1:1","panda_env":"production"})).unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        let protected:bool=sqlx::query_scalar("SELECT NOT rolsuper AND NOT rolbypassrls AND NOT has_table_privilege(current_user,'panda.trips','DELETE') AND NOT has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE') FROM pg_roles WHERE rolname=current_user").fetch_one(&pool).await.unwrap();
        assert!(protected);
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT to_regclass('panda.active_charge_checkpoints') IS NOT NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap());
        let keys = crate::keys::generate_keys().unwrap();
        let state = AppState {
            pool,
            redis: redis::Client::open("redis://127.0.0.1:1").unwrap(),
            jwt_keys: std::sync::Arc::new(
                JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).unwrap(),
            ),
            age_key: keys.age_key,
            config,
            resources: Default::default(),
            nominatim_cache: Default::default(),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
        };
        Self {
            state,
            admin,
            database,
            role,
        }
    }
    pub async fn seed(&self) {
        let p = &self.state.pool;
        for (n, role) in [
            (1, "user"),
            (2, "user"),
            (3, "user"),
            (4, "admin"),
            (5, "user"),
            (6, "super_user"),
        ] {
            sqlx::query("INSERT INTO panda.users(id,email,role,created_at,updated_at) VALUES($1,$2,$3,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(format!("operational-{n}@example.invalid")).bind(role).execute(p).await.unwrap();
        }
        for (n, owner, model, name) in [
            (10, 1, "R1S", Some("Alpha")),
            (11, 1, "R1T", None),
            (12, 5, "R2", Some("Other")),
        ] {
            sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,name,created_at,updated_at) VALUES($1,$2,$3,$4,$5,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(id(owner)).bind(format!("synthetic-{n}")).bind(model).bind(name).execute(p).await.unwrap();
            for (user, role) in if n == 12 {
                vec![(5, "owner")]
            } else {
                vec![(1, "owner"), (2, "manager"), (3, "viewer")]
            } {
                sqlx::query("INSERT INTO panda.vehicle_memberships(vehicle_id,user_id,role,is_default,created_at,updated_at) VALUES($1,$2,$3,false,'2026-01-01Z','2026-01-01Z') ON CONFLICT(vehicle_id,user_id) DO UPDATE SET role=excluded.role,created_at=excluded.created_at,updated_at=excluded.updated_at")
                    .bind(id(n)).bind(id(user)).bind(role).execute(p).await.unwrap();
            }
        }
        sqlx::query("INSERT INTO panda.api_keys(id,user_id,vehicle_id,name,key_hash,access_level,last_used_at,created_at,updated_at) VALUES($1,$2,$3,'synthetic', $4,'read','2099-01-01Z','2026-01-01Z','2026-01-01Z')")
            .bind(id(30)).bind(id(1)).bind(id(10)).bind(crate::routes::api_keys::hash_api_key("panda_operational_synthetic_only").as_slice()).execute(p).await.unwrap();
        // Different sparse rows are deliberately the latest qualifying row for each health category.
        p.execute("INSERT INTO timeseries.telemetry(ts,vehicle_id,battery_level,battery_capacity_wh,power_kw,regen_power_kw,speed_mph,outside_temp_c,cabin_temp_c,odometer_miles,tire_fl_psi,tire_fl_status,door_front_left_closed,ota_current_version,hv_thermal_event,is_online) VALUES
            ('2026-01-01T00:00:00Z','00000000-0000-0000-0000-00000000000a',0,135000,-5,0,0,-12,0,999,0,'low',false,'2025.1','none',false),
            ('2026-01-01T00:00:01.123456Z','00000000-0000-0000-0000-00000000000a',50,NULL,12,2,45,20,22,1000,NULL,NULL,NULL,NULL,NULL,NULL),
            ('2026-01-01T00:00:02Z','00000000-0000-0000-0000-00000000000a',NULL,NULL,0,NULL,NULL,NULL,NULL,NULL,48,'ok',NULL,NULL,NULL,NULL),
            ('2026-01-01T00:00:03Z','00000000-0000-0000-0000-00000000000a',100,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,true,NULL,NULL,NULL),
            ('2026-01-01T00:00:04Z','00000000-0000-0000-0000-00000000000a',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'2026.2','hot',true),
            ('2099-01-01Z','00000000-0000-0000-0000-00000000000a',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,'hot',NULL),
            ('2026-01-01T00:00:01Z','00000000-0000-0000-0000-00000000000c',99,NULL,99,NULL,99,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)").await.unwrap();
        p.execute("INSERT INTO panda.software_versions(vehicle_id,version,installed_at,observed_until) SELECT '00000000-0000-0000-0000-00000000000a', 'v'||i, '2026-01-01Z'::timestamptz+i*interval '1 hour', CASE WHEN i=21 THEN NULL ELSE '2026-01-01Z'::timestamptz+(i+1)*interval '1 hour' END FROM generate_series(0,21)i;
            INSERT INTO panda.vehicle_runtime_state(vehicle_id,is_online,worker_health,auth_state,last_seen_at,last_event_at,updated_at,collector_heartbeat_at) VALUES
            ('00000000-0000-0000-0000-00000000000a',true,'connected','authorized','2099-01-01Z','2026-01-01Z','2026-01-01Z','2099-01-01Z'),
            ('00000000-0000-0000-0000-00000000000c',false,'connected','authorized','2000-01-01Z','2099-01-01Z','2099-01-01Z','2000-01-01Z');
            INSERT INTO panda.parallax_collector_state(vehicle_id,status,owner_kind,connected_at,updated_at,legacy_null_count,legacy_last_classification,reconnect_count) VALUES
            ('00000000-0000-0000-0000-00000000000a','connected','in_process','2026-01-01Z','2099-01-01Z',2,'null',3);
            INSERT INTO panda.rivian_stewardship_counters(vehicle_id,day,ws_messages_received,ws_heartbeats_received,telemetry_writes_persisted,telemetry_writes_suppressed,created_at,updated_at) VALUES
            ('00000000-0000-0000-0000-00000000000a','2099-01-01',12,3,7,2,'2026-01-01Z','2026-01-01Z'),
            ('00000000-0000-0000-0000-00000000000a','2000-01-01',100,100,100,100,'2026-01-01Z','2026-01-01Z'),
            ('00000000-0000-0000-0000-00000000000c','2099-01-01',8,2,4,1,'2026-01-01Z','2026-01-01Z');
            INSERT INTO panda.rivian_ws_raw_events(id,vehicle_id,received_at,event_type,payload_json) VALUES
            ('00000000-0000-0000-0000-000000000028','00000000-0000-0000-0000-00000000000a','2099-01-01Z','synthetic','{}'),
            ('00000000-0000-0000-0000-000000000029','00000000-0000-0000-0000-00000000000c','2000-01-01Z','synthetic','{}')").await.unwrap();
        for (table,columns,values) in [
            ("parallax_network_samples","wifi_connected,wifi_rssi_dbm,wifi_link_speed_mbps,wifi_frequency_mhz,wifi_channel_width_mhz,cellular_access_technology,cellular_signal_dbm","false,-80,0,2400,20,'LTE',-95"),
            ("parallax_efficiency_samples","reference_wh_per_km,learned_wh_per_km,mode_ranges_km","200,215,'{\"sport\":250}'"),
            ("parallax_mass_samples","estimated_mass_kg","3200"),
            ("parallax_cold_weather_samples","available_soc_pct,cold_limited_soc_pct,cold_range_impact_km","70,0,0")
        ] {p.execute(sqlx::AssertSqlSafe(format!("INSERT INTO timeseries.{table}(vehicle_id,source_at,received_at,payload_hash,{columns}) VALUES('00000000-0000-0000-0000-00000000000a','2026-01-01Z','2026-01-01Z',decode('0102','hex'),{values})"))).await.unwrap();}
        p.execute("INSERT INTO panda.vehicle_ingestion_diagnostics(vehicle_id,capture_id,enabled_by,enabled_until,started_at,updated_at,dropped_events) VALUES('00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000000014','00000000-0000-0000-0000-000000000001','2099-01-01T01:00:00Z','2026-01-01Z','2026-01-01Z',4);
            INSERT INTO panda.vehicle_ingestion_capture_events(vehicle_id,capture_id,recorded_at,kind,fields) SELECT '00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000000014','2026-01-01Z'::timestamptz+i*interval '1 millisecond',CASE WHEN i=0 THEN 'capture_started' WHEN i=1001 THEN 'truncated' ELSE 'legacy_frame' END,jsonb_build_object('speed_mph',i,'nested',jsonb_build_object('latitude',1,'VIN','synthetic','ok',false),'token','synthetic-only') FROM generate_series(0,1001)i;
            INSERT INTO panda.vehicle_ingestion_capture_events(vehicle_id,capture_id,recorded_at,kind,fields) VALUES('00000000-0000-0000-0000-00000000000c','00000000-0000-0000-0000-000000000015','2026-01-01Z','legacy_frame','{\"foreign\":true}')").await.unwrap();
    }
    pub fn router(&self) -> Router {
        Router::new()
            .nest(
                "/v1",
                super::super::router()
                    .merge(crate::routes::health::router())
                    .merge(crate::routes::rivian_stewardship::router())
                    .merge(crate::routes::vehicles::router()),
            )
            .layer(Extension(self.state.jwt_keys.decoding.clone()))
            .with_state(self.state.clone())
    }
    pub async fn http(&self, user: u128, method: &str, path: &str, body: Option<Value>) -> Value {
        self.raw_http(user, method, path, body.map(|v| v.to_string()))
            .await
    }
    pub async fn raw_http(
        &self,
        user: u128,
        method: &str,
        path: &str,
        body: Option<String>,
    ) -> Value {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if user != 0 {
            let token = if user == 7 {
                "panda_operational_synthetic_only".into()
            } else {
                issue_access_token(id(user), Some(id(10)), &self.state.jwt_keys).unwrap()
            };
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let response = self
            .router()
            .oneshot(req.body(Body::from(body.unwrap_or_default())).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), json!(v.to_str().unwrap())))
            .collect::<serde_json::Map<_, _>>();
        if status == 500 {
            self.settle_reads(if user == 7 { 1 } else { user }).await;
        }
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let dir = std::env::var("OPERATIONAL_ARTIFACT_DIR").unwrap();
        let mut raw = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{dir}/http-raw.jsonl"))
            .unwrap();
        writeln!(raw,"{}",json!({"user":user,"method":method,"path":path,"status":status,"headers":headers,"body_bytes_utf8":text})).unwrap();
        let value = if headers.get("content-type") == Some(&json!("application/x-ndjson")) {
            json!(text
                .lines()
                .map(|l| serde_json::from_str::<Value>(l).unwrap())
                .collect::<Vec<_>>())
        } else {
            serde_json::from_str(&text).unwrap_or(json!(text))
        };
        json!({"status":status,"headers":headers,"body":value})
    }
    async fn settle_reads(&self, user: u128) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let mut permits = Vec::new();
                for _ in 0..self.state.config.security.metrics_max_per_user {
                    if let Ok(permit) = self
                        .state
                        .resources
                        .heavy(id(user), &self.state.config.security)
                    {
                        permits.push(permit);
                    }
                }
                if permits.len() == self.state.config.security.metrics_max_per_user {
                    break;
                }
                drop(permits);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    pub async fn rows(&self) -> Value {
        let mut state = serde_json::Map::new();
        for table in [
            "panda.users",
            "panda.vehicles",
            "panda.vehicle_memberships",
            "panda.api_keys",
            "timeseries.telemetry",
            "panda.software_versions",
            "panda.vehicle_runtime_state",
            "panda.parallax_collector_state",
            "panda.rivian_stewardship_counters",
            "panda.rivian_ws_raw_events",
            "panda.vehicle_ingestion_diagnostics",
            "panda.vehicle_ingestion_capture_events",
            "panda.active_charge_checkpoints",
            "panda.active_trip_checkpoints",
            "panda.pending_trip_completions",
            "timeseries.parallax_network_samples",
            "timeseries.parallax_efficiency_samples",
            "timeseries.parallax_mass_samples",
            "timeseries.parallax_cold_weather_samples",
        ] {
            let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT to_jsonb(t) FROM {table} t ORDER BY to_jsonb(t)::text"
            )))
            .fetch_all(&self.state.pool)
            .await
            .unwrap();
            state.insert(table.into(), json!(rows));
        }
        json!(state)
    }
    pub async fn cleanup(self) {
        self.state.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER DATABASE {} ALLOW_CONNECTIONS false",
                self.database
            )))
            .await
            .unwrap();
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

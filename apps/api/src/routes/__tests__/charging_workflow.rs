use super::*;
use axum::body::{to_bytes, Body};
use http::Request;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn time(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn snapshot(name: &str, actual: Value, expected: &str) {
    if let Ok(dir) = std::env::var("CHARGING_CONTRACT_ARTIFACT_DIR") {
        std::fs::write(
            format!("{dir}/{name}-actual.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(expected).unwrap(),
        "{name} full contract"
    );
}

struct DatabaseFixture {
    admin: PgPool,
    pool: PgPool,
    database: String,
    role: String,
}

impl DatabaseFixture {
    async fn new() -> Self {
        use sqlx::Executor;
        let mut url = url::Url::parse(
            &std::env::var("CHARGING_CONTRACT_DATABASE_URL")
                .expect("explicit disposable CHARGING_CONTRACT_DATABASE_URL required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "charging_fixture");
        assert_eq!(url.path(), "/charging_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("charging_role_{suffix}");
        let database = format!("charging_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_charging_app'"
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
        url.set_password(Some("synthetic_charging_app")).unwrap();
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
        Self {
            admin,
            pool,
            database,
            role,
        }
    }

    async fn cleanup(self) {
        use sqlx::Executor;
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

async fn seed(pool: &PgPool) {
    for n in 1..=4 {
        sqlx::query("INSERT INTO panda.users(id,email,password_hash,created_at,updated_at) VALUES($1,$2,'synthetic inert hash','2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(format!("charging-{n}@example.invalid")).execute(pool).await.unwrap();
    }
    for (n, user) in [(10, 1), (11, 4), (12, 1)] {
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,battery_capacity_wh,created_at,updated_at) VALUES($1,$2,$3,'synthetic',135000,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(user)).bind(format!("synthetic-charging-{n}")).execute(pool).await.unwrap();
    }
    for (n, vehicle, user, role) in [
        (30, 10, 1, "owner"),
        (31, 10, 2, "viewer"),
        (32, 10, 3, "manager"),
        (33, 11, 4, "owner"),
        (34, 12, 1, "owner"),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_memberships(id,vehicle_id,user_id,role,created_at,updated_at) VALUES($1,$2,$3,$4,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(vehicle)).bind(id(user)).bind(role).execute(pool).await.unwrap();
    }
    for (n, user) in [(20, 1), (21, 4), (22, 3)] {
        sqlx::query("INSERT INTO panda.geofences(id,user_id,name,latitude,longitude,is_home,created_at,updated_at) VALUES($1,$2,$3,0,0,true,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(user)).bind(format!("Synthetic place {n}")).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.system_config(key,value) VALUES('app_timezone','America/Los_Angeles') ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value").execute(pool).await.unwrap();
    for (n, vehicle, started, ended, kind, energy, home, vendor) in [
        (
            100,
            10,
            "2026-01-01T07:30:00Z",
            Some("2026-01-01T08:00:00Z"),
            Some("dc"),
            Some(20.0),
            Some(false),
            Some(" Synthetic Network "),
        ),
        (
            101,
            10,
            "2026-01-02T09:00:00Z",
            Some("2026-01-02T10:00:00Z"),
            Some("ac"),
            Some(12.0),
            Some(true),
            None,
        ),
        (
            102,
            10,
            "2026-01-03T09:00:00Z",
            Some("2026-01-03T10:00:00Z"),
            Some("dcfc"),
            Some(30.0),
            Some(false),
            Some("synthetic network"),
        ),
        (
            103,
            10,
            "2026-01-04T09:00:00Z",
            Some("2026-01-04T10:00:00Z"),
            None,
            None,
            None,
            None,
        ),
        (
            104,
            10,
            "2026-01-05T09:00:00Z",
            Some("2026-01-05T10:00:00Z"),
            Some("dc"),
            Some(-1.0),
            Some(false),
            Some("Other Synthetic"),
        ),
        (
            105,
            10,
            "2026-01-06T09:00:00Z",
            None,
            Some("dc"),
            Some(2.0),
            None,
            Some("Synthetic Network"),
        ),
        (
            106,
            11,
            "2026-01-02T09:00:00Z",
            Some("2026-01-02T10:00:00Z"),
            Some("dc"),
            Some(999.0),
            Some(false),
            Some("Foreign Synthetic"),
        ),
        (
            107,
            10,
            "2026-01-07T09:00:00Z",
            Some("2026-01-07T10:00:00Z"),
            Some("dc"),
            Some(7.0),
            Some(false),
            Some("Sparse"),
        ),
    ] {
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,charger_type,kwh_added,is_home,network_vendor,soc_start,soc_end,duration_minutes,cost_usd,source_location_lat,source_location_lng,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,20,80,60,9.5,0,0,'2026-01-01Z')")
            .bind(id(n)).bind(id(vehicle)).bind(time(started)).bind(ended.map(time)).bind(kind).bind(energy).bind(home).bind(vendor).execute(pool).await.unwrap();
    }
    for (session, ts, soc, power) in [
        (100, "2026-01-01T07:30:00Z", Some(20.0), Some(-400.0)),
        (100, "2026-01-01T07:31:00Z", Some(21.0), None),
        (100, "2026-01-01T07:32:00Z", Some(21.0), Some(25.0)),
        (100, "2026-01-01T07:33:00Z", None, Some(30.0)),
        (101, "2026-01-02T09:00:00Z", Some(20.0), Some(25.0)),
        (101, "2026-01-02T09:01:00Z", Some(21.0), Some(-8.0)),
        (107, "2026-01-07T09:00:00Z", Some(20.0), None),
    ] {
        sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,charge_session_id,ts,battery_level,power_kw,battery_capacity_wh) VALUES($1,$2,$3,$4,$5,135000)")
            .bind(id(10)).bind(id(session)).bind(time(ts)).bind(soc).bind(power).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,battery_level,power_kw) VALUES($1,'2026-01-01T07:34:00Z',33,12)").bind(id(10)).execute(pool).await.unwrap();
    for (ts, power) in [
        ("2026-01-03T09:00:00Z", 100.0),
        ("2026-01-03T09:30:00Z", 80.0),
        ("2026-01-03T10:00:00Z", 0.0),
    ] {
        sqlx::query("INSERT INTO panda.rivian_charge_curve_points(vehicle_id,charge_session_id,ts,power_kw,captured_at) VALUES($1,$2,$3,$4,'2026-01-01Z')")
            .bind(id(10)).bind(id(102)).bind(time(ts)).bind(power).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO timeseries.parallax_charge_curve_points(vehicle_id,charge_session_id,source_at,segment_index,power_kw,soc,received_at) VALUES($1,$2,'2026-01-03T09:00:00Z',0,140,22,'2026-01-01Z')")
        .bind(id(10)).bind(id(102)).execute(pool).await.unwrap();

    for (ts, soc, power) in [
        ("2026-01-04T09:00:00Z", 20.0, Some(5.0)),
        ("2026-01-04T09:01:00Z", 21.0, None),
        ("2026-01-04T09:07:00Z", 22.0, None),
    ] {
        sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,battery_level,power_kw) VALUES($1,$2,$3,$4)")
            .bind(id(10)).bind(time(ts)).bind(soc).bind(power).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.api_keys(id,user_id,vehicle_id,name,key_hash,access_level,created_at,updated_at) VALUES($1,$2,$3,'synthetic charging key',$4,'read','2026-01-01Z','2026-01-01Z')")
        .bind(id(80)).bind(id(1)).bind(id(10)).bind(crate::routes::api_keys::hash_api_key("panda_synthetic_charging_only").as_slice()).execute(pool).await.unwrap();
}

fn state(pool: PgPool) -> AppState {
    let keys = crate::keys::generate_keys().unwrap();
    AppState {
        pool,
        redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
        jwt_keys: Arc::new(crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem,&keys.jwt_public_pem).unwrap()),
        age_key: "synthetic-local-unused".into(),
        config: serde_json::from_value(json!({"database_url":"postgres://unused", "redis_url":"redis://127.0.0.1:1/", "rate_limit":{"auth_read_per_minute":100000,"auth_read_burst":100000,"auth_write_per_minute":100000,"auth_write_burst":100000}})).unwrap(),
        nominatim_cache: Default::default(), supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(), resources: Default::default(),
    }
}

fn normalize(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if ["updated_at", "computed_at", "created_at"].contains(&key.as_str())
                    && value.is_string()
                    && !value.as_str().unwrap().starts_with("2026-01-01T00:00:00")
                {
                    *value = json!("<database clock>");
                } else if key == "live_time_elapsed_seconds"
                    && value.is_number()
                    && value.as_i64().unwrap() > 100000
                {
                    *value = json!("<live elapsed clock>");
                } else {
                    normalize(value);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item);
            }
        }
        _ => {}
    }
}

async fn persisted(pool: &PgPool) -> Value {
    let mut result = json!({});
    for (table, order) in [
        ("charge_sessions", "id"),
        (
            "charge_session_user_annotations",
            "charge_session_id,user_id",
        ),
        (
            "vehicle_charging_network_preferences",
            "vehicle_id,vendor_normalized",
        ),
        ("geofences", "id"),
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY {order}"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        result[table] = json!(rows);
    }

    for (name, table, order) in [
        ("telemetry", "timeseries.telemetry", "vehicle_id,ts"),
        (
            "rivian_curve",
            "panda.rivian_charge_curve_points",
            "vehicle_id,ts",
        ),
        (
            "parallax_curve",
            "timeseries.parallax_charge_curve_points",
            "vehicle_id,source_at,segment_index",
        ),
    ] {
        let data: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM {table} t ORDER BY {order}"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        result[name] = json!(data);
    }
    result
}

async fn request(
    state: &AppState,
    method: &str,
    uri: String,
    user: Option<u128>,
    body: Option<String>,
) -> Value {
    let mut req = Request::builder()
        .method(method)
        .uri(uri.clone())
        .header("x-request-id", "synthetic-charging-contract");
    if let Some(user) = user {
        let token = if user == 0 {
            "panda_synthetic_charging_only".to_owned()
        } else {
            crate::middleware::auth::issue_access_token(id(user), Some(id(10)), &state.jwt_keys)
                .unwrap()
        };
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let mut req = req.body(Body::from(body.unwrap_or_default())).unwrap();
    req.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:40000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = crate::routes::build_router(state.clone())
        .oneshot(req)
        .await
        .unwrap();
    let status = response.status().as_u16();
    let mut headers = serde_json::Map::new();
    for (key, value) in response.headers() {
        if ![
            "x-ratelimit-remaining",
            "x-ratelimit-after",
            "x-panda-ratelimit-reset",
        ]
        .contains(&key.as_str())
        {
            headers.insert(key.to_string(), json!(value.to_str().unwrap()));
        }
    }
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let output = serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| json!(String::from_utf8(bytes.to_vec()).unwrap()));
    json!({"method":method,"uri":uri,"user":user,"status":status,"headers":headers,"body":output})
}

#[tokio::test]
#[ignore = "requires isolated CHARGING_CONTRACT_DATABASE_URL"]
async fn charging_workflow_full_http_database_contract() {
    let fixture = DatabaseFixture::new().await;
    seed(&fixture.pool).await;
    let state = state(fixture.pool.clone());
    let mut rows = vec![];
    let restricted:Value=sqlx::query_scalar("SELECT jsonb_build_object('superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,'charge_delete',has_table_privilege(current_user,'panda.charge_sessions','DELETE'),'charge_truncate',has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE')) FROM pg_roles WHERE rolname=current_user").fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(
        restricted,
        json!({"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"charge_delete":true,"charge_truncate":false})
    );
    rows.push(json!({"privileges":restricted,"initial":persisted(&fixture.pool).await}));
    let bounds = "from=2026-01-01T00%3A00%3A00Z&to=2026-01-10T00%3A00%3A00Z";
    for vehicle in [10, 12] {
        for route in [
            "charging",
            "charging/sessions",
            "charging/chart-series",
            "charging/summary",
            "charging/curve-analysis",
        ] {
            rows.push(
                request(
                    &state,
                    "GET",
                    format!("/v1/{route}?vehicle_id={}&{bounds}", id(vehicle)),
                    Some(1),
                    None,
                )
                .await,
            );
        }
    }
    for suffix in [
        "limit=2",
        "per_page=2&page=2",
        "limit=3&offset=1",
        "per_page=0&page=-2&offset=-1",
        "limit=999",
        "session_day_local=2025-12-31",
        "session_day_local=not-a-date",
        "lifetime=true&limit=1",
        "page=1&per_page=200&offset=999",
    ] {
        rows.push(
            request(
                &state,
                "GET",
                format!("/v1/charging?vehicle_id={}&{bounds}&{suffix}", id(10)),
                Some(1),
                None,
            )
            .await,
        );
    }
    for session in 100..=107 {
        for suffix in ["", "/curve"] {
            rows.push(
                request(
                    &state,
                    "GET",
                    format!("/v1/charging/{}{suffix}?vehicle_id={}", id(session), id(10)),
                    Some(1),
                    None,
                )
                .await,
            );
        }
    }
    for route in [
        format!("vehicles/{}/charging-sessions?{bounds}", id(10)),
        format!("vehicles/{}/costs?{bounds}", id(10)),
        format!("charging/sessions/{}?vehicle_id={}", id(100), id(10)),
        format!("charging/sessions/{}/curve?vehicle_id={}", id(100), id(10)),
        format!("vehicles/{}/charging-sessions/{}", id(10), id(100)),
        format!("vehicles/{}/charging-sessions/{}/curve", id(10), id(100)),
    ] {
        rows.push(request(&state, "GET", format!("/v1/{route}"), Some(1), None).await);
    }
    for user in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
        for uri in [
            format!("/v1/charging?vehicle_id={}&{bounds}", id(10)),
            format!("/v1/charging/{}?vehicle_id={}", id(999), id(10)),
            format!("/v1/charging?vehicle_id={}&{bounds}", id(11)),
            format!("/v1/vehicles/{}/charging-networks", id(10)),
        ] {
            rows.push(request(&state, "GET", uri, user, None).await);
        }
    }
    for query in [
        "",
        "vehicle_id=bad",
        "vehicle_id=00000000-0000-0000-0000-00000000000a&from=bad",
        "vehicle_id=00000000-0000-0000-0000-00000000000a&limit=no",
    ] {
        rows.push(
            request(
                &state,
                "GET",
                format!("/v1/charging?{query}"),
                Some(1),
                None,
            )
            .await,
        );
    }
    let uri = format!("/v1/charging/{}?vehicle_id={}", id(100), id(10));
    for (user, payload) in [
        (0, json!({})),
        (2, json!({})),
        (4, json!({"cost_mode":"manual","cost_usd":-1})),
        (
            1,
            json!({"location_mode":"saved_place","cost_mode":"manual","cost_usd":-1}),
        ),
        (1, json!({"cost_mode":"manual"})),
        (1, json!({"cost_mode":"manual","cost_usd":-1})),
        (1, json!({"cost_usd":3})),
        (1, json!({"cost_mode":"free","cost_usd":0})),
        (1, json!({"location_mode":"saved_place","place_id":id(21)})),
        (1, json!({})),
        (1, json!({"cost_mode":"manual","cost_usd":0})),
        (1, json!({"cost_mode":"manual","cost_usd":12.25})),
        (1, json!({"cost_mode":"manual","cost_usd":12.25})),
        (1, json!({"place_id":id(20)})),
        (1, json!({"place_id":null})),
        (1, json!({"location_mode":"automatic"})),
        (1, json!({"cost_mode":"free"})),
        (1, json!({"cost_mode":"automatic"})),
        (
            3,
            json!({"place_id":id(22),"cost_mode":"manual","cost_usd":2.5}),
        ),
        (1, json!({"place_id":id(999)})),
    ] {
        let response = request(
            &state,
            "PATCH",
            uri.clone(),
            Some(user),
            Some(payload.to_string()),
        )
        .await;
        rows.push(json!({"payload":payload,"response":response,"persisted":persisted(&fixture.pool).await}));
    }
    for raw in [
        "{".into(),
        "{\"cost_mode\":\"unknown\"}".into(),
        "{\"cost_mode\":\"manual\",\"cost_usd\":1e999}".into(),
        format!("{{\"padding\":\"{}\"}}", "x".repeat(65536)),
    ] {
        rows.push(json!({"response":request(&state,"PATCH",uri.clone(),Some(1),Some(raw)).await,"persisted":persisted(&fixture.pool).await}));
    }
    for (user, vendor, mode) in [
        (2, "Synthetic%20Network", "free"),
        (4, "Synthetic%20Network", "free"),
        (1, "%20", "free"),
        (1, "Missing", "free"),
        (1, "Synthetic%20Network", "free"),
        (1, "Synthetic%20Network", "free"),
        (3, "Synthetic%20Network", "automatic"),
    ] {
        rows.push(json!({"response":request(&state,"PATCH",format!("/v1/vehicles/{}/charging-networks/{vendor}",id(10)),Some(user),Some(json!({"cost_mode":mode}).to_string())).await,"persisted":persisted(&fixture.pool).await}));
    }
    rows.push(
        request(
            &state,
            "PATCH",
            format!("/v1/vehicles/{}/charging-sessions/{}", id(10), id(101)),
            Some(1),
            Some("{\"place_id\":null}".into()),
        )
        .await,
    );
    rows.push(
        request(
            &state,
            "PATCH",
            format!("/v1/charging/{}?vehicle_id={}", id(999), id(10)),
            Some(1),
            Some("{\"cost_mode\":\"manual\",\"cost_usd\":1}".into()),
        )
        .await,
    );
    rows.push(json!({"final":persisted(&fixture.pool).await}));

    for route in [
        "charging",
        "charging/summary",
        "charging/chart-series",
        "charging/curve-analysis",
    ] {
        for suffix in ["", "&lifetime=true"] {
            rows.push(
                request(
                    &state,
                    "GET",
                    format!("/v1/{route}?vehicle_id={}{suffix}", id(10)),
                    Some(1),
                    None,
                )
                .await,
            );
        }
    }
    sqlx::raw_sql("CREATE FUNCTION panda.synthetic_reject_annotation() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'synthetic annotation failure'; END$$;
        CREATE TRIGGER synthetic_reject_annotation BEFORE INSERT OR UPDATE ON panda.charge_session_user_annotations FOR EACH ROW EXECUTE FUNCTION panda.synthetic_reject_annotation();")
        .execute(&fixture.pool).await.unwrap();
    for (uri, payload) in [
        (
            format!("/v1/charging/{}?vehicle_id={}", id(100), id(10)),
            json!({"location_mode":"none","cost_mode":"manual","cost_usd":17}),
        ),
        (
            format!("/v1/vehicles/{}/charging-networks/Sparse", id(10)),
            json!({"cost_mode":"free"}),
        ),
    ] {
        rows.push(json!({"case":"annotation failure partial commit","response":request(&state,"PATCH",uri,Some(1),Some(payload.to_string())).await,"persisted":persisted(&fixture.pool).await}));
    }
    sqlx::raw_sql("DROP TRIGGER synthetic_reject_annotation ON panda.charge_session_user_annotations; DROP FUNCTION panda.synthetic_reject_annotation();").execute(&fixture.pool).await.unwrap();
    rows.push(json!({"case":"retry partial failure","response":request(&state,"PATCH",uri.clone(),Some(1),Some("{}".into())).await,"persisted":persisted(&fixture.pool).await}));
    let denied = sqlx::query("DELETE FROM timeseries.telemetry WHERE vehicle_id=$1")
        .bind(id(10))
        .execute(&fixture.pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    rows.push(
        json!({"history_delete_sqlstate":"42501","persisted":persisted(&fixture.pool).await}),
    );
    let mut actual = json!(rows);
    if let Ok(dir) = std::env::var("CHARGING_CONTRACT_ARTIFACT_DIR") {
        std::fs::write(
            format!("{dir}/http-raw.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }
    normalize(&mut actual);
    drop(state);
    fixture.cleanup().await;
    snapshot("http", actual, include_str!("charging-http-expected.json"));
}

#[test]
fn charging_workflow_pure_boundary_contract() {
    let mut rows = vec![];
    for raw in [
        -1.0,
        0.0,
        39.999,
        40.0,
        300.0,
        300.001,
        1000.0,
        1000.001,
        40000.0,
        300000.0,
        300001.0,
        f64::NAN,
        f64::INFINITY,
    ] {
        rows.push(json!({"capacity_bits":format!("{:016x}",raw.to_bits()),"result":normalize_capacity_kwh(raw)}));
    }
    for kind in [
        None,
        Some("dc"),
        Some(" DCFC "),
        Some("ac_l2"),
        Some("other"),
    ] {
        for vendor in [None, Some(" TESLA "), Some("Synthetic")] {
            for power in [
                None,
                Some(-1.0),
                Some(0.0),
                Some(19.999),
                Some(20.0),
                Some(300.0),
                Some(f64::NAN),
            ] {
                rows.push(json!({"kind":kind,"vendor":vendor,"power":power,"result":normalize_chart_charger_type(kind,vendor,power)}));
            }
        }
    }
    let start = time("2026-01-01T00:00:00Z");
    for count in 0..=3 {
        for end_seconds in [-60, 0, 60, 600] {
            for cap in [22.0, 300.0] {
                let points = (0..count)
                    .map(|i| RawCurvePointRow {
                        ts: start + chrono::Duration::seconds(i * 120 - 60),
                        power_kw: Some(if i == 0 { -1.0 } else { 350.0 }),
                        sample_source: "synthetic".into(),
                    })
                    .collect::<Vec<_>>();
                rows.push(json!({"count":count,"end_seconds":end_seconds,"cap":cap,"result":build_fallback_curve_samples(id(100),start,start+chrono::Duration::seconds(end_seconds),Some("dc"),20.0,80.0,&points,cap)}));
            }
        }
    }

    for reverse in [false, true] {
        for energies in [
            [None, Some(-2.0), Some(5.0)],
            [Some(1e16), Some(1.0), Some(1.0)],
            [Some(0.0), Some(-0.0), Some(2.0)],
        ] {
            let mut source = (0..3)
                .map(|i| ChartSeriesSessionSourceRow {
                    session_id: id(100 + i as u128),
                    started_at: start + chrono::Duration::seconds(i as i64),
                    session_day_local: if i == 1 {
                        Some("2025-12-31".into())
                    } else {
                        None
                    },
                    energy_kwh: energies[i],
                    cost_usd: None,
                    location_name: None,
                    charger_type: None,
                    network_vendor: None,
                    peak_power_kw: None,
                })
                .collect::<Vec<_>>();
            if reverse {
                source.reverse();
            }
            let (daily, sessions) = build_charge_chart_series(source);
            rows.push(json!({"reverse":reverse,"energies":energies,
                "daily":daily.iter().map(|d|json!({"day_local":d.day_local,"day_start":d.day_start,"total_energy_kwh":d.total_energy_kwh,"session_count":d.session_count})).collect::<Vec<_>>(),
                "sessions":sessions.iter().map(|d|json!({"session_id":d.session_id,"day_local":d.day_local,"day_start":d.day_start,"started_at":d.started_at,"energy_added_kwh":d.energy_added_kwh,"cost_usd":d.cost_usd,"charger_type":d.charger_type,"location_name":d.location_name})).collect::<Vec<_>>()
            }));
        }
    }
    snapshot(
        "pure",
        json!(rows),
        include_str!("charging-pure-expected.json"),
    );
}

#[tokio::test]
#[ignore = "requires isolated CHARGING_CONTRACT_DATABASE_URL"]
async fn charging_workflow_existing_ignored_authorization_tests() {
    use sqlx::ConnectOptions;
    for name in [
        "list_sessions_requires_auth",
        "session_detail_requires_auth",
        "charging_summary_requires_auth",
        "charging_chart_series_requires_auth",
        "session_detail_invalid_uuid",
    ] {
        let fixture = DatabaseFixture::new().await;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                &format!("routes::charging::tests::{name}"),
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(
                "DATABASE_URL",
                fixture.pool.connect_options().to_url_lossy().as_str(),
            )
            .output()
            .unwrap();
        if let Ok(dir) = std::env::var("CHARGING_CONTRACT_ARTIFACT_DIR") {
            std::fs::write(
                format!("{dir}/legacy-{name}.log"),
                [output.stdout.clone(), output.stderr.clone()].concat(),
            )
            .unwrap();
        }
        fixture.cleanup().await;
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed; 0 ignored"));
    }
}

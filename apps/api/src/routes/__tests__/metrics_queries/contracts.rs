use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::Request,
};
use serde_json::{json, Value};
use sqlx::{Connection, Executor, PgPool};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tower::ServiceExt;

mod fixture;
use fixture::DatabaseFixture;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
const FROM: &str = "2026-01-01T00:00:00Z";
const TO: &str = "2026-01-03T00:00:00Z";

fn compare(name: &str, actual: &Value, expected: &str) {
    let directory = std::env::var("METRICS_CONTRACT_RUN").unwrap_or_else(|_| "/review".into());
    std::fs::write(
        format!("{directory}/{name}-actual.json"),
        serde_json::to_string_pretty(actual).unwrap(),
    )
    .unwrap();
    if std::env::var_os("METRICS_CONTRACT_CAPTURE").is_none() {
        let expected: Value = serde_json::from_str(expected).unwrap();
        let actual = actual.as_array().unwrap();
        let expected = expected.as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "{name} case count");
        for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(a, b, "{name} case {index}");
        }
    }
}

#[test]
fn metrics_workflow_pure_boundaries() {
    let mut cases = Vec::new();
    for minutes in [
        -1, 0, 1, 59, 60, 61, 359, 360, 361, 1439, 1440, 1441, 10079, 10080, 10081,
    ] {
        for bucket in [
            None,
            Some("auto"),
            Some("1min"),
            Some("5min"),
            Some("15min"),
            Some("1h"),
            Some("1d"),
            Some("full"),
            Some("invalid"),
        ] {
            let from = at(FROM);
            let to = from + chrono::Duration::minutes(minutes);
            let result = resolve_bucket(bucket, from, to).map_err(|e| format!("{e}"));
            cases.push(json!({"case":"bucket", "minutes":minutes,"bucket":bucket,"result":result}));
        }
    }
    for density in [None, Some("compact"), Some("full"), Some("invalid")] {
        for bucket in [None, Some("raw"), Some("invalid")] {
            for max in [
                None,
                Some(0),
                Some(1),
                Some(2),
                Some(95),
                Some(96),
                Some(97),
                Some(usize::MAX),
            ] {
                cases.push(json!({"case":"density","density":density,"bucket":bucket,"max":max,
                    "result":resolve_batch_density(density,bucket,max,at(FROM),at(TO)).map_err(|e| format!("{e}"))}));
            }
        }
    }
    for length in [0, 1, 2, 3, 95, 96, 97, 101, 200, 10001] {
        for max in [2, 3, 96] {
            let points = (0..length)
                .map(|n| MetricSeriesPoint {
                    ts: at(FROM) + chrono::Duration::seconds(n),
                    value: if n % 3 == 0 { None } else { Some(n as f64) },
                })
                .collect();
            cases.push(json!({"case":"sampling","length":length,"max":max,"points":cap_metric_points(points,max)}));
        }
    }
    for lifetime in [false, true] {
        cases.push(json!({"case":"bounds","lifetime":lifetime,"result":resolve_time_bounds(Some(at(TO)),Some(at(FROM)),lifetime,30)}));
    }
    for name in [
        "total_trips".to_owned(),
        "unknown".into(),
        "".into(),
        "x".repeat(128),
        "x".repeat(129),
        "total_trips\n".into(),
    ] {
        cases.push(json!({"case":"lookup","name":name,"result":find_metric(&name).map(|m|m.id).map_err(|e|format!("{e}")),"series":is_series_metric(&name)}));
    }
    compare("pure", &json!(cases), include_str!("pure.json"));
}

async fn seed(pool: &PgPool) {
    for n in 1..=4 {
        sqlx::query("INSERT INTO panda.users(id,email,password_hash,created_at,updated_at) VALUES($1,$2,'synthetic inert hash','2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(format!("synthetic-metrics-{n}@example.invalid")).execute(pool).await.unwrap();
    }
    for n in 10..=12 {
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES($1,$2,$3,'synthetic','2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(if n==12 {4} else {1})).bind(format!("synthetic-metrics-{n}")).execute(pool).await.unwrap();
    }
    for (n, vehicle, user, role) in [
        (20, 10, 1, "owner"),
        (21, 10, 2, "viewer"),
        (22, 10, 3, "manager"),
        (23, 11, 1, "owner"),
        (24, 12, 4, "owner"),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_memberships(id,vehicle_id,user_id,role,created_at,updated_at) VALUES($1,$2,$3,$4,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(vehicle)).bind(id(user)).bind(role).execute(pool).await.unwrap();
    }
    for (n, start, distance, efficiency, duration, energy, regen, temp) in [
        (
            100,
            "2026-01-01T00:00:00Z",
            Some(10.0),
            Some(300.0),
            Some(600),
            Some(3000.0),
            Some(500.0),
            Some(10.0),
        ),
        (
            101,
            "2026-01-01T00:01:00Z",
            Some(30.0),
            Some(400.0),
            Some(1200),
            Some(12000.0),
            None,
            Some(20.0),
        ),
        (
            102,
            "2026-01-01T06:00:00Z",
            Some(0.0),
            Some(999.0),
            Some(0),
            Some(1.0),
            Some(-2.0),
            Some(0.0),
        ),
        (
            103,
            "2026-01-02T00:00:00Z",
            Some(-1.0),
            None,
            Some(-60),
            None,
            None,
            None,
        ),
        (
            104,
            "2026-01-03T00:00:00Z",
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        (
            105,
            "2025-12-31T23:59:59Z",
            Some(8.0),
            Some(200.0),
            Some(100),
            Some(1600.0),
            None,
            Some(-10.0),
        ),
    ] {
        sqlx::query("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at,distance_miles,efficiency_wh_per_mile,duration_seconds,energy_wh,regen_wh,outside_temp_c,created_at) VALUES($1,$2,$3,$3+INTERVAL '10 minutes',$4,$5,$6,$7,$8,$9,'2026-01-01Z')")
            .bind(id(n)).bind(id(10)).bind(at(start)).bind(distance).bind(efficiency).bind(duration).bind(energy).bind(regen).bind(temp).execute(pool).await.unwrap();
    }
    for (n, kwh, wh, cost) in [
        (200, Some(12.0), Some(99000.0), Some(5.0)),
        (201, None, Some(2000.0), Some(0.0)),
        (202, None, None, None),
    ] {
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,kwh_added,energy_added_wh,cost_usd,created_at) VALUES($1,$2,$3,$3+INTERVAL '1 hour',$4,$5,$6,'2026-01-01Z')")
            .bind(id(n)).bind(id(10)).bind(at(FROM)+chrono::Duration::hours(n as i64-200)).bind(kwh).bind(wh).bind(cost).execute(pool).await.unwrap();
    }
    for n in 0..=100 {
        sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,battery_level,distance_to_empty_mi,odometer_miles,outside_temp_c,speed_mph,power_kw,tire_fl_psi,tire_fr_psi,tire_rl_psi,tire_rr_psi,power_state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,40,NULL,42,43,'ready')")
            .bind(at(FROM)+chrono::Duration::minutes(n)).bind(id(10)).bind(if n%4==0 {None} else {Some(80.0-n as f64/10.0)})
            .bind(200.0-n as f64).bind(1000.0+n as f64).bind(-10.0+n as f64).bind(n as f64).bind(-50.0+n as f64).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,battery_level) VALUES('2026-01-04Z',$1,99),('2026-01-01Z',$2,1)").bind(id(10)).bind(id(12)).execute(pool).await.unwrap();
    for (n, state, start, end) in [
        (300, "sleep", FROM, Some("2026-01-01T00:30:00Z")),
        (
            301,
            "drive",
            "2026-01-01T01:00:00Z",
            Some("2026-01-01T01:30:00Z"),
        ),
        (302, "ready", TO, None),
        (303, "unknown", "2025-12-31T00:00:00Z", Some(TO)),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_state_periods(id,vehicle_id,state,started_at,ended_at) VALUES($1,$2,$3,$4,$5)")
            .bind(n as i64).bind(id(10)).bind(state).bind(at(start)).bind(end.map(at)).execute(pool).await.unwrap();
    }
    for (n, window) in [
        (400, "24h"),
        (401, "8h"),
        (402, "since_parked"),
        (403, "24h"),
    ] {
        sqlx::query("INSERT INTO timeseries.parallax_parked_energy_samples(vehicle_id,source_at,received_at,payload_hash,period_window,duration_minutes,total_kwh,climate_kwh,total_range_impact_km) VALUES($1,$2,$2,$3,$4,$5,$6,$7,$8)")
            .bind(id(10)).bind(at(FROM)+chrono::Duration::minutes(n-400)).bind(vec![n as u8]).bind(window).bind(if n==402 {None} else {Some(30i32)})
            .bind(if n==402 {None} else {Some(1.25)}).bind(Some(-0.1)).bind(Some(5.0)).execute(pool).await.unwrap();
    }
    for (n, vehicle) in [(500, 10), (501, 10), (502, 12)] {
        sqlx::query("INSERT INTO panda.trip_tags(id,vehicle_id,name,normalized_name,created_by,created_at,updated_at) VALUES($1,$2,$3,$3,$4,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(vehicle)).bind(format!("tag {n}")).bind(id(1)).execute(pool).await.unwrap();
    }
    for (trip, tag) in [(100, 500), (101, 500), (101, 501)] {
        sqlx::query("INSERT INTO panda.trip_tag_assignments(trip_id,tag_id,assigned_by,assigned_at) VALUES($1,$2,$3,'2026-01-01Z')").bind(id(trip)).bind(id(tag)).bind(id(1)).execute(pool).await.unwrap();
    }
}

async fn state(pool: &PgPool) -> AppState {
    let keys = crate::keys::bootstrap_development_keys(pool).await.unwrap();
    let mut config: crate::config::Config=serde_json::from_value(json!({"database_url":"postgres://synthetic.invalid/unused", "redis_url":"redis://127.0.0.1:1", "panda_env":"production"})).unwrap();
    config.rate_limit.heavy_read_burst = 10000;
    config.rate_limit.auth_read_burst = 10000;
    config.rate_limit.auth_metadata_burst = 10000;
    AppState {
        pool: pool.clone(),
        redis: redis::Client::open("redis://127.0.0.1:1").unwrap(),
        jwt_keys: Arc::new(
            crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem)
                .unwrap(),
        ),
        age_key: keys.age_key,
        config,
        resources: Default::default(),
        nominatim_cache: Default::default(),
        supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
    }
}

struct TestApp {
    router: axum::Router,
    pool: PgPool,
    isolate_statements: bool,
}

async fn request(
    app: &TestApp,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<String>,
) -> Value {
    if app.isolate_statements {
        app.pool
            .acquire()
            .await
            .unwrap()
            .clear_cached_statements()
            .await
            .unwrap();
    }
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    let mut request = request.body(Body::from(body.unwrap_or_default())).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 12345))));
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        let value = value.to_str().unwrap();
        headers.insert(
            name.to_string(),
            if name == "x-request-id" {
                json!("<request id>")
            } else if name.as_str().contains("ratelimit")
                && !["x-panda-ratelimit-class", "x-panda-ratelimit-source"].contains(&name.as_str())
            {
                json!("<rate clock/count>")
            } else {
                json!(value)
            },
        );
    }
    match to_bytes(response.into_body(), 16 * 1024 * 1024).await {
        Ok(bytes) => {
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            let mut body: Value =
                serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw":text}));
            let volatile = body.get("generated_at").is_some();
            if volatile {
                let generated = body["generated_at"]
                    .as_str()
                    .unwrap()
                    .parse::<DateTime<Utc>>()
                    .unwrap();
                assert!((Utc::now() - generated).num_seconds().abs() < 60);
                if let Some(length) = headers.get("content-length") {
                    assert_eq!(
                        length.as_str().unwrap().parse::<usize>().unwrap(),
                        text.len()
                    );
                }
                body["generated_at"] = json!("<response clock>");
                if let Some(today) = body.get_mut("today") {
                    today["date"] = json!("<UTC date>");
                }
                if headers.contains_key("content-length") {
                    headers.insert("content-length".into(), json!("<clock dependent length>"));
                }
            }
            json!({"status":status,"headers":headers,"body":body,"wire":if volatile {Value::Null} else {json!(text)}})
        }
        Err(error) => json!({"status":status,"headers":headers,"body_error":error.to_string()}),
    }
}

async fn persisted(pool: &PgPool) -> Value {
    let mut all = serde_json::Map::new();
    for (table, order) in [
        ("panda.trips", "id"),
        ("panda.charge_sessions", "id"),
        ("timeseries.telemetry", "vehicle_id,ts"),
        ("panda.vehicle_state_periods", "id"),
        (
            "timeseries.parallax_parked_energy_samples",
            "vehicle_id,source_at,period_window",
        ),
        ("panda.trip_tags", "id"),
        ("panda.trip_tag_assignments", "trip_id,tag_id"),
        ("panda.vehicle_memberships", "id"),
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM {table} t ORDER BY {order}"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        all.insert(table.into(), json!(rows));
    }
    Value::Object(all)
}

#[tokio::test]
#[ignore = "requires the unique disposable metrics Docker database; execute explicitly"]
async fn metrics_workflow_http_database_contract() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let fixture = DatabaseFixture::with_connections(1).await;
    seed(&fixture.pool).await;
    let state = state(&fixture.pool).await;
    let tokens = (1..=4)
        .map(|n| {
            crate::middleware::auth::issue_access_token(id(n), Some(id(10)), &state.jwt_keys)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let app = TestApp {
        router: crate::routes::build_router(state.clone()),
        pool: fixture.pool.clone(),
        isolate_statements: true,
    };
    let before = persisted(&fixture.pool).await;
    let mut cases = Vec::new();
    let privileges:Value=sqlx::query_scalar("SELECT jsonb_build_object('superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,'delete_history',has_table_privilege(current_user,'panda.trips','DELETE'),'truncate_history',has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE')) FROM pg_roles WHERE rolname=current_user").fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(
        privileges,
        json!({"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"delete_history":false,"truncate_history":false})
    );
    cases.push(json!({"case":"privileges","output":privileges}));
    cases.push(json!({"case":"catalog","output":request(&app,Some(&tokens[0]),"GET","/v1/metrics/catalog",None).await}));
    for vehicle in [10, 11] {
        for metric in METRICS {
            for endpoint in ["value", "series"] {
                for bucket in if endpoint == "series" {
                    vec!["raw", "minute", "5min", "15min", "hour", "day"]
                } else {
                    vec!["day"]
                } {
                    let path=format!("/v1/metrics/{endpoint}?vehicle_id={}&metric={}&from={FROM}&to={TO}&bucket={bucket}",id(vehicle),metric.id);
                    cases.push(json!({"case":path,"output":request(&app,Some(&tokens[0]),"GET",&path,None).await}));
                }
            }
        }
    }
    for extra in [
        json!({}),
        json!({"density":"full"}),
        json!({"max_points":2,"bucket":"raw"}),
        json!({"tag_ids":id(500).to_string(),"density":"full"}),
        json!({"tag_ids":format!("{},{}",id(500),id(501)),"tag_match":"all"}),
        json!({"untagged":true}),
        json!({"lifetime":true}),
    ] {
        let mut body = json!({"vehicle_id":id(10),"from":FROM,"to":TO,"metrics":METRICS.iter().map(|m|json!({"metric":m.id})).collect::<Vec<_>>()});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        cases.push(json!({"case":"batch","input":body,"output":request(&app,Some(&tokens[0]),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
    }
    for metric in [
        "total_miles",
        "trip_miles",
        "total_trips",
        "avg_efficiency",
        "avg_gross_efficiency",
        "avg_outside_temp_c",
        "avg_trip_duration",
    ] {
        for density in ["compact", "full"] {
            let body = json!({"vehicle_id":id(10),"from":FROM,"to":TO,"density":density,"tag_ids":id(500).to_string(),"metrics":[{"metric":metric}]});
            cases.push(json!({"case":"isolated filtered measure","input":body,"output":request(&app,Some(&tokens[0]),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
        }
    }
    for maximum in [2, 96] {
        let body = json!({"vehicle_id":id(10),"from":FROM,"to":TO,"bucket":"raw","max_points":maximum,"metrics":METRICS.iter().filter(|m| m.id!="avg_efficiency").map(|m|json!({"metric":m.id})).collect::<Vec<_>>()});
        cases.push(json!({"case":"compact complete outputs","input":body,"output":request(&app,Some(&tokens[0]),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
    }
    for metrics in [
        json!([]),
        json!([{"metric":"total_trips","include_latest":false,"include_series":false}]),
        json!([{"metric":"battery_level","include_latest":false},{"metric":"total_trips"},{"metric":"battery_level","include_series":false}]),
        json!([{"metric":"unknown"}]),
        json!([{"metric":"total_trips\n"}]),
        json!((0..33)
            .map(|_| json!({"metric":"total_trips"}))
            .collect::<Vec<_>>()),
    ] {
        let body = json!({"vehicle_id":id(10),"metrics":metrics,"from":FROM,"to":TO});
        cases.push(json!({"case":"batch selection","input":body,"output":request(&app,Some(&tokens[0]),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
    }
    for extra in [
        json!({"density":"bad","bucket":"bad"}),
        json!({"bucket":"bad"}),
        json!({"tag_ids":"bad"}),
        json!({"tag_ids":id(502).to_string()}),
        json!({"tag_ids":id(999).to_string()}),
        json!({"from":TO,"to":FROM}),
        json!({"density":"full","bucket":"bad"}),
    ] {
        let mut body =
            json!({"vehicle_id":id(10),"metrics":[{"metric":"total_trips"}],"from":FROM,"to":TO});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        cases.push(json!({"case":"batch validation","input":body,"output":request(&app,Some(&tokens[0]),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
    }
    for body in [
        "{".to_owned(),
        "{}".into(),
        format!("{{\"unused\":\"{}\"}}", "x".repeat(65536)),
    ] {
        cases.push(json!({"case":"malformed/body limit","length":body.len(),"output":request(&app,Some(&tokens[0]),"POST","/v1/metrics/batch",Some(body)).await}));
    }
    for vehicle in [10, 11, 12, 999] {
        for token in [
            None,
            Some(tokens[0].as_str()),
            Some(tokens[1].as_str()),
            Some(tokens[2].as_str()),
            Some(tokens[3].as_str()),
        ] {
            for suffix in [
                format!(
                    "metrics/value?vehicle_id={}&metric=total_trips&from={FROM}&to={TO}",
                    id(vehicle)
                ),
                format!("dashboard/overview/{}", id(vehicle)),
                format!(
                    "vehicles/{}/state-timeline?from={FROM}&to={TO}",
                    id(vehicle)
                ),
                format!("vehicles/{}/parked-energy?from={FROM}&to={TO}", id(vehicle)),
            ] {
                cases.push(json!({"case":"authorization","vehicle":vehicle,"user":token.and_then(|t| tokens.iter().position(|v|v==t)).map(|i|i+1),"path":suffix,"output":request(&app,token,"GET",&format!("/v1/{suffix}"),None).await}));
            }
        }
    }
    for suffix in [
        "metrics/value?metric=unknown".to_owned(),
        format!("metrics/value?vehicle_id={}&metric=unknown", id(12)),
        format!(
            "metrics/series?vehicle_id={}&metric=total_trips&bucket=bad",
            id(10)
        ),
        format!(
            "metrics/value?vehicle_id={}&metric=total_trips&from=bad",
            id(10)
        ),
        format!(
            "metrics/value?vehicle_id={}&metric=total_trips&lifetime=true&to={TO}",
            id(10)
        ),
        format!("metrics/value?vehicle_id={}&metric=battery_level", id(10)),
    ] {
        cases.push(json!({"case":suffix,"output":request(&app,Some(&tokens[0]),"GET",&format!("/v1/{suffix}"),None).await}));
    }
    for limit in ["-1", "0", "1", "2", "500", "2000", "2001", "bad"] {
        let path = format!(
            "/v1/vehicles/{}/state-timeline?from={FROM}&to={TO}&limit={limit}",
            id(10)
        );
        cases.push(
            json!({"case":path,"output":request(&app,Some(&tokens[0]),"GET",&path,None).await}),
        );
    }
    for endpoint in ["state-timeline", "parked-energy"] {
        for query in [
            String::new(),
            "from=bad".into(),
            format!("from={TO}&to={FROM}"),
            format!("from={TO}&to={TO}"),
        ] {
            let path = format!("/v1/vehicles/{}/{endpoint}?{query}", id(10));
            cases.push(
                json!({"case":path,"output":request(&app,Some(&tokens[0]),"GET",&path,None).await}),
            );
        }
    }
    let key = "panda_synthetic_metrics_local_only";
    sqlx::query("INSERT INTO panda.api_keys(id,user_id,vehicle_id,key_hash,access_level,name,created_at,updated_at,last_used_at) VALUES($1,$2,$3,$4,'read','synthetic local-only','2026-01-01Z','2026-01-01Z',now())")
        .bind(id(600)).bind(id(1)).bind(id(10)).bind(crate::routes::api_keys::hash_api_key(key).as_slice()).execute(&fixture.pool).await.unwrap();
    for vehicle in [10, 11, 12] {
        for suffix in [
            format!(
                "metrics/value?vehicle_id={}&metric=total_trips&from={FROM}&to={TO}",
                id(vehicle)
            ),
            format!("dashboard/overview/{}", id(vehicle)),
            format!(
                "vehicles/{}/state-timeline?from={FROM}&to={TO}",
                id(vehicle)
            ),
            format!("vehicles/{}/parked-energy?from={FROM}&to={TO}", id(vehicle)),
        ] {
            cases.push(json!({"case":"API vehicle scope","path":suffix,"output":request(&app,Some(key),"GET",&format!("/v1/{suffix}"),None).await}));
        }
        let body = json!({"vehicle_id":id(vehicle),"metrics":[{"metric":"total_trips"}],"from":FROM,"to":TO});
        cases.push(json!({"case":"API batch vehicle scope","vehicle":vehicle,"output":request(&app,Some(key),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
    }
    let denied = sqlx::query("DELETE FROM panda.trips WHERE id=$1")
        .bind(id(100))
        .execute(&fixture.pool)
        .await
        .unwrap_err();
    cases.push(json!({"case":"history delete denied","code":denied.as_database_error().unwrap().code(),"message":denied.as_database_error().unwrap().message()}));
    let after = persisted(&fixture.pool).await;
    assert_eq!(
        before, after,
        "read routes and denied history deletion preserve all evidence"
    );
    cases.push(json!({"case":"persistence before and after","before":before,"after":after}));
    fixture.cleanup().await;
    compare("http", &json!(cases), include_str!("http.json"));
}

#[tokio::test]
#[ignore = "requires the unique disposable metrics Docker database; execute explicitly"]
async fn metrics_workflow_stream_interruption_contract() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let fixture = DatabaseFixture::new().await;
    seed(&fixture.pool).await;
    let mut state = state(&fixture.pool).await;
    state.config.security.metrics_timeout_seconds = 1;
    state.config.security.metrics_max_per_user = 1;
    let token =
        crate::middleware::auth::issue_access_token(id(1), Some(id(10)), &state.jwt_keys).unwrap();
    let app = TestApp {
        router: crate::routes::build_router(state.clone()),
        pool: fixture.pool.clone(),
        isolate_statements: false,
    };
    let before = persisted(&fixture.pool).await;
    let mut lock = fixture.pool.begin().await.unwrap();
    lock.execute("LOCK TABLE timeseries.telemetry IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let mut cases = Vec::new();
    for endpoint in ["value", "series"] {
        let path = format!(
            "/v1/metrics/{endpoint}?vehicle_id={}&metric=battery_level&from={FROM}&to={TO}",
            id(10)
        );
        let output = request(&app, Some(&token), "GET", &path, None).await;
        if endpoint == "value" {
            let variants: Vec<Value> =
                serde_json::from_str(include_str!("deadline-variants.json")).unwrap();
            assert!(
                variants.contains(&output),
                "deadline response outside both observed baseline races: {output}"
            );
            cases.push(json!({"case":"blocked query deadline","endpoint":endpoint,"observed_response_variants":variants}));
        } else {
            cases
                .push(json!({"case":"blocked query deadline","endpoint":endpoint,"output":output}));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let path = format!(
        "/v1/metrics/series?vehicle_id={}&metric=battery_level&from={FROM}&to={TO}",
        id(10)
    );
    let mut req = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 12345))));
    let response = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), 200);
    drop(response);
    let mut released = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if let Ok(permit) = state.resources.heavy(id(1), &state.config.security) {
            drop(permit);
            released = true;
            break;
        }
    }
    assert!(
        released,
        "disconnect returns admission after cancellation drains"
    );
    lock.rollback().await.unwrap();
    assert_eq!(before, persisted(&fixture.pool).await);
    cases.push(
        json!({"case":"client disconnect","admission_released":released,"evidence_unchanged":true}),
    );
    fixture.cleanup().await;
    compare("stream", &json!(cases), include_str!("stream.json"));
}

#[tokio::test]
#[ignore = "requires the unique disposable metrics Docker database; execute explicitly"]
async fn metrics_workflow_large_cursor_contract() {
    let fixture = DatabaseFixture::with_connections(1).await;
    seed(&fixture.pool).await;
    sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,power_kw) SELECT '2026-01-02Z'::timestamptz+make_interval(secs => n::float8),$1,n::float8-10000 FROM generate_series(0,20000) n").bind(id(10)).execute(&fixture.pool).await.unwrap();
    let state = state(&fixture.pool).await;
    let token =
        crate::middleware::auth::issue_access_token(id(1), Some(id(10)), &state.jwt_keys).unwrap();
    let app = TestApp {
        router: crate::routes::build_router(state),
        pool: fixture.pool.clone(),
        isolate_statements: true,
    };
    let before:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(t) ORDER BY ts) FROM (SELECT ts,power_kw FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts >= '2026-01-02Z') t").bind(id(10)).fetch_one(&fixture.pool).await.unwrap();
    let mut cases = Vec::new();
    let path=format!("/v1/metrics/series?vehicle_id={}&metric=power_kw&bucket=raw&from=2026-01-02T00:00:00Z&to={TO}",id(10));
    let output = request(&app, Some(&token), "GET", &path, None).await;
    assert_eq!(output["body"].as_array().unwrap().len(), 20001);
    assert!(output["wire"].as_str().unwrap().len() > 64 * 1024);
    cases.push(json!({"case":"multiple cursor fetches and output flushes","output":output}));
    for max in [2, 96] {
        let body = json!({"vehicle_id":id(10),"metrics":[{"metric":"power_kw"}],"from":"2026-01-02T00:00:00Z","to":TO,"bucket":"raw","max_points":max});
        cases.push(json!({"case":"cursor sampling across fetch boundaries","max":max,"output":request(&app,Some(&token),"POST","/v1/metrics/batch",Some(body.to_string())).await}));
    }
    let after:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(t) ORDER BY ts) FROM (SELECT ts,power_kw FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts >= '2026-01-02Z') t").bind(id(10)).fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(before, after);
    cases.push(json!({"case":"large persisted input unchanged","before":before,"after":after}));
    fixture.cleanup().await;
    compare("large", &json!(cases), include_str!("large.json"));
}

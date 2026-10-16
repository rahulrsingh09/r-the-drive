use crate::middleware::auth::{AppState, Claims, JwtKeys};
use axum::{
    body::{to_bytes, Body},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use http::Request;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
#[path = "energy_fixture.rs"]
mod fixture;
use fixture::Fixture;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn time(hours: i64) -> DateTime<Utc> {
    "2026-05-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::hours(hours)
}

async fn seed(pool: &PgPool) {
    for user in 1..=4 {
        sqlx::query("INSERT INTO panda.users(id,email,role,created_at,updated_at) VALUES ($1,$2,'user',$3,$3)")
            .bind(id(user)).bind(format!("energy-{user}@example.invalid")).bind(time(0)).execute(pool).await.unwrap();
    }
    for (v, user, capacity) in [
        (10, 1, Some(131000.0)),
        (11, 1, None),
        (12, 1, Some(10000.0)),
        (13, 4, Some(150000.0)),
    ] {
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,battery_capacity_wh,created_at,updated_at) VALUES ($1,$2,$3,'R1T',$4,$5,$5)")
            .bind(id(v)).bind(id(user)).bind(format!("synthetic-energy-{v}")).bind(capacity).bind(time(0)).execute(pool).await.unwrap();
        for (member, role) in [(user, "owner"), (2, "viewer"), (3, "manager")] {
            if v == 13 && member != 4 {
                continue;
            }
            sqlx::query("INSERT INTO panda.vehicle_memberships(id,vehicle_id,user_id,role,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$5)")
                .bind(id(v*100+member)).bind(id(v)).bind(id(member)).bind(role).bind(time(0)).execute(pool).await.unwrap();
        }
    }
    for vehicle in [10, 12, 13] {
        for (n, start, end, distance, efficiency, temp, mode) in [
            (0, 0, 1, 10.0, Some(300.0), Some(-5.0), Some("conserve")),
            (1, 13, 14, 30.0, Some(400.0), Some(0.0), Some("sport")),
            (2, 33, 34, 1.0, Some(450.0), Some(10.0), Some("sport")),
            (3, 100, 101, 5.0, None, Some(20.0), None),
            (4, 108, 109, 0.0, Some(600.0), Some(30.0), Some("sport")),
            (5, 117, 118, -1.0, Some(600.0), None, None),
            (
                6,
                125,
                126,
                20.0,
                Some(250.0),
                Some(15.555555555555555),
                Some("conserve"),
            ),
            (
                7,
                130,
                131,
                2.0,
                Some(500.0),
                Some(-17.77777777777778),
                None,
            ),
        ] {
            sqlx::query("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at,distance_miles,efficiency_wh_per_mile,outside_temp_c,drive_mode,duration_seconds,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
                .bind(id(vehicle*100+n)).bind(id(vehicle)).bind(time(start)).bind(time(end)).bind(distance).bind(efficiency).bind(temp).bind(mode).bind(if n==2 {0} else {3600}).bind(time(0)).execute(pool).await.unwrap();
        }
        for (n, start, end, added, used) in [
            (0, 14, Some(15), Some(10.0), Some(12000.0)),
            (1, 102, None, Some(0.01), None),
            (2, 133, Some(134), None, Some(14000.0)),
        ] {
            sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,kwh_added,energy_added_wh,energy_used_wh,created_at) VALUES ($1,$2,$3,$4,$5,11000,$6,$7)")
                .bind(id(vehicle*100+50+n)).bind(id(vehicle)).bind(time(start)).bind(end.map(time)).bind(added).bind(used).bind(time(0)).execute(pool).await.unwrap();
        }
        for h in [
            1, 4, 8, 13, 15, 20, 28, 33, 34, 45, 60, 80, 99, 100, 101, 103, 106, 108, 109, 117,
            118, 125, 126, 130, 131, 133,
        ] {
            let soc = 80.0 - (h as f64) / 10.0;
            sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,battery_level,battery_capacity_wh,distance_to_empty_mi,odometer_miles,power_state,outside_temp_c,hvac_active) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(id(vehicle)).bind(time(h)).bind(if h==103 {None} else {Some(soc)})
                .bind(if h==106 {Some(10000.0)} else if h==118 {None} else {Some(130000.0-(h as f64)*10.0)})
                .bind(if h==117 {None} else if h==125 {Some(-4.0)} else {Some(soc*3.5)})
                .bind(if h>=118 {102.0} else {100.0}).bind(if h==28 {"drive"} else {"sleep"})
                .bind(if h==20 {-5.1} else {20.0}).bind(h==60).execute(pool).await.unwrap();
        }
        // Health intentionally has no upper time bound; future capacity is observable.
        sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,battery_capacity_wh) VALUES ($1,'2099-01-01',132000),($1,'2099-01-01 12:00Z',134000),($1,'2099-01-02',130000)")
            .bind(id(vehicle)).execute(pool).await.unwrap();
        for (n, start, end, state) in [
            (0, 1, 13, "sleep"),
            (1, 15, 25, "sleep"),
            (2, 25, 33, "ready"),
            (3, 34, 100, "sleep"),
            (4, 101, 108, "sleep"),
        ] {
            sqlx::query("INSERT INTO panda.vehicle_state_periods(id,vehicle_id,started_at,ended_at,state) VALUES ($1,$2,$3,$4,$5)")
                .bind((vehicle*100+n) as i64).bind(id(vehicle)).bind(time(start)).bind(time(end)).bind(state).execute(pool).await.unwrap();
        }
    }
    for (tag, vehicle, name) in [(40, 10, "Alpha"), (41, 10, "beta"), (42, 13, "Foreign")] {
        sqlx::query("INSERT INTO panda.trip_tags(id,vehicle_id,name,normalized_name,created_by,created_at,updated_at) VALUES ($1,$2,$3,lower($3),$4,$5,$5)")
            .bind(id(tag)).bind(id(vehicle)).bind(name).bind(id(if vehicle==13 {4} else {1})).bind(time(0)).execute(pool).await.unwrap();
    }
    for (trip, tag) in [(1000, 40), (1001, 40), (1001, 41), (1003, 41)] {
        sqlx::query("INSERT INTO panda.trip_tag_assignments(trip_id,tag_id,assigned_by,assigned_at) VALUES ($1,$2,$3,$4)")
            .bind(id(trip)).bind(id(tag)).bind(id(1)).bind(time(0)).execute(pool).await.unwrap();
    }
    let interval_cases: Vec<Vec<(i64, i64)>> = vec![
        vec![],
        vec![(0, 1)],
        vec![(0, 1), (1, 2), (8, 9)],
        vec![(0, 10), (2, 3), (9, 12), (20, 21)],
        vec![(0, 1), (0, 1), (8, 9), (8, 9)],
        vec![(0, 10), (2, 20), (19, 21), (40, 42)],
        vec![(0, 1), (10, 2), (20, 21)],
        vec![(-100, -90), (0, 1), (167, 168), (200, 210)],
    ];
    for (index, intervals) in interval_cases.into_iter().enumerate() {
        let vehicle = 100 + index as u128;
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES ($1,$2,$3,'R1S',$4,$4)")
            .bind(id(vehicle)).bind(id(1)).bind(format!("synthetic-interval-{index}")).bind(time(0)).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO panda.vehicle_memberships(id,vehicle_id,user_id,role,created_at,updated_at) VALUES ($1,$2,$3,'owner',$4,$4)")
            .bind(id(vehicle*100)).bind(id(vehicle)).bind(id(1)).bind(time(0)).execute(pool).await.unwrap();
        for (i, (start, end)) in intervals.into_iter().enumerate() {
            sqlx::query("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at,created_at) VALUES ($1,$2,$3,$4,$5)")
                .bind(id(vehicle*100+i as u128)).bind(id(vehicle)).bind(time(start)).bind(time(end)).bind(time(0)).execute(pool).await.unwrap();
        }
    }
}

fn state(pool: PgPool) -> AppState {
    let keys = crate::keys::generate_keys().unwrap();
    AppState {
        pool, redis: redis::Client::open("redis://127.0.0.1/").unwrap(),
        jwt_keys: std::sync::Arc::new(JwtKeys::new(&keys.jwt_private_pem,&keys.jwt_public_pem).unwrap()),
        age_key: "synthetic-energy-local-only".into(),
        config: serde_json::from_value(json!({"database_url":"postgres://synthetic.invalid", "redis_url":"redis://127.0.0.1/", "rate_limit":{"auth_read_per_minute":100000,"auth_read_burst":100000}})).unwrap(),
        nominatim_cache: Default::default(), supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(), resources: Default::default(),
    }
}
fn token(state: &AppState, user: u128) -> String {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &Claims {
            sub: id(user),
            iss: "panda.app".into(),
            iat: Utc::now().timestamp(),
            exp: Utc::now().timestamp() + 3600,
            default_vehicle_id: Some(id(10)),
            sid: None,
        },
        &state.jwt_keys.encoding,
    )
    .unwrap()
}
async fn request(app: &Router, token: Option<&str>, uri: &str, bytes: usize) -> Value {
    let mut req = Request::builder()
        .uri(uri)
        .header("x-request-id", "synthetic-energy-request");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if bytes > 0 {
        req = req.header("content-length", bytes);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(vec![b'x'; bytes])).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let mut headers = std::collections::BTreeMap::new();
    for (key, value) in response.headers() {
        let value = if key.as_str().starts_with("x-ratelimit-") {
            "<limiter>"
        } else {
            value.to_str().unwrap()
        };
        headers.insert(key.to_string(), value.to_string());
    }
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice::<Value>(&body)
        .unwrap_or_else(|_| json!(String::from_utf8(body.to_vec()).unwrap()));
    json!({"status":status,"headers":headers,"body":body})
}
async fn snapshot(pool: &PgPool) -> Value {
    let mut result = serde_json::Map::new();
    for (table, order) in [
        ("panda.users", "id"),
        ("panda.vehicles", "id"),
        ("panda.vehicle_memberships", "id"),
        ("panda.trips", "id"),
        ("panda.charge_sessions", "id"),
        ("timeseries.telemetry", "vehicle_id,ts"),
        ("panda.vehicle_state_periods", "id"),
        ("panda.trip_tags", "id"),
        ("panda.trip_tag_assignments", "trip_id,tag_id"),
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM {table} t ORDER BY {order}"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        result.insert(table.into(), json!(rows));
    }
    Value::Object(result)
}

#[tokio::test]
#[ignore = "requires isolated ENERGY_CONTRACT_TEST_DATABASE_URL; run explicitly"]
async fn energy_complete_http_database_contract() {
    let fixture = Fixture::new().await;
    seed(&fixture.pool).await;
    let state = state(fixture.pool.clone());
    let tokens = (1..=4).map(|u| token(&state, u)).collect::<Vec<_>>();
    let app = crate::routes::build_router(state);
    let before = snapshot(&fixture.pool).await;
    let mut responses = Vec::new();
    let endpoints = [
        "battery/soc",
        "battery/range",
        "battery/capacity",
        "battery/health",
        "battery/mileage",
        "battery/degradation",
        "battery/phantom-drain",
        "efficiency/summary",
        "efficiency/by-mode",
        "efficiency/by-tag",
        "efficiency/range-vs-temp",
        "efficiency/vs-temp",
        "efficiency/trend",
        "idle-drain",
    ];
    for endpoint in endpoints {
        for (label, vehicle, who, extra) in [
            ("owner", 10, Some(0), ""),
            ("empty", 11, Some(0), ""),
            ("observed-capacity", 12, Some(0), ""),
            ("viewer", 10, Some(1), ""),
            ("manager", 10, Some(2), ""),
            ("foreign", 13, Some(0), ""),
            ("unknown-vehicle", 99, Some(0), ""),
            ("anonymous", 10, None, ""),
            ("lifetime", 10, Some(0), "&lifetime=true"),
            ("reverse", 10, Some(0), "&from=2026-06-01T00:00:00Z"),
            ("malformed", 10, Some(0), "&lifetime=wrong"),
        ] {
            let path = if endpoint == "idle-drain" {
                format!("vehicles/{}/idle-drain", id(vehicle))
            } else {
                endpoint.to_string()
            };
            let bounds = if label == "reverse" {
                "to=2026-05-01T00:00:00Z"
            } else {
                "from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z"
            };
            let uri = format!("/v1/{path}?vehicle_id={}&{bounds}{extra}", id(vehicle));
            responses.push(json!({"endpoint":endpoint,"case":label,"response":request(&app,who.map(|i|tokens[i].as_str()),&uri,0).await}));
        }
        let path = if endpoint == "idle-drain" {
            format!("vehicles/{}/idle-drain", id(10))
        } else {
            endpoint.to_string()
        };
        for (label, query, bytes) in [
            ("defaults", format!("vehicle_id={}", id(11)), 0),
            ("missing-vehicle", String::new(), 0),
            ("bad-date", format!("vehicle_id={}&from=bad", id(10)), 0),
            ("body-limit", format!("vehicle_id={}", id(10)), 65537),
        ] {
            let path = if endpoint == "idle-drain" && label == "defaults" {
                format!("vehicles/{}/idle-drain", id(11))
            } else {
                path.clone()
            };
            responses.push(json!({"endpoint":endpoint,"case":label,"response":request(&app,Some(&tokens[0]),&format!("/v1/{path}?{query}"),bytes).await}));
        }
        if endpoint.starts_with("efficiency/") {
            for extra in [
                format!("tag_ids={}", id(40)),
                format!("tag_ids={},{}&tag_match=all", id(40), id(41)),
                format!("tag_ids={},{}&tag_match=any", id(40), id(41)),
                "untagged=true".into(),
                "tag_ids=nope".into(),
                format!("tag_ids={},{}", id(40), id(40)),
                format!("tag_ids={}", id(42)),
                format!("untagged=true&tag_ids={}", id(40)),
                format!(
                    "tag_ids={}",
                    (100..151)
                        .map(|x| id(x).to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            ] {
                let uri = format!(
                    "/v1/{endpoint}?vehicle_id={}&lifetime=true&to=2026-05-08T00:00:00Z&{extra}",
                    id(10)
                );
                responses.push(json!({"endpoint":endpoint,"case":extra,"response":request(&app,Some(&tokens[0]),&uri,0).await}));
            }
        }
    }
    for extra in [
        "include_excluded=true",
        "include_excluded=true&limit=1",
        "limit=0",
        "limit=-2",
        "limit=999999",
        "min_duration_hours=-1",
        "min_duration_hours=18",
        "min_duration_hours=48",
        "min_duration_hours=NaN",
        "min_duration_hours=inf",
        "min_duration_hours=bad",
        "limit=9223372036854775808",
        "include_excluded=wrong",
    ] {
        let uri = format!(
            "/v1/vehicles/{}/idle-drain?from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z&{extra}",
            id(10)
        );
        responses.push(json!({"endpoint":"idle-drain","case":extra,"response":request(&app,Some(&tokens[0]),&uri,0).await}));
    }

    for vehicle in 100..108 {
        let uri=format!("/v1/vehicles/{}/idle-drain?from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z&include_excluded=true&min_duration_hours=0",id(vehicle));
        responses.push(json!({"endpoint":"idle-drain","case":format!("intervals-{vehicle}"),"response":request(&app,Some(&tokens[0]),&uri,0).await}));
    }
    let denied = sqlx::query("DELETE FROM panda.trips WHERE id=$1")
        .bind(id(1000))
        .execute(&fixture.pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    let after = snapshot(&fixture.pool).await;
    assert_eq!(
        before, after,
        "reads and denied deletion preserve every persisted field"
    );
    let actual = json!({"responses":responses,"before":before,"after":after,"denied_delete_sqlstate":"42501"});
    if let Ok(dir) = std::env::var("ENERGY_CONTRACT_CAPTURE") {
        std::fs::write(
            format!("{dir}/energy_http.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    } else {
        assert_eq!(
            actual,
            serde_json::from_str::<Value>(include_str!("energy_http.json")).unwrap()
        );
    }
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated ENERGY_CONTRACT_TEST_DATABASE_URL; run explicitly"]
async fn energy_original_auth_tests() {
    let fixture = Fixture::new().await;
    for (module, count) in [("battery", 5), ("efficiency", 5), ("idle_drain", 1)] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                format!("routes::{module}::tests::"),
                "--ignored".into(),
                "--nocapture".into(),
            ])
            .env("DATABASE_URL", &fixture.database_url)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        println!("{stdout}");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains(&format!(
            "test result: ok. {count} passed; 0 failed; 0 ignored"
        )));
    }
    fixture.cleanup().await;
}

#[test]
fn energy_default_time_windows() {
    for days in [7, 30, 90, 180, 365, 730] {
        let before = Utc::now();
        let (from, to) = super::resolve_time_bounds(None, None, false, days);
        let after = Utc::now();
        assert!(from >= before - Duration::days(days) && from <= after - Duration::days(days));
        assert!(to >= before && to <= after);
    }
}

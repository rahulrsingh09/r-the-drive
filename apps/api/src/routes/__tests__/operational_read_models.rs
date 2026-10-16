//! Frozen HTTP, export and relational contracts on fresh restricted installations.
use crate::middleware::auth::{issue_access_token, AppState, JwtKeys};
use crate::services::ingestion_capture;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Extension, Router,
};
use serde_json::{json, Value};
use sqlx::{Executor, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

#[path = "operational_fixture.rs"]
mod fixture;
use fixture::*;

fn body(targets: Value, max: Value) -> Value {
    json!({"range":{"from":"2026-01-01T00:00:00Z","to":"2026-01-01T00:00:03Z"},
           "targets":targets,"maxDataPoints":max})
}
fn target(metric: &str, vehicle: Option<u128>) -> Value {
    json!({"target":metric,"vehicleId":vehicle.map(Uuid::from_u128)})
}

#[tokio::test]
#[ignore = "requires the dedicated fresh operational fixture; run with --include-ignored"]
async fn operational_http_database_contract() {
    let f = Fixture::new().await;
    f.seed().await;
    let before = f.rows().await;
    let mut out = serde_json::Map::new();
    for user in [0, 1, 2, 3, 4, 5, 6, 7] {
        let role = format!("user-{user}");
        for (label, path) in [
            ("health_sparse", format!("/v1/vehicles/{}/health", id(11))),
            ("health_full", format!("/v1/vehicles/{}/health", id(10))),
            ("health_foreign", format!("/v1/vehicles/{}/health", id(12))),
            ("stewardship", "/v1/admin/rivian/stewardship".into()),
            (
                "capture_status",
                format!("/v1/vehicles/{}/ingestion-diagnostics", id(10)),
            ),
            (
                "capture_export",
                format!("/v1/vehicles/{}/ingestion-diagnostics/export", id(10)),
            ),
            (
                "capture_empty",
                format!("/v1/vehicles/{}/ingestion-diagnostics/export", id(11)),
            ),
        ] {
            out.insert(
                format!("{role}/{label}"),
                f.http(user, "GET", &path, None).await,
            );
        }
        out.insert(
            format!("{role}/grafana"),
            f.http(
                user,
                "POST",
                "/v1/grafana/query",
                Some(body(json!([target("battery_level", Some(10))]), json!(2))),
            )
            .await,
        );
    }
    for (label, path, method) in [
        ("liveness", "/v1/grafana", "GET"),
        ("search", "/v1/grafana/search", "POST"),
        ("annotations", "/v1/grafana/annotations", "POST"),
        ("keys", "/v1/grafana/tag-keys", "POST"),
        ("values", "/v1/grafana/tag-values", "POST"),
    ] {
        out.insert(label.into(), f.http(1, method, path, None).await);
    }
    let cases = vec![
        ("empty", body(json!([]), Value::Null)),
        (
            "all_metrics",
            body(
                json!(super::ALLOWED_METRICS
                    .iter()
                    .map(|m| target(m, Some(10)))
                    .collect::<Vec<_>>()),
                Value::Null,
            ),
        ),
        (
            "sparse",
            body(json!([target("battery_level", Some(11))]), json!(100)),
        ),
        (
            "missing_vehicle",
            body(json!([target("battery_level", None)]), Value::Null),
        ),
        (
            "unknown_before_vehicle",
            body(
                json!([target("injected;DROP TABLE panda.users", None)]),
                Value::Null,
            ),
        ),
        (
            "foreign_before_unknown",
            body(
                json!([target("power_kw", Some(12)), target("bad", None)]),
                Value::Null,
            ),
        ),
        (
            "missing_before_unknown",
            body(
                json!([target("power_kw", None), target("bad", None)]),
                Value::Null,
            ),
        ),
        (
            "unknown_before_foreign",
            body(
                json!([target("bad", None), target("power_kw", Some(12))]),
                Value::Null,
            ),
        ),
        (
            "duplicates",
            body(
                json!([target("speed_mph", Some(10)), target("speed_mph", Some(10))]),
                json!(1),
            ),
        ),
        (
            "negative_max",
            body(json!([target("power_kw", Some(10))]), json!(-1)),
        ),
        (
            "zero_max",
            body(json!([target("regen_power_kw", Some(10))]), json!(0)),
        ),
        (
            "large_max",
            body(json!([target("outside_temp_c", Some(10))]), json!(i64::MAX)),
        ),
        (
            "33_targets",
            body(json!(vec![target("bad", None); 33]), json!(1)),
        ),
        (
            "32_targets",
            body(json!(vec![target("power_kw", Some(11)); 32]), json!(1)),
        ),
    ];
    for (name, input) in cases {
        out.insert(
            name.into(),
            f.http(1, "POST", "/v1/grafana/query", Some(input)).await,
        );
    }
    let mut reversed = body(json!([target("bad", None)]), Value::Null);
    reversed["range"]["from"] = json!("2026-01-02T00:00:00Z");
    out.insert(
        "range_before_metric".into(),
        f.http(1, "POST", "/v1/grafana/query", Some(reversed)).await,
    );
    let mut equal = body(json!([target("battery_level", Some(10))]), Value::Null);
    equal["range"]["to"] = equal["range"]["from"].clone();
    out.insert(
        "equal_bounds".into(),
        f.http(1, "POST", "/v1/grafana/query", Some(equal)).await,
    );
    for (label, targets) in [
        ("fallback", json!([target("power_kw", None)])),
        ("override", json!([target("power_kw", Some(11))])),
    ] {
        out.insert(
            label.into(),
            f.http(
                1,
                "POST",
                &format!("/v1/grafana/query?vehicle_id={}", id(10)),
                Some(body(targets, Value::Null)),
            )
            .await,
        );
    }
    for (label, raw) in [
        ("malformed", "{".to_owned()),
        ("oversized", " ".repeat(65_537)),
        ("null", "null".into()),
        (
            "bad_uuid",
            body(
                json!([{"target":"power_kw","vehicleId":"bad"}]),
                Value::Null,
            )
            .to_string(),
        ),
        ("wrong_max", body(json!([]), json!("1")).to_string()),
    ] {
        out.insert(
            label.into(),
            f.raw_http(1, "POST", "/v1/grafana/query", Some(raw)).await,
        );
    }
    out.insert(
        "bad_query_uuid".into(),
        f.http(
            1,
            "POST",
            "/v1/grafana/query?vehicle_id=bad",
            Some(body(json!([]), Value::Null)),
        )
        .await,
    );
    out.insert(
        "bad_health_uuid".into(),
        f.http(1, "GET", "/v1/vehicles/bad/health", None).await,
    );
    let mut permits = Vec::new();
    for _ in 0..f.state.config.security.metrics_max_per_user {
        permits.push(
            f.state
                .resources
                .heavy(id(1), &f.state.config.security)
                .unwrap(),
        );
    }
    out.insert(
        "admission".into(),
        f.http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([]), Value::Null)),
        )
        .await,
    );
    out.insert(
        "validation_before_admission".into(),
        f.http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([target("bad", None)]), Value::Null)),
        )
        .await,
    );
    drop(permits);
    out.insert(
        "admission_retry".into(),
        f.http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([]), Value::Null)),
        )
        .await,
    );
    // The route reads a fixed capture in pages of 1000; compare every emitted line.
    let pages = ingestion_capture::export_rows(&f.state.pool, id(20), 0, 2)
        .await
        .unwrap();
    let next = ingestion_capture::export_rows(&f.state.pool, id(20), pages.last().unwrap().0, 2)
        .await
        .unwrap();
    out.insert("capture_pages".into(), json!([pages, next]));
    out.insert(
        "capture_zero_page".into(),
        json!(ingestion_capture::export_rows(&f.state.pool, id(20), 0, 0)
            .await
            .unwrap()),
    );
    let error = ingestion_capture::export_rows(&f.state.pool, id(20), 0, -1)
        .await
        .unwrap_err();
    out.insert("capture_negative_page".into(),json!({"code":error.as_database_error().unwrap().code(),"message":error.as_database_error().unwrap().message()}));
    assert_eq!(
        before,
        f.rows().await,
        "all HTTP and export reads leave complete rows unchanged"
    );
    out.insert("state_before_and_after".into(), before);
    let mut states = Vec::new();
    for (end, stop, reason) in [
        ("2099-01-01T01:00:00Z", None, None),
        ("2000-01-01T01:00:00Z", None, None),
        (
            "2099-01-01T01:00:00Z",
            Some("2026-01-01T00:30:00Z"),
            Some("user"),
        ),
        ("2099-01-01T01:00:00Z", Some("2026-01-01T00:30:00Z"), None),
        ("2099-01-01T01:00:00Z", None, Some("user")),
    ] {
        sqlx::query("UPDATE panda.vehicle_ingestion_diagnostics SET enabled_until=$1::text::timestamptz,stopped_at=$2::text::timestamptz,stop_reason=$3 WHERE vehicle_id=$4")
            .bind(end).bind(stop).bind(reason).bind(id(10)).execute(&f.state.pool).await.unwrap();
        let before = f.rows().await;
        let status = ingestion_capture::status(&f.state.pool, id(10))
            .await
            .unwrap();
        let export = ingestion_capture::export(&f.state.pool, id(10))
            .await
            .unwrap()
            .unwrap();
        states.push(json!({"status":status,"header":export.header,"filename":export.filename,"capture_id":export.capture_id,"rows":before}));
        assert_eq!(before, f.rows().await);
    }
    out.insert("capture_states".into(), json!(states));
    let sparse = ingestion_capture::status(&f.state.pool, id(11))
        .await
        .unwrap();
    out.insert("idle_capture".into(), json!(sparse));
    // Late read failure must release admission and preserve the fixture for retry.
    f.state
        .pool
        .execute("REVOKE SELECT ON timeseries.telemetry FROM CURRENT_USER")
        .await
        .unwrap();
    out.insert(
        "grafana_storage_failure".into(),
        f.http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([target("power_kw", Some(10))]), Value::Null)),
        )
        .await,
    );
    out.insert(
        "health_storage_failure".into(),
        f.http(1, "GET", &format!("/v1/vehicles/{}/health", id(10)), None)
            .await,
    );
    f.state
        .pool
        .execute("GRANT SELECT ON timeseries.telemetry TO CURRENT_USER")
        .await
        .unwrap();
    out.insert(
        "storage_retry".into(),
        f.http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([target("power_kw", Some(11))]), Value::Null)),
        )
        .await,
    );

    let mut health_states = Vec::new();
    for (status, owner, at) in [
        ("connected", Some("in_process"), "2000-01-01Z"),
        ("connected", None, "2099-01-01Z"),
        ("disconnected", Some("in_process"), "2099-01-01Z"),
    ] {
        sqlx::query("UPDATE panda.parallax_collector_state SET status=$1,owner_kind=$2,updated_at=$3::text::timestamptz WHERE vehicle_id=$4").bind(status).bind(owner).bind(at).bind(id(10)).execute(&f.state.pool).await.unwrap();
        health_states.push(json!({"response":f.http(1,"GET",&format!("/v1/vehicles/{}/health",id(10)),None).await,"rows":f.rows().await}));
    }
    sqlx::query("INSERT INTO panda.active_trip_checkpoints(vehicle_id,snapshot,updated_at) VALUES($1,'{}','2026-01-01Z')").bind(id(10)).execute(&f.state.pool).await.unwrap();
    health_states.push(json!({"response":f.http(1,"GET",&format!("/v1/vehicles/{}/health",id(10)),None).await,"rows":f.rows().await}));
    sqlx::query("INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip,created_at) VALUES($1,$2,'{}','2026-01-01Z')").bind(id(50)).bind(id(10)).execute(&f.state.pool).await.unwrap();
    health_states.push(json!({"response":f.http(1,"GET",&format!("/v1/vehicles/{}/health",id(10)),None).await,"rows":f.rows().await}));
    sqlx::query("UPDATE panda.vehicle_runtime_state SET trip_persistence_error='synthetic pending failure',collector_heartbeat_at='2000-01-01Z' WHERE vehicle_id=$1").bind(id(10)).execute(&f.state.pool).await.unwrap();
    health_states.push(json!({"response":f.http(1,"GET",&format!("/v1/vehicles/{}/health",id(10)),None).await,"rows":f.rows().await}));
    out.insert("health_states".into(), json!(health_states));

    sqlx::query("INSERT INTO panda.vehicle_ingestion_diagnostics(vehicle_id,capture_id,enabled_by,enabled_until,started_at,updated_at) VALUES($1,$2,$3,'2000-01-01Z','2000-01-01Z','2026-01-01Z')").bind(id(11)).bind(id(22)).bind(id(1)).execute(&f.state.pool).await.unwrap();
    out.insert(
        "no_event_status".into(),
        json!(ingestion_capture::status(&f.state.pool, id(11))
            .await
            .unwrap()),
    );
    out.insert(
        "no_event_export".into(),
        f.http(
            1,
            "GET",
            &format!("/v1/vehicles/{}/ingestion-diagnostics/export", id(11)),
            None,
        )
        .await,
    );
    for (kind, fields) in [
        ("capture_started", Value::Null),
        ("capture_stopped", json!([null, false, 0])),
        ("truncated", json!({"token":"fixture","safe":null})),
        ("unrecognized", json!("scalar")),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_ingestion_capture_events(vehicle_id,capture_id,recorded_at,kind,fields) VALUES($1,$2,'2026-01-01T00:00:00.999999Z',$3,$4)").bind(id(11)).bind(id(22)).bind(kind).bind(fields).execute(&f.state.pool).await.unwrap();
    }
    let marker_before = f.rows().await;
    out.insert(
        "markers_status".into(),
        json!(ingestion_capture::status(&f.state.pool, id(11))
            .await
            .unwrap()),
    );
    out.insert(
        "markers_export".into(),
        f.http(
            1,
            "GET",
            &format!("/v1/vehicles/{}/ingestion-diagnostics/export", id(11)),
            None,
        )
        .await,
    );
    assert_eq!(marker_before, f.rows().await);
    out.insert("markers_rows".into(), marker_before);
    let actual = json!(out);
    write("operational-raw.json", &actual);
    let mut normalized = actual;
    normalize(&mut normalized);
    write("operational-actual.json", &normalized);
    f.cleanup().await;
    if std::env::var_os("OPERATIONAL_CAPTURE").is_none() {
        let mut expected =
            serde_json::from_str::<Value>(include_str!("operational_expected.json")).unwrap();
        numeric_correction::corrected_outputs(&mut expected);
        let differences: Vec<_> = normalized
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, value)| expected.get(*key) != Some(*value))
            .map(|(key, _)| key.clone())
            .collect();
        assert!(
            differences.is_empty(),
            "Complete output mismatch; inspect raw artifacts for cases: {differences:?}"
        );
        assert_eq!(
            normalized.as_object().unwrap().len(),
            expected.as_object().unwrap().len()
        );
    }
}

fn normalize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(rows) = object
                .get_mut("panda.vehicle_memberships")
                .and_then(Value::as_array_mut)
            {
                for row in rows.iter_mut() {
                    assert!(row["id"].as_str().unwrap().parse::<Uuid>().is_ok());
                    row["id"] = json!(format!(
                        "membership:{}:{}",
                        row["vehicle_id"].as_str().unwrap(),
                        row["user_id"].as_str().unwrap()
                    ));
                }
                rows.sort_by_key(|row| row["id"].as_str().unwrap().to_owned());
            }
            if object
                .get("body")
                .is_some_and(|body| body.get("generated_at").is_some())
            {
                if let Some(headers) = object.get_mut("headers").and_then(Value::as_object_mut) {
                    if headers.contains_key("content-length") {
                        headers.insert("content-length".into(), json!("<generated_at-dependent>"));
                    }
                }
            }
            for (key, value) in object {
                if key == "generated_at" {
                    assert!(value
                        .as_str()
                        .unwrap()
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .is_ok());
                    *value = json!("<generated_at>");
                } else {
                    normalize(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(normalize),
        _ => {}
    }
}
fn write(name: &str, value: &Value) {
    let dir = std::env::var("OPERATIONAL_ARTIFACT_DIR").unwrap();
    std::fs::write(
        format!("{dir}/{name}"),
        serde_json::to_string_pretty(value).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires the dedicated fresh operational fixture; run with --include-ignored"]
async fn operational_stream_failure_and_cancellation_contract() {
    use futures::StreamExt;
    let f = Fixture::new().await;
    f.seed().await;
    let before = f.rows().await;
    let path = format!("/v1/vehicles/{}/ingestion-diagnostics/export", id(10));
    let token = issue_access_token(id(1), Some(id(10)), &f.state.jwt_keys).unwrap();
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut stream = response.into_body().into_data_stream();
    let header = stream.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&header).unwrap()["kind"],
        "header"
    );
    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(std::str::from_utf8(&first).unwrap().lines().count(), 1000);
    f.state
        .pool
        .execute("REVOKE SELECT ON panda.vehicle_ingestion_capture_events FROM CURRENT_USER")
        .await
        .unwrap();
    let error = stream.next().await.unwrap().unwrap_err();
    assert!(error.to_string().contains("error"));
    assert!(stream.next().await.is_none());
    f.state
        .pool
        .execute("GRANT SELECT ON panda.vehicle_ingestion_capture_events TO CURRENT_USER")
        .await
        .unwrap();
    let retry = f
        .http(
            1,
            "GET",
            &format!("/v1/vehicles/{}/ingestion-diagnostics/export", id(10)),
            None,
        )
        .await;
    assert_eq!(retry["status"], 200);
    assert_eq!(retry["body"].as_array().unwrap().len(), 1003);
    let mut lock = f.state.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE timeseries.telemetry IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let request = super::GrafanaQueryBody {
        range: super::GrafanaRange {
            from: "2026-01-01T00:00:00Z".parse().unwrap(),
            to: "2026-01-02T00:00:00Z".parse().unwrap(),
        },
        targets: vec![super::GrafanaTarget {
            target: "power_kw".into(),
            vehicle_id: Some(id(10)),
        }],
        max_data_points: None,
    };
    let state = f.state.clone();
    let pending = tokio::spawn(async move {
        super::query(
            axum::extract::State(state),
            crate::middleware::auth::AuthUser {
                user_id: id(1),
                default_vehicle_id: Some(id(10)),
                api_access_level: None,
                api_vehicle_id: None,
            },
            axum::extract::Query(super::VehicleIdQuery { vehicle_id: None }),
            axum::Json(request),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE usename=current_user AND wait_event_type='Lock' AND query LIKE '%timeseries.telemetry%' AND pid<>pg_backend_pid())").fetch_one(&f.state.pool).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    pending.abort();
    assert!(matches!(pending.await,Err(error) if error.is_cancelled()));
    lock.rollback().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let mut permits = Vec::new();
            for _ in 0..f.state.config.security.metrics_max_per_user {
                if let Ok(permit) = f.state.resources.heavy(id(1), &f.state.config.security) {
                    permits.push(permit);
                }
            }
            if permits.len() == f.state.config.security.metrics_max_per_user {
                break;
            }
            drop(permits);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([]), Value::Null))
        )
        .await["status"],
        200
    );
    assert_eq!(before, f.rows().await);
    write(
        "stream-fault.json",
        &json!({"first_chunk":std::str::from_utf8(&first).unwrap(),"error":error.to_string(),"retry":retry,"cancelled_query":true,"admission_recovered":true,"complete_rows_unchanged":true}),
    );
    f.cleanup().await;
}

#[path = "operational_grafana_numeric.rs"]
mod numeric_correction;

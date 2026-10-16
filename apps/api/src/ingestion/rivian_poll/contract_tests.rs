use super::*;
use serde_json::{json, Value};
use std::{cell::RefCell, collections::VecDeque};

struct ExpectedRequest {
    operation: &'static str,
    url: &'static str,
    variables: Value,
    result: Result<Value>,
}
tokio::task_local! { static REQUESTS: RefCell<VecDeque<ExpectedRequest>>; }
pub(super) fn scripted_request(
    url: &str,
    operation: &str,
    query: &str,
    variables: &Value,
) -> Option<Result<Value>> {
    REQUESTS
        .try_with(|requests| {
            let expected = requests
                .borrow_mut()
                .pop_front()
                .expect("unexpected provider request");
            assert_eq!(operation, expected.operation);
            assert_eq!(url, expected.url);
            assert_eq!(variables, &expected.variables);
            let contracts: Value =
                serde_json::from_str(include_str!("request_contracts.json")).unwrap();
            assert_eq!(
                query,
                contracts[operation].as_str().expect("known operation")
            );
            expected.result
        })
        .ok()
}
fn expected(
    operation: &'static str,
    url: &'static str,
    variables: Value,
    data: Value,
) -> ExpectedRequest {
    ExpectedRequest {
        operation,
        url,
        variables,
        result: Ok(data),
    }
}
fn tokens() -> RivianTokenBundle {
    RivianTokenBundle {
        access_token: "synthetic".into(),
        refresh_token: "synthetic".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: "synthetic-user".into(),
        csrf_token: "synthetic-csrf".into(),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn provider_contract_http_envelopes_have_typed_auth_and_no_automatic_retry() {
    use axum::{body::Body, http::StatusCode, response::Response, routing::post, Router};
    for (status, body, auth, error) in [
        (
            401,
            "secret upstream",
            true,
            Some("Rivian API: authentication required"),
        ),
        (
            403,
            "secret upstream",
            false,
            Some("Rivian API: HTTP 403 Forbidden"),
        ),
        (
            429,
            "secret upstream",
            false,
            Some("Rivian API: HTTP 429 Too Many Requests"),
        ),
        (
            500,
            "secret upstream",
            false,
            Some("Rivian API: HTTP 500 Internal Server Error"),
        ),
        (
            200,
            r#"{"data":{"ok":1},"errors":[{"message":"secret upstream","extensions":{"code":"UNAUTHENTICATED"}}]}"#,
            true,
            Some("Rivian API: authentication required"),
        ),
        (
            200,
            r#"{"data":{"ok":1},"errors":[{"message":"secret upstream"}]}"#,
            false,
            Some("Rivian GQL errors: 1 upstream errors (details withheld)"),
        ),
        (
            200,
            r#"{"data":null}"#,
            false,
            Some("Rivian API: empty data for getUserInfo"),
        ),
        (
            200,
            "not json",
            false,
            Some("failed to parse Rivian API response"),
        ),
        (200, r#"{"data":{"ok":1},"errors":[]}"#, false, None),
    ] {
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = count.clone();
        let app = Router::new().route(
            "/graphql",
            post(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Response::builder()
                        .status(StatusCode::from_u16(status).unwrap())
                        .body(Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        crate::services::outbound_policy::with_mock_gateway(url.clone(), async {
            let result = gql_request::<Value>(
                &reqwest::Client::new(),
                &url,
                &tokens(),
                "getUserInfo",
                "query getUserInfo { currentUser { id } }",
                Value::Null,
            )
            .await;
            if let Some(message) = error {
                let e = result.unwrap_err();
                assert_eq!(is_auth_error(&e), auth);
                assert_eq!(e.to_string(), message);
                assert!(!format!("{e:?}").contains("secret upstream"));
            } else {
                assert_eq!(result.unwrap(), json!({"ok":1}));
            }
        })
        .await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        server.abort();
    }
}

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
async fn provider_contract_all_read_adapters_requests_and_persistence() {
    let db = super::tests::DisposableDatabase::new().await;
    let vehicle_id = super::tests::seed_vehicle(&db.pool).await;
    let user_id: Uuid = sqlx::query_scalar("SELECT user_id FROM panda.vehicles WHERE id=$1")
        .bind(vehicle_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let requests = VecDeque::from([
        expected(
            "getUserInfo",
            GATEWAY_URL,
            Value::Null,
            json!({"currentUser":{"vehicles":[{"id":"other","vehicle":{}},{"id":"rv","vehicle":{"mobileConfiguration":{"trimOption":{"optionName":"synthetic trim"}},"vehicleState":{"supportedFeatures":[{"name":"synthetic feature","status":"enabled"}]}}}]}}),
        ),
        expected(
            "getRegisteredWallboxes",
            CHRG_URL,
            Value::Null,
            json!({"getRegisteredWallboxes":[{"wallboxId":"box","maxPower":"11.5"},{"name":"skip missing id"}]}),
        ),
        expected(
            "getCompletedSessionSummaries",
            CHRG_URL,
            Value::Null,
            json!({"getCompletedSessionSummaries":[]}),
        ),
        expected(
            "getCompletedSessionSummaries",
            CHRG_URL,
            Value::Null,
            json!({"getCompletedSessionSummaries":[]}),
        ),
        expected(
            "getLiveSessionHistory",
            CHRG_URL,
            json!({"vehicleId":"rv"}),
            json!({"getLiveSessionHistory":{"chartData":[{"kw":4,"time":"2026-10-16T00:00:00Z"},{"kw":5,"time":null}]}}),
        ),
        expected(
            "GetChargingSchedule",
            GATEWAY_URL,
            json!({"vehicleId":"rv"}),
            json!({"getVehicle":{"chargingSchedules":[{"enabled":true,"startTime":10},{"enabled":false,"startTime":20,"weekDays":["MONDAY"]}]}}),
        ),
        expected(
            "getDepartureSchedules",
            GATEWAY_URL,
            json!({"vehicleId":"rv"}),
            json!({"getDepartureSchedules":[{"id":"schedule","name":"synthetic","enabled":true,"occurrence":{"type":"daily"},"preconditioningSettings":{"defrost":true}},{"name":"skip missing id"}]}),
        ),
        expected(
            "getOTAUpdateDetails",
            GATEWAY_URL,
            json!({"vehicleId":"rv"}),
            json!({"getOTAUpdateDetails":{"releaseNotesUrl":"https://example.invalid/notes"}}),
        ),
    ]);
    REQUESTS.scope(RefCell::new(requests),async {
        let pool=&db.pool; let client=reqwest::Client::new(); let t=tokens();
        fetch_vehicle_enrichment("rv",vehicle_id,pool,&client,&t).await.unwrap();
        fetch_wallboxes(user_id,vehicle_id,pool,&client,&t).await.unwrap();
        assert_eq!(fetch_charge_history("rv",vehicle_id,pool,&client,&t).await.unwrap(),0);
        assert_eq!(fetch_charge_history_full("rv",vehicle_id,pool,&client,&t).await.unwrap(),0);
        assert_eq!(fetch_live_session_history("rv",vehicle_id,None,pool,&client,&t).await.unwrap(),1);
        fetch_charging_schedule("rv",vehicle_id,pool,&client,&t).await.unwrap();
        fetch_departure_schedules("rv",vehicle_id,pool,&client,&t).await.unwrap();
        fetch_ota_details("rv",vehicle_id,pool,&client,&t).await.unwrap();
        assert!(REQUESTS.with(|r| r.borrow().is_empty()));
        let row:(String,Value,Option<String>)=sqlx::query_as("SELECT trim,supported_features,ota_release_notes_url FROM panda.vehicles WHERE id=$1").bind(vehicle_id).fetch_one(pool).await.unwrap();
        assert_eq!(row,("synthetic trim".into(),json!(["synthetic feature"]),Some("https://example.invalid/notes".into())));
        let row:(bool,i32)=sqlx::query_as("SELECT enabled,start_time_minutes FROM panda.charging_schedules WHERE vehicle_id=$1").bind(vehicle_id).fetch_one(pool).await.unwrap(); assert_eq!(row,(false,20));
        let count:i64=sqlx::query_scalar("SELECT outbound_graphql_requests FROM panda.rivian_stewardship_counters WHERE vehicle_id=$1").bind(vehicle_id).fetch_one(pool).await.unwrap(); assert_eq!(count,8);
    }).await;
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
async fn provider_contract_auth_retry_resets_counts_and_stops_after_one_refresh() {
    use age::secrecy::ExposeSecret;
    use axum::{routing::post, Json, Router};
    let db = super::tests::DisposableDatabase::new().await;
    let id = super::tests::seed_vehicle(&db.pool).await;
    let identity = age::x25519::Identity::generate();
    let key = identity.to_string().expose_secret().to_owned();
    let t = tokens();
    sqlx::query("INSERT INTO panda.vehicle_credentials(vehicle_id,encrypted_tokens,token_created_at) VALUES($1,$2,$3)").bind(id).bind(encrypt_tokens(&t,&identity).unwrap()).bind(t.created_at).execute(&db.pool).await.unwrap();
    let app=Router::new().route("/graphql",post(|| async { Json(json!({"data":{"createCsrfToken":{"csrfToken":"fresh","appSessionToken":"fresh-app"}}})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    crate::services::outbound_policy::with_mock_gateway(url,async {
        for round in 1..=3 {
            let mut calls=0;
            let result=with_vehicle_auth_retry(id,&db.pool,&reqwest::Client::new(),&key,"synthetic",|_,_,_,_| { calls+=1; Box::pin(async { Err::<(),_>(anyhow!(AuthError)) }) }).await;
            assert!(is_auth_error(&result.unwrap_err()));
            assert_eq!(calls,2);
            let (count,state):(i32,Option<String>)=sqlx::query_as("SELECT consecutive_auth_failures,auth_state FROM panda.vehicle_runtime_state WHERE vehicle_id=$1").bind(id).fetch_one(&db.pool).await.unwrap();
            assert_eq!(count,round);
            assert_eq!(state.as_deref()==Some("needs_reauth"),round==3);
        }
        let mut calls=0;
        with_vehicle_auth_retry(id,&db.pool,&reqwest::Client::new(),&key,"synthetic",|_,_,_,_| { calls+=1; Box::pin(async { Ok(()) }) }).await.unwrap();
        assert_eq!(calls,1);
        let count:i32=sqlx::query_scalar("SELECT consecutive_auth_failures FROM panda.vehicle_runtime_state WHERE vehicle_id=$1").bind(id).fetch_one(&db.pool).await.unwrap(); assert_eq!(count,0);
        let mut calls=0;
        let error=with_vehicle_auth_retry(id,&db.pool,&reqwest::Client::new(),&key,"synthetic",|_,_,_,_| { calls+=1; Box::pin(async { Err::<(),_>(anyhow!("ordinary authentication-looking text")) }) }).await.unwrap_err();
        assert!(!is_auth_error(&error)); assert_eq!(calls,1);
    }).await;
    server.abort();
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
async fn provider_contract_startup_order_survives_stage_failure_and_forwards_baseline() {
    use age::secrecy::ExposeSecret;
    use axum::{routing::post, Json, Router};
    let db = super::tests::DisposableDatabase::new().await;
    let id = super::tests::seed_vehicle(&db.pool).await;
    let (user,rv):(Uuid,String)=sqlx::query_as("UPDATE panda.vehicles SET history_backfilled_at=now() WHERE id=$1 RETURNING user_id,rivian_vehicle_id").bind(id).fetch_one(&db.pool).await.unwrap();
    let identity = age::x25519::Identity::generate();
    let key = identity.to_string().expose_secret().to_owned();
    let t = tokens();
    sqlx::query("INSERT INTO panda.vehicle_credentials(vehicle_id,encrypted_tokens,token_created_at) VALUES($1,$2,$3)").bind(id).bind(encrypt_tokens(&t,&identity).unwrap()).bind(t.created_at).execute(&db.pool).await.unwrap();
    let requests = VecDeque::from([
        expected(
            "GetVehicleState",
            GATEWAY_URL,
            json!({"vehicleID":rv}),
            json!({"vehicleState":{"batteryLevel":{"value":61,"timeStamp":"2026-10-16T00:00:00Z"}}}),
        ),
        ExpectedRequest {
            operation: "getUserInfo",
            url: GATEWAY_URL,
            variables: Value::Null,
            result: Err(anyhow!("synthetic enrichment unavailable")),
        },
        expected(
            "getRegisteredWallboxes",
            CHRG_URL,
            Value::Null,
            json!({"getRegisteredWallboxes":[]}),
        ),
        expected(
            "getCompletedSessionSummaries",
            CHRG_URL,
            Value::Null,
            json!({"getCompletedSessionSummaries":[]}),
        ),
        expected(
            "GetChargingSchedule",
            GATEWAY_URL,
            json!({"vehicleId":rv}),
            json!({"getVehicle":null}),
        ),
    ]);
    let app=Router::new().route("/graphql",post(|| async { Json(json!({"data":{"createCsrfToken":{"csrfToken":"fresh","appSessionToken":"fresh-app"}}})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (tx, mut rx) = mpsc::channel(1);
    crate::services::outbound_policy::with_mock_gateway(
        url,
        REQUESTS.scope(RefCell::new(requests), async {
            run_startup_polls(id, user, db.pool.clone(), reqwest::Client::new(), key, tx).await;
            assert!(REQUESTS.with(|r| r.borrow().is_empty()));
        }),
    )
    .await;
    let baseline = rx.recv().await.unwrap();
    assert_eq!(baseline.message_type.as_deref(), Some("baseline"));
    assert_eq!(baseline.telemetry.unwrap().vehicle_id, id);
    assert_eq!(baseline.kind, WsInboundKind::Telemetry);
    assert!(baseline.raw.contains("61"));
    server.abort();
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
async fn provider_contract_curve_overlap_dedup_and_vehicle_isolation() {
    let db = super::tests::DisposableDatabase::new().await;
    let left = super::tests::seed_vehicle(&db.pool).await;
    let right = super::tests::seed_vehicle(&db.pool).await;
    let chart=parser::parse_charging_session_message(r#"{"type":"next","payload":{"data":{"chargingSession":{"chartData":[{"powerKW":3,"startTime":"2026-10-16T12:10:00Z"},{"powerKW":9}]}}}}"#).unwrap().unwrap();
    assert_eq!(
        persist_charging_session_chart(&db.pool, left, None, &chart.chart_data)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        persist_charging_session_chart(&db.pool, left, None, &chart.chart_data)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        persist_charging_session_chart(&db.pool, right, None, &chart.chart_data)
            .await
            .unwrap(),
        1
    );
    REQUESTS.scope(RefCell::new(VecDeque::from([expected("getLiveSessionHistory",CHRG_URL,json!({"vehicleId":"left"}),json!({"getLiveSessionHistory":{"chartData":[{"kw":1,"time":"2026-10-16T12:04:59Z"},{"kw":2,"time":"2026-10-16T12:05:00Z"},{"kw":4,"time":"2026-10-16T12:10:00Z"}]}}))])),async {
        assert_eq!(fetch_live_session_history("left",left,None,&db.pool,&reqwest::Client::new(),&tokens()).await.unwrap(),2);
    }).await;
    let left_power: Vec<f64> = sqlx::query_scalar(
        "SELECT power_kw FROM panda.rivian_charge_curve_points WHERE vehicle_id=$1 ORDER BY ts",
    )
    .bind(left)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let right_power: Vec<f64> = sqlx::query_scalar(
        "SELECT power_kw FROM panda.rivian_charge_curve_points WHERE vehicle_id=$1 ORDER BY ts",
    )
    .bind(right)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(left_power, vec![2.0, 4.0]);
    assert_eq!(right_power, vec![3.0]);
    let null_power=parser::parse_charging_session_message(r#"{"type":"next","payload":{"data":{"chargingSession":{"chartData":[{"powerKW":null,"startTime":"2026-10-16T12:10:00Z"}]}}}}"#).unwrap().unwrap();
    assert_eq!(
        persist_charging_session_chart(&db.pool, left, None, &null_power.chart_data)
            .await
            .unwrap(),
        0
    );
    db.cleanup().await;
}

#[tokio::test(start_paused = true)]
async fn provider_correction_shutdown_cancels_inflight_poll() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(3600))
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let (_power, power_state_rx) =
        tokio::sync::watch::channel(Some(crate::models::telemetry::PowerState::Ready));
    let (_charging, charging_rx) = tokio::sync::watch::channel(false);
    let (stop, shutdown) = tokio::sync::broadcast::channel(1);
    let mut task = tokio::spawn(run_poll_loop(
        Uuid::nil(),
        pool,
        reqwest::Client::new(),
        "synthetic-invalid-key".into(),
        PollLoopSignals {
            power_state_rx,
            charging_rx,
            shutdown,
        },
        redis::Client::open("redis://127.0.0.1:1/").unwrap(),
    ));
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(300)).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    stop.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), &mut task).await;
    task.abort();
    assert!(
        result.is_ok(),
        "shutdown must interrupt a poll waiting for database acquisition"
    );
}

#[test]
fn provider_correction_ota_root_matches_operation_and_accepts_legacy_spelling() {
    for root in ["getOTAUpdateDetails", "getOtaUpdateDetails"] {
        let response: OtaDetailsData = serde_json::from_value(
            json!({root:{"releaseNotesUrl":"https://example.invalid/notes"}}),
        )
        .unwrap();
        assert_eq!(
            response
                .get_ota_update_details
                .and_then(|value| value.release_notes_url)
                .as_deref(),
            Some("https://example.invalid/notes"),
            "response root {root}"
        );
    }
}

use super::*;
use sqlx::Executor;
use std::sync::{Arc, Mutex};

fn body(remote: &str) -> AddVehicleBody {
    serde_json::from_value(json!({"rivian_vehicle_id":remote,"name":" Demo R2S ","trim":"synthetic","home_lat":0,"home_lng":180})).unwrap()
}
fn account(remote: &str) -> Vec<RivianVehicleSummary> {
    vec![RivianVehicleSummary {
        id: remote.into(),
        name: Some("Provider".into()),
        vin: Some("SYNTHETIC-VIN".into()),
        model: Some("R2S".into()),
        model_year: Some(2026),
    }]
}

async fn credential_row(f: &Fixture, vehicle: Uuid) -> Value {
    let row: Value = sqlx::query_scalar(
        "SELECT to_jsonb(c) FROM panda.vehicle_credentials c WHERE vehicle_id=$1",
    )
    .bind(vehicle)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    let encrypted: Vec<u8> = sqlx::query_scalar(
        "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id=$1",
    )
    .bind(vehicle)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    let identity = age_identity(&f.state).unwrap();
    let decoded = crate::ingestion::session_store::decrypt_tokens(&encrypted, &identity).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(tokens()).unwrap()
    );
    row
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_enrollment() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    e.record(
        &f,
        "missing-session",
        result(
            add_vehicle_with_lookup(f.state.clone(), f.auth(1), body("new-remote"), async |_| {
                panic!("missing token must stop lookup")
            })
            .await,
        )
        .await,
    )
    .await;
    f.stage(1).await;
    e.record(
        &f,
        "provider-failure",
        result(
            add_vehicle_with_lookup(f.state.clone(), f.auth(1), body("new-remote"), async |_| {
                Err(AppError::RivianApi("scripted lookup failure".into()))
            })
            .await,
        )
        .await,
    )
    .await;
    e.record(
        &f,
        "forged-remote",
        result(
            add_vehicle_with_lookup(f.state.clone(), f.auth(1), body("new-remote"), async |_| {
                Ok(account("different-remote"))
            })
            .await,
        )
        .await,
    )
    .await;
    let before = f.snapshot().await;
    f.state
        .pool
        .execute("REVOKE INSERT ON panda.vehicle_runtime_state FROM CURRENT_USER")
        .await
        .unwrap();
    e.push(
        "late-failure",
        result(
            add_vehicle_with_lookup(f.state.clone(), f.auth(1), body("new-remote"), async |_| {
                Ok(account("new-remote"))
            })
            .await,
        )
        .await,
    );
    assert_eq!(before, f.snapshot().await);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.vehicle_credentials")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.state
        .pool
        .execute("GRANT INSERT ON panda.vehicle_runtime_state TO CURRENT_USER")
        .await
        .unwrap();
    let r = result(
        add_vehicle_with_lookup(
            f.state.clone(),
            f.auth(1),
            body(" new-remote "),
            async |_| Ok(account("new-remote")),
        )
        .await,
    )
    .await;
    assert_eq!(r["status"], 200);
    let vehicle: Uuid = r["body"]["vehicle_id"].as_str().unwrap().parse().unwrap();
    e.identity(vehicle, "<enrolled-vehicle>");
    e.record(&f, "retry-created", r).await;
    let original_credentials = credential_row(&f, vehicle).await;
    let run = std::env::var("VEHICLE_CONTRACT_RUN").unwrap_or_else(|_| "actual".into());
    // AGE ciphertext is compared exactly across replay, and its complete plaintext contract is checked above.
    std::fs::write(
        format!("/review/{run}-enrollment-credential-row.json"),
        serde_json::to_vec_pretty(&original_credentials).unwrap(),
    )
    .unwrap();
    let mut metadata = original_credentials.clone();
    metadata.as_object_mut().unwrap().remove("encrypted_tokens");
    e.push("credential-metadata", metadata);
    e.record(
        &f,
        "consumed-session",
        result(
            add_vehicle_with_lookup(f.state.clone(), f.auth(1), body("new-remote"), async |_| {
                panic!("consumed session")
            })
            .await,
        )
        .await,
    )
    .await;
    f.stage(1).await;
    let r = result(
        add_vehicle_with_lookup(f.state.clone(), f.auth(1), body("new-remote"), async |_| {
            Ok(account("new-remote"))
        })
        .await,
    )
    .await;
    assert_eq!(original_credentials, credential_row(&f, vehicle).await);
    e.record(&f, "same-owner-replay", r).await;
    for n in [2, 3, 4, 5] {
        f.stage(n).await;
        e.record(
            &f,
            &format!("foreign-enrollment-{n}"),
            result(
                add_vehicle_with_lookup(
                    f.state.clone(),
                    f.auth(n),
                    body("new-remote"),
                    async |_| Ok(account("new-remote")),
                )
                .await,
            )
            .await,
        )
        .await;
    }
    sqlx::query(
        "INSERT INTO panda.vehicle_memberships(vehicle_id,user_id,role) VALUES($1,$2,'manager')",
    )
    .bind(vehicle)
    .bind(id(2))
    .execute(&f.state.pool)
    .await
    .unwrap();
    e.record(
        &f,
        "manager-reenrollment",
        result(
            add_vehicle_with_lookup(f.state.clone(), f.auth(2), body("new-remote"), async |_| {
                Ok(account("new-remote"))
            })
            .await,
        )
        .await,
    )
    .await;
    assert_eq!(original_credentials, credential_row(&f, vehicle).await);
    f.stage(1).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let other = barrier.clone();
    let first = add_vehicle_with_lookup(
        f.state.clone(),
        f.auth(1),
        body("concurrent-remote"),
        async move |_| {
            barrier.wait().await;
            Ok(account("concurrent-remote"))
        },
    );
    let second = add_vehicle_with_lookup(
        f.state.clone(),
        f.auth(1),
        body("concurrent-remote"),
        async move |_| {
            other.wait().await;
            Ok(account("concurrent-remote"))
        },
    );
    let (a, b) = tokio::join!(first, second);
    let a = result(a).await;
    let b = result(b).await;
    // Both lookups complete before enrollment serialization; session cleanup is deliberately post-commit.
    assert_eq!(a["status"], 200);
    assert_eq!(b["status"], 200);
    assert_eq!(a["body"]["vehicle_id"], b["body"]["vehicle_id"]);
    e.identity(
        a["body"]["vehicle_id"].as_str().unwrap(),
        "<concurrent-vehicle>",
    );
    let mut pair = vec![a, b];
    pair.sort_by_key(Value::to_string);
    e.record(&f, "concurrent-enrollment", json!(pair)).await;
    f.cleanup().await;
    e.finish("enrollment", include_str!("enrollment.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_connect() {
    use crate::private_deployment::outbound::with_mock_gateway;
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    for mode in [
        "connected",
        "otp",
        "rejected",
        "bad-response",
        "vehicle-failure",
    ] {
        let mode = mode.to_owned();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = requests.clone();
        let behavior = mode.clone();
        let gateway=Router::new().route("/graphql",post(move |Json(body):Json<Value>| {
            let seen=seen.clone();let mode=behavior.clone();async move {
                let op=body["operationName"].as_str().unwrap().to_owned();seen.lock().unwrap().push(op.clone());
                Json(match op.as_str() {
                    "CreateCSRFToken"=>json!({"data":{"createCsrfToken":{"csrfToken":"synthetic-csrf","appSessionToken":"synthetic-app"}}}),
                    "Login" if mode=="otp"=>json!({"data":{"login":{"__typename":"MobileMFALoginResponse","otpToken":"synthetic-otp"}}}),
                    "Login" if mode=="rejected"=>json!({"errors":[{"message":"invalid credentials","extensions":{"code":"UNAUTHENTICATED"}}]}),
                    "Login" if mode=="bad-response"=>json!({"data":{}}),
                    "Login"=>json!({"data":{"login":{"__typename":"MobileLoginResponse","accessToken":"synthetic-access","refreshToken":"synthetic-refresh","userSessionToken":"synthetic-user"}}}),
                    "LoginWithOTP" if body["variables"]["otpCode"]=="rejected"=>json!({"errors":[{"message":"invalid OTP","extensions":{"code":"BAD_USER_INPUT","reason":"INVALID_OTP"}}]}),
                    "LoginWithOTP" if body["variables"]["otpCode"]=="bad"=>json!({"errors":[{"message":"invalid OTP","extensions":{"code":"INVALID_OTP"}}]}),
                    "LoginWithOTP"=>json!({"data":{"loginWithOTP":{"__typename":"MobileLoginResponse","accessToken":"synthetic-access","refreshToken":"synthetic-refresh","userSessionToken":"synthetic-user"}}}),
                    "getUserInfo" if mode=="vehicle-failure"=>json!({"errors":[{"message":"scripted failure"}]}),
                    "getUserInfo"=>json!({"data":{"currentUser":{"vehicles":[{"id":"one","name":null,"vin":null,"vehicle":{"model":"R1T","modelYear":2026}},{"id":"two","name":"Second","vehicle":{"model":"R2S"}}]}}}),
                    _=>panic!("unexpected provider request {op}"),
                })
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
        with_mock_gateway(url, async {
            let r = f
                .request(
                    Some(1),
                    "POST",
                    "/v1/vehicles/connect",
                    json!({"email":"synthetic@example.invalid","password":"synthetic-password"}),
                )
                .await;
            if let Some(challenge) = r["body"]["challenge_id"].as_str() {
                e.identity(challenge, "<otp-challenge>");
                for (n, code) in [
                    (4, "good"),
                    (1, "bad"),
                    (1, "rejected"),
                    (1, "good"),
                    (1, "good"),
                ] {
                    e.record(
                        &f,
                        &format!("otp-{n}-{code}"),
                        f.request(
                            Some(n),
                            "POST",
                            "/v1/vehicles/connect/otp",
                            json!({"challenge_id":challenge,"otp_code":code}),
                        )
                        .await,
                    )
                    .await;
                }
            }
            e.record(&f, &mode, r).await;
        })
        .await;
        e.push(
            &format!("{mode}-ordered-requests"),
            json!(requests.lock().unwrap().clone()),
        );
        server.abort();
    }
    e.record(
        &f,
        "missing-otp",
        f.request(
            Some(1),
            "POST",
            "/v1/vehicles/connect/otp",
            json!({"challenge_id":"missing","otp_code":"123456"}),
        )
        .await,
    )
    .await;
    let mut conn = f
        .state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let key = format!("rivian:connect:{}", id(1));
    let _: () = redis::AsyncCommands::del(&mut conn, &key).await.unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = entered.clone();
    let gateway = Router::new().route(
        "/graphql",
        post(move || {
            let signal = signal.clone();
            async move {
                signal.notify_one();
                std::future::pending::<Json<Value>>().await
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let before = f.snapshot().await;
    with_mock_gateway(url,async {
        let request=f.request(Some(1),"POST","/v1/vehicles/connect",json!({"email":"synthetic@example.invalid","password":"synthetic-password"}));
        tokio::pin!(request);
        tokio::select!{ biased; _=entered.notified()=>{}, _=&mut request=>panic!("scripted request must remain pending") }
    }).await;
    server.abort();
    assert_eq!(before, f.snapshot().await);
    let exists: bool = redis::AsyncCommands::exists(&mut conn, &key).await.unwrap();
    assert!(!exists);
    e.push(
        "canceled-before-login-completion",
        json!({"relational_state_unchanged":true,"connect_session_exists":exists}),
    );
    f.cleanup().await;
    e.finish("connect", include_str!("connect.json"));
}

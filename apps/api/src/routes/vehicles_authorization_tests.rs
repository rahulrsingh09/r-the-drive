use super::*;
use crate::{
    authorization_test_support::Fixture,
    ingestion::session_store::{decrypt_tokens, RivianTokenBundle},
};

async fn stage(f: &Fixture, user: Uuid) -> RivianTokenBundle {
    let tokens = RivianTokenBundle {
        access_token: format!("synthetic-{user}"),
        refresh_token: "synthetic-refresh".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: "synthetic-user".into(),
        csrf_token: "synthetic-csrf".into(),
        created_at: Utc::now(),
    };
    let mut conn = f
        .state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    store_encrypted_redis(
        &f.state,
        &mut conn,
        &format!("rivian:connect:{user}"),
        &tokens,
        60,
    )
    .await
    .unwrap();
    tokens
}

fn body(remote_id: &str) -> AddVehicleBody {
    serde_json::from_value(serde_json::json!({
        "rivian_vehicle_id": remote_id, "name": "Test vehicle"
    }))
    .unwrap()
}

fn upstream(remote_id: &str) -> Vec<RivianVehicleSummary> {
    vec![RivianVehicleSummary {
        id: remote_id.into(),
        name: Some("Test vehicle".into()),
        vin: None,
        model: Some("R1T".into()),
        model_year: Some(2026),
    }]
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_enrollment_rejects_forged_ids_and_upstream_failure_without_writes() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let owner = f.user("user").await;
    f.vehicle(owner, "demo-forged-existing").await;
    let tokens = stage(&f, user).await;
    let before = f.snapshot().await;
    for remote_id in ["demo-forged-existing", "demo-forged-new"] {
        let result = add_vehicle_with_lookup(
            f.state.clone(),
            f.auth(user),
            body(remote_id),
            async |staged: &RivianTokenBundle| {
                assert_eq!(staged.access_token, tokens.access_token);
                Ok(upstream("different-vehicle"))
            },
        )
        .await;
        assert!(matches!(result, Err(AppError::Validation(_))));
        assert_eq!(f.snapshot().await, before);
    }
    let result = add_vehicle_with_lookup(
        f.state.clone(),
        f.auth(user),
        body("demo-upstream-failure"),
        async |_| Err(AppError::RivianApi("synthetic outage".into())),
    )
    .await;
    assert!(matches!(result, Err(AppError::RivianApi(_))));
    assert_eq!(f.snapshot().await, before);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_enrollment_cannot_hijack_existing_vehicle_or_escalate_viewer() {
    let f = Fixture::new().await;
    let owner = f.user("user").await;
    let attacker = f.user("user").await;
    let vehicle_id = f.vehicle(owner, "demo-owned").await;
    sqlx::query(
        "INSERT INTO panda.vehicle_credentials (vehicle_id, encrypted_tokens, token_created_at)
         VALUES ($1, $2, now())",
    )
    .bind(vehicle_id)
    .bind(b"original-owner-credentials".as_slice())
    .execute(&f.state.pool)
    .await
    .unwrap();
    stage(&f, attacker).await;
    for viewer in [false, true] {
        if viewer {
            f.member(attacker, vehicle_id, "viewer").await;
        }
        let before = f.snapshot().await;
        let result = add_vehicle_with_lookup(
            f.state.clone(),
            f.auth(attacker),
            body("demo-owned"),
            async |_| Ok(upstream("demo-owned")),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden)));
        assert_eq!(f.snapshot().await, before);
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_verified_enrollment_and_existing_manager_reconnect_remain_usable() {
    use crate::private_deployment::outbound::with_mock_gateway;
    use axum::{http::StatusCode, routing::post};
    use serde_json::{json, Value};

    let f = Fixture::new().await;
    let owner = f.user("user").await;
    let joiner = f.user("user").await;
    let owner_token = f.token(owner);
    let joiner_token = f.token(joiner);
    let tokens = stage(&f, owner).await;
    stage(&f, joiner).await;
    let gateway = Router::new().route(
        "/graphql",
        post(
            |headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["A-Sess"], "synthetic-app");
                assert_eq!(headers["U-Sess"], "synthetic-user");
                assert_eq!(body["operationName"], "getUserInfo");
                Json(json!({"data":{"currentUser":{"vehicles":[
                    {"id":"demo-verified", "vin":"provider-vin", "vehicle":{"model":"R1S"}},
                    {"id":"demo-verified-race", "vin":"race-vin", "vehicle":{"model":"R1S"}}
                ]}}}))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    with_mock_gateway(url, async {
        let before = f.snapshot().await;
        assert_eq!(f.request("POST", "/v1/vehicles", &owner_token,
            json!({"rivian_vehicle_id":"unlisted-id", "vin":"forged"})).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(f.snapshot().await, before);

        let response = f.request("POST", "/v1/vehicles", &owner_token,
            json!({"rivian_vehicle_id":" demo-verified ", "model":"R1T", "vin":"forged-vin"})).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        let vehicle_id: Uuid = serde_json::from_value(response["vehicle_id"].clone()).unwrap();
        let metadata: (String, String) = sqlx::query_as(
            "SELECT model, vin FROM panda.vehicles WHERE id=$1")
            .bind(vehicle_id).fetch_one(&f.state.pool).await.unwrap();
        assert_eq!(metadata, ("R1S".into(), "provider-vin".into()));
        let membership: (String, bool) = sqlx::query_as(
            "SELECT role,is_default FROM panda.vehicle_memberships WHERE vehicle_id=$1 AND user_id=$2")
            .bind(vehicle_id).bind(owner).fetch_one(&f.state.pool).await.unwrap();
        assert_eq!(membership, ("owner".into(), true));
        let encrypted: Vec<u8> = sqlx::query_scalar(
            "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id=$1")
            .bind(vehicle_id).fetch_one(&f.state.pool).await.unwrap();
        assert_eq!(decrypt_tokens(&encrypted, &age_identity(&f.state).unwrap()).unwrap().access_token,
            tokens.access_token);

        let before = f.snapshot().await;
        let (first, second) = tokio::join!(
            f.request("POST", "/v1/vehicles", &joiner_token, json!({"rivian_vehicle_id":"demo-verified"})),
            f.request("POST", "/v1/vehicles", &joiner_token, json!({"rivian_vehicle_id":"demo-verified"})));
        assert_eq!(first.status(), StatusCode::FORBIDDEN);
        assert_eq!(second.status(), StatusCode::FORBIDDEN);
        assert_eq!(f.snapshot().await, before);

        f.member(joiner, vehicle_id, "manager").await;
        let credentials_before: Value = sqlx::query_scalar(
            "SELECT jsonb_build_object('credentials',to_jsonb(c),'runtime',to_jsonb(r))
             FROM panda.vehicle_credentials c JOIN panda.vehicle_runtime_state r USING(vehicle_id)
             WHERE vehicle_id=$1")
            .bind(vehicle_id).fetch_one(&f.state.pool).await.unwrap();
        let reconnected = f.request("POST", "/v1/vehicles", &joiner_token,
            json!({"rivian_vehicle_id":"demo-verified"})).await;
        assert_eq!(reconnected.status(), StatusCode::OK);
        let reconnected: Value = serde_json::from_slice(&axum::body::to_bytes(reconnected.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(reconnected["vehicle_id"], response["vehicle_id"]);
        assert_eq!(reconnected["telemetry_status"], "unchanged");
        let credentials_after: Value = sqlx::query_scalar(
            "SELECT jsonb_build_object('credentials',to_jsonb(c),'runtime',to_jsonb(r))
             FROM panda.vehicle_credentials c JOIN panda.vehicle_runtime_state r USING(vehicle_id)
             WHERE vehicle_id=$1")
            .bind(vehicle_id).fetch_one(&f.state.pool).await.unwrap();
        assert_eq!(credentials_after, credentials_before);
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT role FROM panda.vehicle_memberships WHERE vehicle_id=$1 ORDER BY role")
            .bind(vehicle_id).fetch_all(&f.state.pool).await.unwrap();
        assert_eq!(roles, vec!["manager", "owner"]);

        for user in [owner, joiner] { stage(&f, user).await; }
        let (first, second) = tokio::join!(
            f.request("POST", "/v1/vehicles", &owner_token,
                json!({"rivian_vehicle_id":"demo-verified-race", "vin":"forged-vin", "model":"R1T"})),
            f.request("POST", "/v1/vehicles", &joiner_token,
                json!({"rivian_vehicle_id":"demo-verified-race"})));
        assert!(matches!((first.status(), second.status()),
            (StatusCode::OK, StatusCode::FORBIDDEN) | (StatusCode::FORBIDDEN, StatusCode::OK)));
        let created: (Uuid, String, String) = sqlx::query_as(
            "SELECT id,model,vin FROM panda.vehicles WHERE rivian_vehicle_id='demo-verified-race'")
            .fetch_one(&f.state.pool).await.unwrap();
        assert_eq!((created.1, created.2), ("R1S".into(), "race-vin".into()));
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT role FROM panda.vehicle_memberships WHERE vehicle_id=$1 ORDER BY role")
            .bind(created.0).fetch_all(&f.state.pool).await.unwrap();
        assert_eq!(roles, vec!["owner"]);
        assert_eq!(f.request("GET", "/v1/vehicles", &owner_token, Value::Null).await.status(), StatusCode::OK);
    }).await;
    server.abort();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_enrollment_rechecks_users_after_upstream_lookup() {
    let f = Fixture::new().await;
    for delete in [false, true] {
        let user = f.user("user").await;
        stage(&f, user).await;
        let result = add_vehicle_with_lookup(
            f.state.clone(),
            f.auth(user),
            body("demo-revoked-during-lookup"),
            async |_| {
                let sql = if delete {
                    "DELETE FROM panda.users WHERE id = $1"
                } else {
                    "UPDATE panda.users SET is_disabled = TRUE WHERE id = $1"
                };
                sqlx::query(sql)
                    .bind(user)
                    .execute(&f.state.pool)
                    .await
                    .unwrap();
                Ok(upstream("demo-revoked-during-lookup"))
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::Unauthorized | AppError::Forbidden)
        ));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.vehicles")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let memberships: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.vehicle_memberships")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
        assert_eq!(memberships, 0);
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_concurrent_enrollment_creates_only_one_owner() {
    let f = Fixture::new().await;
    let first = f.user("user").await;
    let second = f.user("user").await;
    stage(&f, first).await;
    stage(&f, second).await;
    let (a, b) = tokio::join!(
        add_vehicle_with_lookup(
            f.state.clone(),
            f.auth(first),
            body("demo-race"),
            async |_| Ok(upstream("demo-race"))
        ),
        add_vehicle_with_lookup(
            f.state.clone(),
            f.auth(second),
            body("demo-race"),
            async |_| Ok(upstream("demo-race"))
        ),
    );
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(AppError::Forbidden)) | (Err(AppError::Forbidden), Ok(_))
    ));
    let owners: i64 =
        sqlx::query_scalar("SELECT count(*) FROM panda.vehicle_memberships WHERE role = 'owner'")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(owners, 1);
    let vehicles: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.vehicles")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(vehicles, 1);
    f.cleanup().await;
}

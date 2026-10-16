use super::*;
use sqlx::Executor;

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_failure_boundaries() {
    use crate::private_deployment::outbound::with_mock_gateway;
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        e.record(
            &f,
            "nonfinite-pressure",
            result(
                update_vehicle_settings_impl(
                    &f.state,
                    id(1),
                    id(101),
                    UpdateBatteryConfigBody {
                        battery_capacity_kwh: None,
                        battery_config: None,
                        target_tire_pressure_psi: Some(value),
                    },
                )
                .await,
            )
            .await,
        )
        .await;
    }
    let members = format!("/v1/vehicles/{}/members", id(101));
    f.state
        .pool
        .execute("REVOKE INSERT ON panda.vehicle_user_settings FROM CURRENT_USER")
        .await
        .unwrap();
    e.record(
        &f,
        "member-partial-failure",
        f.request(
            Some(1),
            "POST",
            &members,
            json!({"email":"recipient@example.invalid","role":"viewer"}),
        )
        .await,
    )
    .await;
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM panda.vehicle_memberships WHERE user_id=$1 AND vehicle_id=$2)"
    )
    .bind(id(6))
    .bind(id(101))
    .fetch_one(&f.state.pool)
    .await
    .unwrap());
    f.state
        .pool
        .execute("GRANT INSERT ON panda.vehicle_user_settings TO CURRENT_USER")
        .await
        .unwrap();
    e.record(
        &f,
        "member-partial-retry",
        f.request(
            Some(1),
            "POST",
            &members,
            json!({"email":"recipient@example.invalid","role":"viewer"}),
        )
        .await,
    )
    .await;
    let r = f
        .request(
            Some(1),
            "POST",
            &members,
            json!({"email":"unregistered@example.invalid","role":"viewer"}),
        )
        .await;
    let token = r["body"]["invite_token"].as_str().unwrap();
    let row: Value =
        sqlx::query_scalar("SELECT to_jsonb(i) FROM panda.vehicle_invites i WHERE token_hash=$1")
            .bind(hash_invite_token(token))
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    e.identity(token, "<unregistered-invite-token>");
    e.identity(row["id"].as_str().unwrap(), "<unregistered-invite>");
    e.identity(
        row["expires_at"].as_str().unwrap(),
        "<generated-expiry-14d>",
    );
    e.identity(
        row["token_hash"].as_str().unwrap(),
        "<hash-of-unregistered-invite-token>",
    );
    e.record(&f, "unregistered-member", r).await;
    sqlx::query("UPDATE panda.vehicles SET rivian_vehicle_id='refresh-remote' WHERE id=$1")
        .bind(id(101))
        .execute(&f.state.pool)
        .await
        .unwrap();
    let path = format!("/v1/vehicles/{}/credentials", id(101));
    e.record(
        &f,
        "refresh-mismatch",
        f.request(Some(1), "PUT", &path, json!({"rivian_vehicle_id":"wrong"}))
            .await,
    )
    .await;
    e.record(
        &f,
        "refresh-missing-session",
        f.request(Some(1), "PUT", &path, json!({})).await,
    )
    .await;
    f.stage(2).await;
    let gateway=Router::new().route("/graphql",post(|Json(body):Json<Value>|async move {
        assert_eq!(body["operationName"],"getUserInfo");
        Json(json!({"data":{"currentUser":{"vehicles":[{"id":"refresh-remote","vehicle":{"model":"R1S"}}]}}}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    with_mock_gateway(url, async {
        let before = f.snapshot().await;
        f.state
            .pool
            .execute("REVOKE INSERT ON panda.vehicle_runtime_state FROM CURRENT_USER")
            .await
            .unwrap();
        e.push(
            "refresh-late-failure",
            f.request(Some(2), "PUT", &path, json!({})).await,
        );
        assert_eq!(before, f.snapshot().await);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM panda.vehicle_credentials")
                .fetch_one(&f.state.pool)
                .await
                .unwrap(),
            0
        );
        f.state
            .pool
            .execute("GRANT INSERT ON panda.vehicle_runtime_state TO CURRENT_USER")
            .await
            .unwrap();
        e.record(
            &f,
            "refresh-manager-retry",
            f.request(Some(2), "PUT", &path, json!({})).await,
        )
        .await;
    })
    .await;
    server.abort();
    let encrypted: Vec<u8> = sqlx::query_scalar(
        "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id=$1",
    )
    .bind(id(101))
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(
            crate::ingestion::session_store::decrypt_tokens(
                &encrypted,
                &age_identity(&f.state).unwrap()
            )
            .unwrap()
        )
        .unwrap(),
        serde_json::to_value(tokens()).unwrap()
    );
    e.record(
        &f,
        "refresh-viewer-denied",
        f.request(Some(3), "PUT", &path, json!({})).await,
    )
    .await;
    f.cleanup().await;
    e.finish("extra", include_str!("extra.json"));
}

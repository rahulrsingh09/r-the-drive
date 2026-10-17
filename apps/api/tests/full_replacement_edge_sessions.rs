// Reuse the unchanged oracle fixture and assertions; this target adds boundary cases.
include!("auth_integration.rs");

use panda_api::{
    errors::AppError,
    middleware::{
        auth::{AuthUser, Claims},
        rate_limit::{AuthIdentityKeyExtractor, TrustedProxyIpKeyExtractor},
    },
    services::{security_audit::SecurityAuditEvent, sessions},
};
use tower_governor::key_extractor::KeyExtractor;

async fn edge_claims(app: &TestApp) -> Claims {
    register_and_login(app, "edge-fixture@example.test").await;
    let (sub, sid): (Uuid, Uuid) =
        sqlx::query_as("SELECT user_id, id FROM panda.session_families LIMIT 1")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    Claims {
        sub,
        sid: Some(sid),
        iss: "panda.app".into(),
        iat: chrono::Utc::now().timestamp(),
        exp: chrono::Utc::now().timestamp() + 900,
        default_vehicle_id: None,
    }
}

#[tokio::test]
async fn edge_sessions_authorization_matrix_and_database_errors() {
    let app = TestApp::new().await;
    let mut claims = edge_claims(&app).await;
    sessions::require_enabled_session(&app.pool, &claims)
        .await
        .unwrap();
    claims.exp = chrono::Utc::now().timestamp();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
    claims.exp += 900;
    let sid = claims.sid;
    claims.sid = Some(Uuid::new_v4());
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
    claims.sid = sid;
    sqlx::query("UPDATE panda.refresh_tokens SET expires_at=now()")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
    claims.sid = None;
    sessions::require_enabled_session(&app.pool, &claims)
        .await
        .unwrap();
    sqlx::query("UPDATE panda.users SET is_disabled=true")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Forbidden)
    ));
    claims.sub = Uuid::new_v4();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
    app.pool.close().await;
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Database(sqlx::Error::PoolClosed))
    ));
    claims.exp = 0;
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
}

#[tokio::test]
async fn edge_sessions_revocation_is_transactional_idempotent_and_waits_for_rotation() {
    let app = TestApp::new_with_rate_limit_and_pool_size(RateLimitConfig::default(), 4).await;
    let claims = edge_claims(&app).await;
    let family = claims.sid.unwrap();
    let mut rollback = app.pool.begin().await.unwrap();
    sessions::revoke_family(&mut rollback, family)
        .await
        .unwrap();
    rollback.rollback().await.unwrap();
    sessions::require_enabled_session(&app.pool, &claims)
        .await
        .unwrap();

    let mut rotation = app.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM panda.session_families WHERE id=$1 FOR UPDATE")
        .bind(family)
        .execute(&mut *rotation)
        .await
        .unwrap();
    let pool = app.pool.clone();
    let user = claims.sub;
    let mut revoker = tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        sessions::revoke_user_sessions(&mut tx, user).await.unwrap();
        tx.commit().await.unwrap();
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoker)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO panda.refresh_tokens(token_hash,user_id,family_id,expires_at) VALUES ($1,$2,$3,now()+interval '1 day')")
        .bind(b"edge-synthetic-descendant".as_slice()).bind(user).bind(family).execute(&mut *rotation).await.unwrap();
    rotation.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), revoker)
        .await
        .unwrap()
        .unwrap();
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM panda.refresh_tokens WHERE user_id=$1 AND revoked_at IS NULL",
    )
    .bind(user)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(active, 0);
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT revoked_at FROM panda.session_families WHERE id=$1")
            .bind(family)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    sessions::revoke_family(&mut tx, family).await.unwrap();
    tx.commit().await.unwrap();
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT revoked_at FROM panda.session_families WHERE id=$1")
            .bind(family)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn edge_sessions_rate_identity_and_proxy_trust_matrix() {
    let app = TestApp::new().await;
    let claims = edge_claims(&app).await;
    let token = panda_api::middleware::auth::issue_session_access_token(
        claims.sub,
        None,
        claims.sid,
        &app.state.jwt_keys,
    )
    .unwrap();
    let identity = AuthIdentityKeyExtractor::new(app.state.jwt_keys.decoding.clone());
    for (peer, real, forwarded, expected) in [
        (
            "192.0.2.8:80",
            "198.51.100.1",
            "198.51.100.2",
            "ip:192.0.2.8",
        ),
        (
            "127.0.0.1:80",
            "198.51.100.1",
            "198.51.100.2",
            "ip:198.51.100.1",
        ),
        ("127.0.0.1:80", "invalid", "198.51.100.2", "ip:198.51.100.2"),
        (
            "127.0.0.1:80",
            "invalid",
            "198.51.100.2, 198.51.100.3",
            "ip:127.0.0.1",
        ),
        (
            "[::1]:80",
            "::ffff:192.0.2.8",
            "invalid",
            "ip:::ffff:192.0.2.8",
        ),
    ] {
        let mut req = Request::builder()
            .header("x-real-ip", real)
            .header("x-forwarded-for", forwarded)
            .body(())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        assert_eq!(TrustedProxyIpKeyExtractor.extract(&req).unwrap(), expected);
        assert_eq!(identity.extract(&req).unwrap(), expected);
        req.headers_mut()
            .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        assert_eq!(
            identity.extract(&req).unwrap(),
            format!("user:{}", claims.sub)
        );
        req.headers_mut()
            .insert(AUTHORIZATION, "Bearer panda_synthetic".parse().unwrap());
        let key = identity.extract(&req).unwrap();
        assert!(key.starts_with("api:"));
        assert_eq!(key.len(), 68);
        assert!(!key.contains("synthetic"));
    }
}

#[tokio::test]
async fn edge_sessions_real_http_bearer_cookie_selection_and_fail_closed_database() {
    let app = TestApp::new_with_rate_limit_and_pool_size(RateLimitConfig::default(), 4).await;
    let claims = edge_claims(&app).await;
    let token = panda_api::middleware::auth::issue_session_access_token(
        claims.sub,
        None,
        claims.sid,
        &app.state.jwt_keys,
    )
    .unwrap();
    let endpoint =
        axum::routing::get(|auth: AuthUser| async move { axum::Json(json!({"id": auth.user_id})) });
    let router = Router::new()
        .route("/probe", endpoint)
        .layer(axum::Extension(app.state.jwt_keys.decoding.clone()))
        .with_state(app.state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for header in [
        None,
        Some("bearer malformed"),
        Some("Basic ignored"),
        Some("Bearer malformed"),
    ] {
        let mut req = client.get(format!("http://{address}/probe")).header(
            COOKIE,
            format!("access_token={token}; refresh_token=synthetic"),
        );
        if let Some(value) = header {
            req = req.header(AUTHORIZATION, value);
        }
        assert_eq!(req.send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        client
            .get(format!("http://{address}/probe"))
            .bearer_auth(&token)
            .header(COOKIE, "access_token=forged")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    app.pool.close().await;
    let response = client
        .get(format!("http://{address}/probe"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"error":{"code":"INTERNAL","message":"Database error"}})
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn edge_sessions_audit_persistence_rollback_request_ids_and_retention() {
    let app = TestApp::new().await;
    let claims = edge_claims(&app).await;
    let id = Uuid::new_v4();
    let event = SecurityAuditEvent::failure("edge_synthetic", Some(claims.sub))
        .target("session:synthetic")
        .request_id(Some(id))
        .metadata(json!({"reason":"revoked","family_id":claims.sid}));
    let mut tx = app.pool.begin().await.unwrap();
    event.clone().record_tx(&mut tx).await.unwrap();
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM panda.security_events WHERE event_type='edge_synthetic'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    event.record(&app.pool).await.unwrap();
    let (detail, target, request_id, outcome, metadata): (String, String, Uuid, String, Value) = sqlx::query_as("SELECT detail,target,request_id,outcome,metadata FROM panda.security_events WHERE event_type='edge_synthetic'").fetch_one(&app.pool).await.unwrap();
    assert_eq!(detail, "outcome=failure target=session:synthetic");
    assert_eq!(target, "session:synthetic");
    assert_eq!(request_id, id);
    assert_eq!(outcome, "failure");
    assert_eq!(metadata, json!({"reason":"revoked","family_id":claims.sid}));
    assert!(!format!("{detail}{metadata}").contains("edge-fixture@example.test"));
    sqlx::query("UPDATE panda.security_events SET created_at=now()-interval '366 days' WHERE event_type='edge_synthetic'").execute(&app.pool).await.unwrap();
    assert_eq!(
        panda_api::services::security_audit::prune_expired(&app.pool)
            .await
            .unwrap(),
        1
    );
    let user_count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.users")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(user_count, 1);
    app.pool.close().await;
    assert!(matches!(
        SecurityAuditEvent::success("edge_synthetic", None)
            .record(&app.pool)
            .await,
        Err(AppError::Database(sqlx::Error::PoolClosed))
    ));
}

#[tokio::test]
async fn edge_sessions_api_key_scope_expiration_touch_and_missing_extension() {
    use axum::extract::FromRequestParts;
    let app = TestApp::new().await;
    let claims = edge_claims(&app).await;
    let vehicle = insert_vehicle(&app.pool, claims.sub, "edge-vehicle", "Synthetic").await;
    let token = "panda_edge_synthetic_key";
    let hash = panda_api::routes::api_keys::hash_api_key(token);
    sqlx::query("INSERT INTO panda.api_keys(user_id,vehicle_id,key_hash,access_level) VALUES ($1,$2,$3,'read')")
        .bind(claims.sub).bind(vehicle).bind(hash.as_slice()).execute(&app.pool).await.unwrap();
    async fn extract(
        app: &TestApp,
        token: &str,
        method: Method,
        path: &str,
    ) -> Result<AuthUser, AppError> {
        let (mut parts, _) = Request::builder()
            .method(method)
            .uri(path)
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(())
            .unwrap()
            .into_parts();
        AuthUser::from_request_parts(&mut parts, &app.state).await
    }
    assert!(matches!(
        extract(&app, token, Method::GET, "/v1/admin/users").await,
        Err(AppError::Forbidden)
    ));
    let untouched: bool =
        sqlx::query_scalar("SELECT last_used_at IS NULL FROM panda.api_keys WHERE key_hash=$1")
            .bind(hash.as_slice())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(untouched);
    let auth = extract(&app, token, Method::GET, "/v1/vehicles")
        .await
        .unwrap();
    assert_eq!(auth.api_vehicle_id, Some(vehicle));
    assert_eq!(auth.api_access_level.as_deref(), Some("read"));
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_used_at FROM panda.api_keys WHERE key_hash=$1")
            .bind(hash.as_slice())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    extract(&app, token, Method::POST, "/metrics/batch")
        .await
        .unwrap();
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_used_at FROM panda.api_keys WHERE key_hash=$1")
            .bind(hash.as_slice())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    sqlx::query("UPDATE panda.api_keys SET expires_at=now()")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(matches!(
        extract(&app, token, Method::GET, "/vehicles").await,
        Err(AppError::Unauthorized)
    ));
    sqlx::query("UPDATE panda.api_keys SET expires_at=NULL, revoked_at=now()")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(matches!(
        extract(&app, token, Method::GET, "/vehicles").await,
        Err(AppError::Unauthorized)
    ));
    let jwt = panda_api::middleware::auth::issue_session_access_token(
        claims.sub,
        None,
        claims.sid,
        &app.state.jwt_keys,
    )
    .unwrap();
    assert!(matches!(
        extract(&app, &jwt, Method::GET, "/vehicles").await,
        Err(AppError::Unauthorized)
    ));
}

#[tokio::test]
async fn edge_sessions_database_failures_preserve_authorization_order() {
    let app = TestApp::new().await;
    let mut claims = edge_claims(&app).await;
    sqlx::query("ALTER TABLE panda.refresh_tokens RENAME TO edge_unavailable_tokens")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Database(_))
    ));
    claims.sid = None;
    sessions::require_enabled_session(&app.pool, &claims)
        .await
        .unwrap();
    claims.sid = Some(Uuid::new_v4());
    sqlx::query("UPDATE panda.users SET is_disabled=true")
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Forbidden)
    ));
    claims.sub = Uuid::new_v4();
    assert!(matches!(
        sessions::require_enabled_session(&app.pool, &claims).await,
        Err(AppError::Unauthorized)
    ));
}

#[tokio::test]
async fn edge_sessions_concurrent_limiter_uses_one_identity_bucket() {
    let app =
        TestApp::new_with_rate_limit_and_pool_size(deterministic_rate_limit_config(), 4).await;
    let token = register_and_login(&app, "edge-burst@example.test").await;
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let router = app.router.clone();
        let token = token.clone();
        requests.spawn(async move {
            router
                .oneshot(
                    Request::builder()
                        .uri("/v1/auth/me")
                        .header(AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        });
    }
    let mut accepted = 0;
    let mut limited = 0;
    while let Some(response) = requests.join_next().await {
        match response.unwrap() {
            StatusCode::OK => accepted += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected concurrent response: {other}"),
        }
    }
    assert_eq!(accepted, 1);
    assert_eq!(limited, 31);
}

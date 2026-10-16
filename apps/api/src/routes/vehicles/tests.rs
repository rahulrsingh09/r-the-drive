use super::*;
use axum::body::Body;
use axum::extract::State;
use axum::Json;
use http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn verified_account_lookup_uses_scoped_gateway_and_provider_metadata() {
    use crate::{
        ingestion::session_store::RivianTokenBundle,
        private_deployment::outbound::with_mock_gateway,
    };
    use serde_json::json;
    let gateway = axum::Router::new().route(
        "/graphql",
        axum::routing::post(
            |headers: http::HeaderMap, Json(body): Json<serde_json::Value>| async move {
                assert_eq!(headers["A-Sess"], "synthetic-app");
                assert_eq!(headers["U-Sess"], "synthetic-user");
                assert_eq!(headers["Csrf-Token"], "synthetic-csrf");
                assert_eq!(headers["Authorization"], "Bearer synthetic-access");
                assert_eq!(body["operationName"], "getUserInfo");
                Json(json!({"data":{"currentUser":{"vehicles":[{
                    "id":"verified-id", "vin":"provider-vin", "vehicle":{"model":"R1S"}
                }]}}}))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let tokens = RivianTokenBundle {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: "synthetic-user".into(),
        csrf_token: "synthetic-csrf".into(),
        created_at: chrono::Utc::now(),
    };
    with_mock_gateway(url, async {
        let vehicles = super::lookup_rivian_vehicles(&tokens).await.unwrap();
        assert!(matches!(
            super::require_account_vehicle(&vehicles, "forged-id"),
            Err(crate::errors::AppError::Validation(_))
        ));
        let verified = super::require_account_vehicle(&vehicles, "verified-id").unwrap();
        assert_eq!(verified.vin.as_deref(), Some("provider-vin"));
        assert_eq!(verified.model.as_deref(), Some("R1S"));
    })
    .await;
    server.abort();
}

#[test]
fn maps_rejected_rivian_credentials_to_an_actionable_error() {
    assert!(matches!(
        map_rivian_login_error(crate::ingestion::rivian_auth::RivianAuthError::InvalidCredentials),
        crate::errors::AppError::RivianCredentialsRejected
    ));
}

#[test]
fn maps_rejected_rivian_otp_to_an_actionable_error() {
    assert!(matches!(
        map_rivian_otp_error(crate::ingestion::rivian_auth::RivianAuthError::InvalidOtp),
        crate::errors::AppError::RivianOtpRejected
    ));
}

#[test]
fn requires_authorized_connected_runtime_before_reporting_refresh_ready() {
    assert!(vehicle_data_ready(Some("connected"), Some("authorized")));
    assert!(!vehicle_data_ready(Some("starting"), Some("authorized")));
    assert!(!vehicle_data_ready(Some("connected"), Some("needs_reauth")));
    assert!(!vehicle_data_ready(None, None));
}

// Run with: cargo test -- --ignored

#[tokio::test]
async fn outbound_policy_blocks_artwork_downloads_including_saved_private_urls() {
    assert_eq!(
        super::vehicle_artwork_restoring_response().status(),
        StatusCode::NOT_FOUND
    );
    let state = make_helper_state("redis://127.0.0.1/".into());
    for source in [
        "https://images.example.test/vehicle.webp",
        "http://127.0.0.1:1/vehicle.webp",
        "http://169.254.169.254/latest/meta-data",
        "https://user:password@images.example.test/vehicle.webp",
    ] {
        let result = super::download_and_store_asset(
            &reqwest::Client::new(),
            &state.config,
            Uuid::new_v4(),
            source,
        )
        .await;
        let error = result.err().expect("artwork must be denied");
        assert!(matches!(
            error.downcast_ref::<crate::services::outbound_policy::PolicyError>(),
            Some(crate::services::outbound_policy::PolicyError::OptionalTraffic)
        ));
    }
}

pub(super) fn make_helper_state(redis_url: String) -> crate::middleware::auth::AppState {
    use std::sync::Arc;

    use crate::middleware::auth::{AppState, JwtKeys};

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgresql://panda:devpassword@127.0.0.1:5432/panda")
        .expect("lazy pool");
    let redis = redis::Client::open(redis_url.clone()).expect("redis client");

    let generated = crate::keys::generate_keys().expect("keys");
    let jwt_keys = Arc::new(
        JwtKeys::new(&generated.jwt_private_pem, &generated.jwt_public_pem).expect("jwt keys"),
    );

    let config = crate::config::Config {
        database_url: "postgresql://panda:devpassword@127.0.0.1:5432/panda".into(),
        redis_url,
        jwt_secret: None,
        jwt_public_key: None,
        age_encryption_key: None,
        port: 3001,
        allowed_origins: vec!["http://localhost:3000".into()],
        s3_endpoint: None,
        s3_access_key: None,
        s3_secret_key: None,
        backup_artifact_dir: std::env::temp_dir()
            .join("panda-route-test-backups")
            .to_string_lossy()
            .into_owned(),
        vehicle_image_cache_dir: std::env::temp_dir()
            .join("panda-route-test-vehicle-images")
            .to_string_lossy()
            .into_owned(),
        backup_driver: "pg_dump".into(),
        backup_poll_interval_seconds: 60,
        restore_agent_url: "http://127.0.0.1:3002".into(),
        restore_agent_key_file: "/backups/.restore-agent-key".into(),
        recovery: crate::config::RecoveryConfig::default(),
        origin_bind: crate::config::OriginBindConfig::default(),
        security: Default::default(),
        rivian_ws_reconnect_initial_seconds: 10,
        rivian_ws_reconnect_max_seconds: 900,
        rivian_raw_event_retention_days: 7,
        rivian_persist_raw_events: true,
        rivian_suppress_duplicate_telemetry: true,
        panda_env: None,
        cookie_insecure: None,
        allow_insecure_lan_http_auth: false,
        rate_limit: crate::config::RateLimitConfig::default(),
    };

    AppState {
        pool,
        redis,
        jwt_keys,
        age_key: generated.age_key,
        config,
        nominatim_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
        resources: Default::default(),
    }
}

async fn make_app() -> axum::Router {
    use crate::middleware::auth::{AppState, JwtKeys};
    use std::sync::Arc;

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());

    let pool = crate::db::pool::create_pool(&database_url)
        .await
        .expect("create_pool");
    let redis = redis::Client::open(redis_url).expect("redis client");

    let keys = crate::keys::generate_keys().expect("generate test keys");
    let jwt_keys =
        Arc::new(JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).expect("jwt keys"));

    let config = crate::config::Config {
        database_url: database_url.clone(),
        redis_url: "redis://127.0.0.1/".into(),
        jwt_secret: None,
        jwt_public_key: None,
        age_encryption_key: None,
        port: 3001,
        allowed_origins: vec!["http://localhost:3000".into()],
        s3_endpoint: None,
        s3_access_key: None,
        s3_secret_key: None,
        backup_artifact_dir: std::env::temp_dir()
            .join("panda-route-test-backups")
            .to_string_lossy()
            .into_owned(),
        vehicle_image_cache_dir: std::env::temp_dir()
            .join("panda-route-test-vehicle-images")
            .to_string_lossy()
            .into_owned(),
        backup_driver: "pg_dump".into(),
        backup_poll_interval_seconds: 60,
        restore_agent_url: "http://127.0.0.1:3002".into(),
        restore_agent_key_file: "/backups/.restore-agent-key".into(),
        recovery: crate::config::RecoveryConfig::default(),
        origin_bind: crate::config::OriginBindConfig::default(),
        security: Default::default(),
        rivian_ws_reconnect_initial_seconds: 10,
        rivian_ws_reconnect_max_seconds: 900,
        rivian_raw_event_retention_days: 7,
        rivian_persist_raw_events: true,
        rivian_suppress_duplicate_telemetry: true,
        panda_env: None,
        cookie_insecure: None,
        allow_insecure_lan_http_auth: false,
        rate_limit: crate::config::RateLimitConfig::default(),
    };

    let state = AppState {
        pool,
        redis,
        jwt_keys,
        age_key: "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
            .to_string(),
        config,
        nominatim_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
        resources: Default::default(),
    };

    crate::routes::build_router(state)
}

async fn get_status(app: axum::Router, uri: &str) -> http::StatusCode {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

async fn post_status(app: axum::Router, uri: &str, body: serde_json::Value) -> http::StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn list_vehicles_requires_auth() {
    let app = make_app().await;
    assert_eq!(
        get_status(app, "/v1/vehicles").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn add_vehicle_requires_auth() {
    let app = make_app().await;
    let status = post_status(
        app,
        "/v1/vehicles",
        serde_json::json!({"rivian_vehicle_id": "test"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn demo_vehicle_requires_auth() {
    let app = make_app().await;
    let status = post_status(app, "/v1/vehicles/demo", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn refresh_demo_vehicle_requires_auth() {
    let app = make_app().await;
    let status = post_status(
        app,
        &format!("/v1/vehicles/{}/demo/refresh", Uuid::new_v4()),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn vehicle_status_requires_auth() {
    let app = make_app().await;
    let status = get_status(
        app,
        &format!("/v1/vehicles/{}/status", uuid::Uuid::new_v4()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn connect_requires_auth() {
    let app = make_app().await;
    let status = post_status(
        app,
        "/v1/vehicles/connect",
        serde_json::json!({"email": "a@b.com", "password": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires REDIS_URL"]
async fn encrypted_redis_round_trips_connect_tokens_without_plaintext_storage() {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".into());
    let state = make_helper_state(redis_url);
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection");
    let key = format!("test:rivian:connect:{}", Uuid::new_v4());
    let tokens = crate::ingestion::session_store::RivianTokenBundle {
        access_token: "access-token".into(),
        refresh_token: "refresh-token".into(),
        app_session_token: "app-session-token".into(),
        user_session_token: "user-session-token".into(),
        csrf_token: "csrf-token".into(),
        created_at: chrono::Utc::now(),
    };

    store_encrypted_redis(&state, &mut conn, &key, &tokens, 60)
        .await
        .expect("store encrypted connect session");

    let raw: Vec<u8> = redis::AsyncCommands::get(&mut conn, &key)
        .await
        .expect("redis get ciphertext");
    assert_ne!(raw, serde_json::to_vec(&tokens).expect("plain json"));

    let round_trip = load_encrypted_redis::<crate::ingestion::session_store::RivianTokenBundle>(
        &state, &mut conn, &key,
    )
    .await
    .expect("load encrypted connect session")
    .expect("stored connect session");

    assert_eq!(round_trip.access_token, tokens.access_token);
    assert_eq!(round_trip.refresh_token, tokens.refresh_token);
    assert_eq!(round_trip.app_session_token, tokens.app_session_token);
    assert_eq!(round_trip.user_session_token, tokens.user_session_token);
    assert_eq!(round_trip.csrf_token, tokens.csrf_token);

    let _: () = redis::AsyncCommands::del(&mut conn, &key)
        .await
        .expect("cleanup redis key");
}

#[tokio::test]
#[ignore = "requires REDIS_URL"]
async fn connect_otp_rejects_challenges_staged_for_other_users() {
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".into());
    let state = make_helper_state(redis_url);
    let owner_id = Uuid::new_v4();
    let other_user_id = Uuid::new_v4();
    let challenge_id = Uuid::new_v4().to_string();
    let key = format!("rivian:otp:{challenge_id}");
    let pending = PendingOtpChallenge {
        email: "driver@example.com".into(),
        otp_token: "otp-token".into(),
        csrf_token: "csrf-token".into(),
        app_session_token: "app-session-token".into(),
        user_id: owner_id,
    };

    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection");
    store_encrypted_redis(&state, &mut conn, &key, &pending, 60)
        .await
        .expect("store encrypted otp challenge");
    drop(conn);

    let result = connect_otp(
        State(state.clone()),
        crate::middleware::auth::AuthUser {
            user_id: other_user_id,
            default_vehicle_id: None,
            api_access_level: None,
            api_vehicle_id: None,
        },
        Json(OtpBody {
            challenge_id: challenge_id.clone(),
            otp_code: "123456".into(),
        }),
    )
    .await;

    assert!(matches!(
        result,
        Err(crate::errors::AppError::RivianConnectSessionExpired)
    ));

    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection");
    let _: () = redis::AsyncCommands::del(&mut conn, &key)
        .await
        .expect("cleanup redis key");
}

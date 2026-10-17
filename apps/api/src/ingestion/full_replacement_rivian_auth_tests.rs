use super::*;
use axum::{http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn tokens() -> RivianTokenBundle {
    RivianTokenBundle {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: "synthetic-user".into(),
        csrf_token: "synthetic-csrf".into(),
        created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
    }
}

#[tokio::test]
async fn full_replacement_key_custody_provider_failures_and_retry() {
    for (status, body, expected) in [
        (401, "not json", "Invalid credentials"),
        (
            403,
            "synthetic-secret",
            "Unexpected response: Upstream HTTP request failed",
        ),
        (
            429,
            "synthetic-secret",
            "Unexpected response: Upstream HTTP request failed",
        ),
        (
            503,
            "synthetic-secret",
            "Unexpected response: Upstream HTTP request failed",
        ),
        (
            200,
            "not json",
            "Unexpected response: Invalid upstream JSON",
        ),
        (
            200,
            r#"{"data":null}"#,
            "Unexpected response: empty CSRF payload",
        ),
        (
            200,
            r#"{"data":null,"errors":[]}"#,
            "Unexpected response: Rivian GraphQL request failed (0 errors)",
        ),
        (
            200,
            r#"{"errors":[{"message":"synthetic-secret"}]}"#,
            "Unexpected response: Rivian GraphQL request failed (1 errors)",
        ),
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let app = Router::new().route("/graphql", post(move || {
            let seen = seen.clone();
            async move {
                let attempt = seen.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    (StatusCode::from_u16(status).unwrap(), body.to_string())
                } else {
                    (StatusCode::OK, json!({"data":{"createCsrfToken":{"csrfToken":"fresh","appSessionToken":"fresh-app"}}}).to_string())
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        outbound_policy::with_mock_gateway(url, async {
            let client = reqwest::Client::new();
            let original = tokens();
            let error = rivian_refresh_csrf(&client, &original).await.unwrap_err();
            assert_eq!(error.to_string(), expected);
            assert!(!format!("{error:?}").contains("synthetic-secret"));
            assert_eq!(count.load(Ordering::SeqCst), 1, "no hidden retry");
            let refreshed = rivian_refresh_csrf(&client, &original).await.unwrap();
            assert_eq!(refreshed.app_session_token, "fresh-app");
            assert_eq!(refreshed.csrf_token, "fresh");
            assert_eq!(refreshed.access_token, original.access_token);
            assert_eq!(refreshed.refresh_token, original.refresh_token);
            assert_eq!(refreshed.user_session_token, original.user_session_token);
            assert_eq!(refreshed.created_at, original.created_at);
        })
        .await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn full_replacement_key_custody_cancel_refresh_and_deny_images_before_io() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let seen = entered.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let app = Router::new().route(
        "/graphql",
        post(move || {
            let seen = seen.clone();
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                seen.notify_one();
                std::future::pending::<Json<Value>>().await
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    outbound_policy::with_mock_gateway(url, async {
        let client = reqwest::Client::new();
        let original = tokens();
        assert!(rivian_vehicle_images(&client, &original).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let mut refresh = Box::pin(rivian_refresh_csrf(&client, &original));
        tokio::select! {
            _ = entered.notified() => {},
            result = &mut refresh => panic!("unexpected completion: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => panic!("local request never arrived"),
        }
        drop(refresh);
        assert_eq!(serde_json::to_value(&original).unwrap(), serde_json::to_value(tokens()).unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }).await;
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[test]
fn full_replacement_key_custody_auth_error_classification_matrix() {
    for phrase in [
        "invalid credentials",
        "INVALID EMAIL OR PASSWORD",
        "incorrect email or password",
        "invalid password",
        "incorrect password",
    ] {
        let errors = vec![GqlError {
            message: phrase.into(),
            extensions: None,
        }];
        assert!(matches!(
            map_login_errors(errors),
            RivianAuthError::InvalidCredentials
        ));
    }
    for (code, reason, invalid) in [
        ("BAD_USER_INPUT", "INVALID_OTP", true),
        ("UNAUTHENTICATED", "OTP_TOKEN_EXPIRED", true),
        ("UNAUTHENTICATED", "INVALID_OTP", false),
    ] {
        let errors = vec![GqlError {
            message: "synthetic-secret".into(),
            extensions: Some(GqlErrorExtensions {
                code: Some(code.into()),
                reason: Some(reason.into()),
            }),
        }];
        let error = map_otp_errors(errors);
        assert_eq!(matches!(error, RivianAuthError::InvalidOtp), invalid);
        assert!(!error.to_string().contains("synthetic-secret"));
    }
}

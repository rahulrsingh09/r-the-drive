use crate::authorization_test_support::Fixture;
use axum::{body::Body, http::Request, http::StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

async fn revoke_others(f: &Fixture, user: Uuid, cookie: Option<&str>) -> axum::response::Response {
    let mut request = Request::post("/v1/auth/sessions/revoke-others")
        .header("authorization", format!("Bearer {}", f.token(user)));
    if let Some(token) = cookie {
        request = request.header("cookie", format!("refresh_token={token}"));
    }
    f.router()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_sign_out_other_devices_keeps_only_this_browser_session() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let other_user = f.user("user").await;
    let vehicle = f.vehicle(user, "sessions-auth").await;
    let other_vehicle = f.vehicle(other_user, "sessions-auth-other").await;
    let (key, this_browser) = f.sessions(user, vehicle).await;
    let (_, other_device) = f.sessions(user, vehicle).await;
    let (_, foreign) = f.sessions(other_user, other_vehicle).await;

    // No cookie, or another user's cookie: nothing changes.
    for cookie in [None, Some(foreign.as_str()), Some("not-a-session")] {
        let before = f.snapshot().await;
        assert_eq!(
            revoke_others(&f, user, cookie).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(f.snapshot().await, before);
    }
    // Integration API keys cannot sign devices out.
    let response = f
        .router()
        .oneshot(
            Request::post("/v1/auth/sessions/revoke-others")
                .header("authorization", format!("Bearer {key}"))
                .header("cookie", format!("refresh_token={this_browser}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = revoke_others(&f, user, Some(&this_browser)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["revoked"], 1);

    assert_eq!(
        f.refresh(&other_device).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(f.refresh(&this_browser).await.status(), StatusCode::OK);
    // The other user's session is untouched.
    assert_eq!(f.refresh(&foreign).await.status(), StatusCode::OK);
    f.cleanup().await;
}

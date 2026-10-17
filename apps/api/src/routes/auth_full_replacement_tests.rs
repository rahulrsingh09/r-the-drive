use super::*;
use crate::authorization_test_support::Fixture;
use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request},
};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB"]
async fn replacement_prepare_base_database() {
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    crate::db::migrations::run_current_migrations(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[test]
fn replacement_cookie_bytes_and_unicode_password_contract() {
    for insecure in [false, true] {
        let secure = if insecure { "" } else { "; Secure" };
        assert_eq!(
            refresh_cookie("abc", 2592000, insecure),
            format!(
                "refresh_token=abc; HttpOnly{secure}; SameSite=Lax; Path=/v1/auth; Max-Age=2592000"
            )
        );
        assert_eq!(
            oidc_state_cookie("", 0, insecure),
            format!("oidc_state=; HttpOnly{secure}; SameSite=Lax; Path=/v1/auth; Max-Age=0")
        );
    }
    assert!(password_meets_minimum("éééééé"));
    assert!(!password_meets_minimum("ééééé"));
    assert!(matches!(
        verify_password("guess", "malformed"),
        Err(AppError::Unauthorized)
    ));
    for link in [false, true] {
        let response = oidc_callback_error_response(link, "oidc_denied", false);
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()["location"],
            if link {
                "/settings?section=account&error=oidc_denied"
            } else {
                "/login?error=oidc_denied"
            }
        );
        assert_eq!(response.headers().get_all(SET_COOKIE).iter().count(), 1);
    }
}

#[test]
fn replacement_preferences_fallback_and_validation_order() {
    let metric = resolved_units_payload(
        "invalid", None, None, None, None, None, None, None, "miles", "CELSIUS",
    );
    assert_eq!(metric.mode, "metric");
    let custom = resolved_units_payload(
        "custom",
        Some("stored-value"),
        None,
        None,
        None,
        None,
        None,
        None,
        "kilometers",
        "celsius",
    );
    assert_eq!(custom.mode, "custom");
    assert_eq!(custom.distance_unit, "stored-value");
    assert_eq!(custom.speed_unit, "mph");
    assert!(
        matches!(normalize_units_payload(custom), Err(AppError::Validation(message)) if message=="invalid unit preference value")
    );
    assert!(
        matches!(normalize_theme_payload(ThemePreferencesPayload { mode:"bad".into(),palette:"bad".into() }), Err(AppError::Validation(message)) if message=="invalid theme mode")
    );
}

async fn post(state: &AppState, path: &str, value: serde_json::Value) -> Response {
    router()
        .with_state(state.clone())
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn cookie_headers(raw: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("cookie", format!("refresh_token={raw}").parse().unwrap());
    headers
}

async fn invitation(f: &Fixture, owner: Uuid, methods: &str) -> (Uuid, String, String) {
    let token = Uuid::new_v4().to_string();
    let email = format!("{}@example.invalid", Uuid::new_v4());
    let id = sqlx::query_scalar("INSERT INTO panda.account_invitations(invited_by,invitee_email,token_hash,auth_methods,expires_at) VALUES($1,$2,$3,$4,now()+interval '1 day') RETURNING id")
        .bind(owner).bind(&email).bind(sha2_hash(&token)).bind(methods).fetch_one(&f.state.pool).await.unwrap();
    (id, token, email)
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_concurrent_owner_claim_and_production_proof_denial() {
    let f = Fixture::new().await;
    let mut production = f.state.clone();
    production.config.panda_env = Some("production".into());
    let denied = post(
        &production,
        "/auth/register",
        serde_json::json!({"email":"owner@example.invalid","password":"synthetic-password"}),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.users")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let a = post(
        &f.state,
        "/auth/register",
        serde_json::json!({"email":"first@example.invalid","password":"synthetic-password"}),
    );
    let b = post(
        &f.state,
        "/auth/register",
        serde_json::json!({"email":"second@example.invalid","password":"synthetic-password"}),
    );
    let (a, b) = tokio::join!(a, b);
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [201, 403]);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.users WHERE role='super_user'")
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM panda.security_events WHERE event_type='setup_claimed'",
    )
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_parallel_refresh_replay_revokes_winning_descendant() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let (raw, sid) = issue_new_session(&f.state.pool, user).await.unwrap();
    let headers = cookie_headers(&raw);
    let (a, b) = tokio::join!(
        refresh_from_cookie(&f.state, &headers),
        refresh_from_cookie(&f.state, &headers)
    );
    let responses = [a.unwrap(), b.unwrap()];
    assert_eq!(responses.iter().filter(|r| r.is_some()).count(), 1);
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM panda.session_families WHERE id=$1")
            .bind(sid)
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert!(revoked);
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM panda.refresh_tokens WHERE family_id=$1 AND revoked_at IS NULL",
    )
    .bind(sid)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(active, 0);
    let response = responses.into_iter().flatten().next().unwrap();
    let descendant = response.headers()[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("refresh_token=")
        .unwrap();
    assert!(refresh_from_cookie(&f.state, &cookie_headers(descendant))
        .await
        .unwrap()
        .is_none());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_concurrent_password_change_serializes_current_credential() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    sqlx::query("UPDATE panda.users SET password_hash=$2 WHERE id=$1")
        .bind(user)
        .bind(hash_password("original-password").unwrap())
        .execute(&f.state.pool)
        .await
        .unwrap();
    let raw = issue_refresh_token(&f.state.pool, user).await.unwrap();
    let change = |new: &str| {
        change_password(
            State(f.state.clone()),
            f.auth(user),
            HeaderMap::new(),
            Json(ChangePasswordBody {
                current_password: "original-password".into(),
                new_password: new.into(),
            }),
        )
    };
    let (a, b) = tokio::join!(
        change("replacement-password-a"),
        change("replacement-password-b")
    );
    assert_eq!([&a, &b].into_iter().filter(|r| r.is_ok()).count(), 1);
    let failure = if a.is_err() { a } else { b };
    assert!(
        matches!(failure,Err(AppError::Validation(message)) if message=="current password is incorrect")
    );
    assert!(refresh_from_cookie(&f.state, &cookie_headers(&raw))
        .await
        .unwrap()
        .is_none());
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM panda.security_events WHERE event_type='password_changed' AND user_id=$1").bind(user).fetch_one(&f.state.pool).await.unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_invitation_race_and_negative_state_precedence() {
    let f = Fixture::new().await;
    let owner = f.user("super_user").await;
    let (id, token, email) = invitation(&f, owner, "both").await;
    let accept = || {
        post(
            &f.state,
            "/auth/account-invitations/accept",
            serde_json::json!({"token":token,"password":"invited-password"}),
        )
    };
    let (a, b) = tokio::join!(accept(), accept());
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [201, 422]);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.users WHERE email=$1")
        .bind(email)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query("UPDATE panda.account_invitations SET revoked_at=now(),expires_at=now()-interval '1 day' WHERE id=$1").bind(id).execute(&f.state.pool).await.unwrap();
    let response = post(
        &f.state,
        "/auth/account-invitations/preview",
        serde_json::json!({"token":token}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 10000).await.unwrap()).unwrap();
    assert_eq!(body["error"]["message"], "invitation revoked");
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_oidc_invitation_concurrency_and_unverified_email_denial() {
    let f = Fixture::new().await;
    let owner = f.user("super_user").await;
    let (id, _, email) = invitation(&f, owner, "sso").await;
    let mut identity = oidc::VerifiedIdentity {
        issuer: "https://issuer.example.invalid".into(),
        subject: Uuid::new_v4().to_string(),
        email: Some(email),
        email_verified: false,
        claims: serde_json::json!({}),
    };
    assert!(matches!(
        accept_oidc_invitation(&f.state.pool, id, &identity).await,
        Err(AppError::Forbidden)
    ));
    identity.email_verified = true;
    let (a, b) = tokio::join!(
        accept_oidc_invitation(&f.state.pool, id, &identity),
        accept_oidc_invitation(&f.state.pool, id, &identity)
    );
    assert_eq!([a, b].into_iter().filter(|r| r.is_ok()).count(), 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM panda.user_oidc_identities WHERE issuer=$1 AND subject=$2",
    )
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_password_and_sso_policy_denials_issue_no_sessions() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let email: String = sqlx::query_scalar("SELECT email FROM panda.users WHERE id=$1")
        .bind(user)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
    for (methods, disabled, status) in [
        ("both", false, 401),
        ("sso", false, 401),
        ("both", true, 403),
    ] {
        sqlx::query(
            "UPDATE panda.users SET auth_methods=$2,is_disabled=$3,password_hash=$4 WHERE id=$1",
        )
        .bind(user)
        .bind(methods)
        .bind(disabled)
        .bind(hash_password("correct-password").unwrap())
        .execute(&f.state.pool)
        .await
        .unwrap();
        let password = if methods == "both" && !disabled {
            "incorrect-password"
        } else {
            "correct-password"
        };
        let response = post(
            &f.state,
            "/auth/login",
            serde_json::json!({"email":email,"password":password}),
        )
        .await;
        assert_eq!(response.status().as_u16(), status);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM panda.session_families WHERE user_id=$1")
            .bind(user)
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_oidc_malformed_and_missing_transactions_do_not_exchange() {
    let f = Fixture::new().await;
    let key = format!("malformed-{}", Uuid::new_v4());
    f.state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap()
        .set_ex::<_, _, ()>(format!("panda:oidc:tx:{key}"), "not json", 60)
        .await
        .unwrap();
    for query in [
        String::new(),
        format!("?state={key}&code=never-exchange"),
        format!("?state={key}&code=replayed"),
    ] {
        let response = router()
            .with_state(f.state.clone())
            .oneshot(
                Request::get(format!("/auth/oidc/callback{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()["location"], "/login?error=oidc_expired");
        assert_eq!(
            response.headers()[SET_COOKIE],
            "oidc_state=; HttpOnly; Secure; SameSite=Lax; Path=/v1/auth; Max-Age=0"
        );
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_real_http_rate_limit_origin_and_access_denials() {
    let mut f = Fixture::new().await;
    f.state.config.rate_limit.auth_public_per_minute = 1;
    f.state.config.rate_limit.auth_public_burst = 2;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = crate::routes::build_router(f.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://{address}");
    let bootstrap = client
        .post(format!("{base}/v1/auth/bootstrap"))
        .header("origin", "http://untrusted.invalid")
        .send()
        .await
        .unwrap();
    assert_eq!(bootstrap.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        bootstrap.headers()[SET_COOKIE],
        "refresh_token=; HttpOnly; Secure; SameSite=Lax; Path=/v1/auth; Max-Age=0"
    );
    assert!(bootstrap
        .headers()
        .get("access-control-allow-origin")
        .is_none());
    for expected in [StatusCode::UNAUTHORIZED, StatusCode::TOO_MANY_REQUESTS] {
        let response = client.post(format!("{base}/v1/auth/login")).json(&serde_json::json!({"email":"missing@example.invalid","password":"incorrect-password"})).send().await.unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(response.headers()["x-panda-ratelimit-source"], "api");
        }
        assert!(response.headers().get(SET_COOKIE).is_none());
    }
    let denied = client
        .get(format!("{base}/v1/auth/me"))
        .bearer_auth("invalid-credential")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_oidc_provisioning_serializes_identity_and_preserves_owner_boundary() {
    let f = Fixture::new().await;
    let mut settings = authentication_settings::load_effective(&f.state.pool, &f.state.age_key)
        .await
        .unwrap();
    settings.auto_signup = true;
    settings.allowed_email_domains.clear();
    let mut transaction = oidc::Transaction {
        state: "synthetic-state".into(),
        browser_binding: "synthetic-binding".into(),
        nonce: "synthetic-nonce".into(),
        verifier: "synthetic-verifier".into(),
        return_to: "/".into(),
        user_id: None,
        link: false,
        pending_password_hash: None,
        invitation_id: None,
    };
    let identity = oidc::VerifiedIdentity {
        issuer: "https://issuer.example.invalid".into(),
        subject: Uuid::new_v4().to_string(),
        email: Some(format!("{}@example.invalid", Uuid::new_v4())),
        email_verified: true,
        claims: serde_json::json!({}),
    };
    assert!(matches!(
        resolve_oidc_identity(&f.state.pool, &settings, &transaction, &identity).await,
        Err(AppError::Forbidden)
    ));
    let owner = f.user("super_user").await;
    let (first, second) = tokio::join!(
        resolve_oidc_identity(&f.state.pool, &settings, &transaction, &identity),
        resolve_oidc_identity(&f.state.pool, &settings, &transaction, &identity)
    );
    let user = first.unwrap();
    assert_eq!(second.unwrap(), user);
    let account: (String, bool, i64) = sqlx::query_as("SELECT role,password_hash IS NULL,(SELECT count(*) FROM panda.user_preferences WHERE user_id=$1) FROM panda.users WHERE id=$1").bind(user).fetch_one(&f.state.pool).await.unwrap();
    assert_eq!(account, ("user".into(), true, 1));
    transaction.link = true;
    transaction.user_id = Some(owner);
    assert!(
        matches!(resolve_oidc_identity(&f.state.pool, &settings, &transaction, &identity).await, Err(AppError::Conflict(message)) if message == "this SSO identity is already linked to another account")
    );
    sqlx::query("UPDATE panda.users SET is_disabled=TRUE WHERE id=$1")
        .bind(user)
        .execute(&f.state.pool)
        .await
        .unwrap();
    transaction.link = false;
    transaction.user_id = None;
    assert!(matches!(
        resolve_oidc_identity(&f.state.pool, &settings, &transaction, &identity).await,
        Err(AppError::Forbidden)
    ));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable no-egress TimescaleDB and Redis"]
async fn replacement_metadata_favorites_merge_concurrently_and_map_style_stays_closed() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let initial = get_chart_favorites(State(f.state.clone()), f.auth(user))
        .await
        .unwrap();
    assert_eq!(initial.0.chart_favorites, serde_json::json!({}));
    let update = |key: &str, chart: &str| {
        update_chart_favorite(
            State(f.state.clone()),
            f.auth(user),
            Json(DashboardChartFavoriteUpdateBody {
                key: key.into(),
                chart_id: chart.into(),
            }),
        )
    };
    let (a, b) = tokio::join!(update("first", "speed"), update("second", "temperature"));
    assert_eq!(a.unwrap().0.chart_favorites["first"], "speed");
    assert_eq!(b.unwrap().0.chart_favorites["second"], "temperature");
    let favorites = get_chart_favorites(State(f.state.clone()), f.auth(user))
        .await
        .unwrap();
    assert_eq!(
        favorites.0.chart_favorites,
        serde_json::json!({"first":"speed","second":"temperature"})
    );
    assert!(
        matches!(update(&"é".repeat(101), "speed").await, Err(AppError::Validation(message)) if message == "invalid dashboard chart favorite")
    );
    let style = update_map_style(
        State(f.state.clone()),
        f.auth(user),
        Json(MapStyleUpdateBody {
            map_style: "dark".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(style.0.map_style, "dark");
    assert!(
        matches!(update_map_style(State(f.state.clone()), f.auth(user), Json(MapStyleUpdateBody { map_style: "satellite".into() })).await, Err(AppError::Validation(message)) if message == "invalid map style")
    );
    let prefs = get_preferences(State(f.state.clone()), f.auth(user))
        .await
        .unwrap();
    assert_eq!(prefs.0.map_style, "dark");
    assert_eq!(prefs.0.theme.palette, "classic");
    f.cleanup().await;
}

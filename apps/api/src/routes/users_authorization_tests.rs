use super::*;
use crate::authorization_test_support::Fixture;
use axum::http::StatusCode;
use serde_json::{json, Value};

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_jwt_and_api_keys_reject_disabled_and_nonexistent_users() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-auth").await;
    let token = f.token(user);
    let (key, _) = f.sessions(user, vehicle).await;
    for bearer in [&token, &key] {
        assert_eq!(
            f.request("GET", "/v1/vehicles", bearer, Value::Null)
                .await
                .status(),
            StatusCode::OK
        );
    }
    sqlx::query("UPDATE panda.users SET is_disabled = TRUE WHERE id = $1")
        .bind(user)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert_eq!(
        f.request("GET", "/v1/vehicles", &token, Value::Null)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request("GET", "/v1/vehicles", &key, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(f.snapshot().await, before);
    // Deliberately leave a legacy membership behind to exercise the missing-user guard.
    sqlx::query("DELETE FROM panda.users WHERE id = $1")
        .bind(user)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert_eq!(
        f.request("GET", "/v1/vehicles", &token, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.request(
            "POST",
            "/v1/vehicles",
            &token,
            json!({"rivian_vehicle_id": "forged"})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(f.snapshot().await, before);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_user_deletion_removes_sessions_and_memberships_preserving_history() {
    let f = Fixture::new().await;
    let admin = f.user("super_user").await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-deletion").await;
    f.member(admin, vehicle, "owner").await;
    let token = f.token(user);
    let (key, refresh) = f.sessions(user, vehicle).await;
    sqlx::query("INSERT INTO panda.user_preferences (user_id) VALUES ($1)")
        .bind(user)
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO panda.user_oidc_identities (user_id, issuer, subject)
         VALUES ($1, 'https://identity.example.invalid', 'synthetic-subject')",
    )
    .bind(user)
    .execute(&f.state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO panda.vehicle_credentials (vehicle_id, encrypted_tokens, token_created_at)
         VALUES ($1, $2, now())",
    )
    .bind(vehicle)
    .bind(b"synthetic-vehicle-credentials".as_slice())
    .execute(&f.state.pool)
    .await
    .unwrap();
    let trip: Uuid = sqlx::query_scalar(
        "INSERT INTO panda.trips (vehicle_id, started_at, ended_at)
         VALUES ($1, now() - interval '1 hour', now()) RETURNING id",
    )
    .bind(vehicle)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO timeseries.telemetry (vehicle_id, ts, battery_level)
         VALUES ($1, now(), 42)",
    )
    .bind(vehicle)
    .execute(&f.state.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO panda.vehicle_invites
         (vehicle_id, invited_by, invitee_email, role, token_hash, expires_at)
         SELECT $1, $2, email, 'viewer', $3, now() + interval '1 day'
         FROM panda.users WHERE id = $4",
    )
    .bind(vehicle)
    .bind(admin)
    .bind(b"synthetic-invite".as_slice())
    .bind(user)
    .execute(&f.state.pool)
    .await
    .unwrap();
    let mut conn = f
        .state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: () = redis::AsyncCommands::set_ex(
        &mut conn,
        format!("rivian:connect:{user}"),
        "synthetic-staging",
        60,
    )
    .await
    .unwrap();
    let history_before: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'vehicles', (SELECT jsonb_agg(to_jsonb(v)) FROM panda.vehicles v),
            'trips', (SELECT jsonb_agg(to_jsonb(t)) FROM panda.trips t),
            'telemetry', (SELECT jsonb_agg(to_jsonb(t)) FROM timeseries.telemetry t),
            'credentials', (SELECT jsonb_agg(to_jsonb(c)) FROM panda.vehicle_credentials c))",
    )
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(
        f.request(
            "DELETE",
            &format!("/v1/admin/users/{user}"),
            &f.token(admin),
            Value::Null
        )
        .await
        .status(),
        StatusCode::OK
    );
    let remaining: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
         (SELECT count(*) FROM panda.users WHERE id = $1),
         (SELECT count(*) FROM panda.vehicle_memberships WHERE user_id = $1),
         (SELECT count(*) FROM panda.vehicle_user_settings WHERE user_id = $1),
         (SELECT count(*) FROM panda.refresh_tokens WHERE user_id = $1),
         (SELECT count(*) FROM panda.api_keys WHERE user_id = $1),
         (SELECT count(*) FROM panda.user_preferences WHERE user_id = $1),
         (SELECT count(*) FROM panda.user_oidc_identities WHERE user_id = $1)",
    )
    .bind(user)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(remaining, (0, 0, 0, 0, 0, 0, 0));
    let history_after: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'vehicles', (SELECT jsonb_agg(to_jsonb(v)) FROM panda.vehicles v),
            'trips', (SELECT jsonb_agg(to_jsonb(t)) FROM panda.trips t),
            'telemetry', (SELECT jsonb_agg(to_jsonb(t)) FROM timeseries.telemetry t),
            'credentials', (SELECT jsonb_agg(to_jsonb(c)) FROM panda.vehicle_credentials c))",
    )
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(
        history_after, history_before,
        "vehicle and trip {trip} must survive deletion"
    );
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM panda.vehicle_invites WHERE revoked_at IS NULL")
            .fetch_one(&f.state.pool)
            .await
            .unwrap();
    assert_eq!(pending, 0);
    let staged: bool = redis::AsyncCommands::exists(&mut conn, format!("rivian:connect:{user}"))
        .await
        .unwrap();
    assert!(!staged);
    for bearer in [&token, &key] {
        assert_eq!(
            f.request("GET", "/v1/vehicles", bearer, Value::Null)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(f.refresh(&refresh).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.request("GET", "/v1/vehicles", &f.token(admin), Value::Null)
            .await
            .status(),
        StatusCode::OK
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_failed_user_deletion_rolls_back_all_cleanup() {
    let f = Fixture::new().await;
    let admin = f.user("super_user").await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-rollback").await;
    f.sessions(user, vehicle).await;
    sqlx::query("INSERT INTO panda.user_preferences (user_id) VALUES ($1)")
        .bind(user)
        .execute(&f.state.pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE FUNCTION panda.fail_test_user_delete() RETURNS trigger LANGUAGE plpgsql AS
         $$ BEGIN RAISE EXCEPTION 'synthetic late delete failure'; END $$;
         CREATE TRIGGER fail_test_user_delete BEFORE DELETE ON panda.users
         FOR EACH ROW EXECUTE FUNCTION panda.fail_test_user_delete();",
    )
    .execute(&f.state.pool)
    .await
    .unwrap();
    let before = f.snapshot().await;
    assert_eq!(
        f.request(
            "DELETE",
            &format!("/v1/admin/users/{user}"),
            &f.token(admin),
            Value::Null
        )
        .await
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(
        f.request(
            "DELETE",
            &format!("/v1/admin/users/{admin}"),
            &f.token(user),
            Value::Null
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(
        f.request(
            "DELETE",
            &format!("/v1/admin/users/{admin}"),
            &f.token(admin),
            Value::Null
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(f.snapshot().await, before);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_disabling_user_revokes_refresh_and_api_sessions() {
    let f = Fixture::new().await;
    let admin = f.user("super_user").await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-disable").await;
    let (key, refresh) = f.sessions(user, vehicle).await;
    assert_eq!(
        f.request(
            "PATCH",
            &format!("/v1/admin/users/{user}"),
            &f.token(admin),
            json!({"is_disabled": true})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(f.refresh(&refresh).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.request("GET", "/v1/vehicles", &key, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let active: (i64, i64) = sqlx::query_as(
        "SELECT
         (SELECT count(*) FROM panda.refresh_tokens WHERE user_id = $1 AND revoked_at IS NULL),
         (SELECT count(*) FROM panda.api_keys WHERE user_id = $1 AND revoked_at IS NULL)",
    )
    .bind(user)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    assert_eq!(active, (0, 0));
    assert_eq!(
        f.request(
            "PATCH",
            &format!("/v1/admin/users/{user}"),
            &f.token(admin),
            json!({"is_disabled": false})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        f.request("GET", "/v1/vehicles", &f.token(user), Value::Null)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(f.refresh(&refresh).await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.request("GET", "/v1/vehicles", &key, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    f.cleanup().await;
}

use crate::authorization_test_support::Fixture;
use axum::http::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

async fn session(f: &Fixture, vehicle: Uuid, hours_ago: i32, lat: f64, lng: f64) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO panda.charge_sessions
           (vehicle_id, started_at, ended_at, source_location_lat, source_location_lng,
            location_lat, location_lng, kwh_added, source)
         VALUES ($1, now() - make_interval(hours => $2), now() - make_interval(hours => $2 - 1),
                 $3, $4, $3, $4, 10, 'telemetry')
         RETURNING id",
    )
    .bind(vehicle)
    .bind(hours_ago)
    .bind(lat)
    .bind(lng)
    .fetch_one(&f.state.pool)
    .await
    .unwrap()
}

async fn mode(f: &Fixture, id: Uuid) -> (String, Option<Uuid>) {
    sqlx::query_as(
        "SELECT location_override_mode, geofence_id FROM panda.charge_sessions WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&f.state.pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_place_tag_is_a_manager_change_for_charges_at_one_place() {
    let f = Fixture::new().await;
    let owner = f.user("user").await;
    let viewer = f.user("user").await;
    let stranger = f.user("user").await;
    let vehicle = f.vehicle(owner, "place-tag-auth").await;
    f.member(viewer, vehicle, "viewer").await;
    let home: Uuid = sqlx::query_scalar(
        "INSERT INTO panda.geofences (user_id, name, latitude, longitude, radius_m, is_home)
         VALUES ($1, 'Home', 47.6000, -122.3000, 75, TRUE) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    // Two charges about 50 m apart, away from the saved Home pin, and one far away.
    let anchor = session(&f, vehicle, 30, 47.7000, -122.2000).await;
    let nearby = session(&f, vehicle, 20, 47.7004, -122.2002).await;
    let far = session(&f, vehicle, 10, 47.8000, -122.1000).await;
    let path = format!("/v1/vehicles/{vehicle}/charging-sessions/{anchor}/place-tag");

    for user in [viewer, stranger] {
        let before = f.snapshot().await;
        let response = f
            .request("POST", &path, &f.token(user), json!({"tag": "home"}))
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(f.snapshot().await, before);
    }
    assert_eq!(mode(&f, anchor).await, ("automatic".into(), None));

    let response = f
        .request("POST", &path, &f.token(owner), json!({"tag": "home"}))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["tag"], "home");
    assert_eq!(body["place_id"], json!(home));
    assert_eq!(body["updated"], 2);
    for id in [anchor, nearby] {
        assert_eq!(mode(&f, id).await, ("saved_place".into(), Some(home)));
    }
    assert_eq!(mode(&f, far).await, ("automatic".into(), None));

    // Other then follows the saved place to every charge linked to it.
    let response = f
        .request("POST", &path, &f.token(owner), json!({"tag": "other"}))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    for id in [anchor, nearby] {
        assert_eq!(mode(&f, id).await, ("none".into(), None));
    }
    let response = f
        .request("POST", &path, &f.token(owner), json!({"tag": "office"}))
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // A place named "Office" without the flag is the fallback.
    let office: Uuid = sqlx::query_scalar(
        "INSERT INTO panda.geofences (user_id, name, latitude, longitude, radius_m)
         VALUES ($1, 'Office', 47.6100, -122.3300, 75) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    let response = f
        .request("POST", &path, &f.token(owner), json!({"tag": "office"}))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    for id in [anchor, nearby] {
        assert_eq!(mode(&f, id).await, ("saved_place".into(), Some(office)));
    }
    f.cleanup().await;
}

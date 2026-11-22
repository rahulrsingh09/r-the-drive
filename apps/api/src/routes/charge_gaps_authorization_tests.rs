use crate::authorization_test_support::Fixture;
use axum::{body::Body, http::Request, http::StatusCode};
use serde_json::Value;
use tower::ServiceExt;

async fn body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_charge_gaps_are_member_reads_scoped_to_the_vehicle() {
    let f = Fixture::new().await;
    let owner = f.user("user").await;
    let viewer = f.user("user").await;
    let stranger = f.user("user").await;
    let vehicle = f.vehicle(owner, "charge-gaps-auth").await;
    let other = f.vehicle(owner, "charge-gaps-other").await;
    f.member(viewer, vehicle, "viewer").await;
    // Parked overnight, battery rose 40 -> 62 with no charge session.
    sqlx::query(
        "INSERT INTO timeseries.telemetry (ts, vehicle_id, battery_level)
         VALUES (now() - interval '10 hours', $1, 40),
                (now() - interval '9 hours', $1, 41),
                (now() - interval '6 hours', $1, 62)",
    )
    .bind(vehicle)
    .execute(&f.state.pool)
    .await
    .unwrap();
    let path = format!("/v1/vehicles/{vehicle}/charge-gaps");

    for user in [owner, viewer] {
        let response = f.request("GET", &path, &f.token(user), Value::Null).await;
        assert_eq!(response.status(), StatusCode::OK);
        let report = body(response).await;
        assert_eq!(report["vehicle_id"], serde_json::json!(vehicle));
        assert_eq!(report["lookback_days"], 30);
        assert_eq!(report["parked_periods_checked"], 1);
        let days = report["daily_samples"].as_array().expect("daily samples");
        assert_eq!(
            days.iter()
                .map(|day| day["samples"].as_i64().unwrap())
                .sum::<i64>(),
            3
        );
        assert!(days
            .iter()
            .all(|day| day["day"].as_str().is_some_and(|day| day.len() == 10)));
        let gaps = report["gaps"].as_array().expect("a JSON list");
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0]["battery_from"], 40.0);
        assert_eq!(gaps[0]["battery_to"], 62.0);
        assert!(gaps[0]["start"].is_string() && gaps[0]["end"].is_string());
    }

    let response = f
        .request("GET", &path, &f.token(stranger), Value::Null)
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Integration API keys are not part of this surface, for this vehicle or
    // any other: only the integration read list is open to them.
    for scoped in [other, vehicle] {
        let (key, _) = f.sessions(owner, scoped).await;
        let response = f.request("GET", &path, &key, Value::Null).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    let anonymous = f
        .router()
        .oneshot(Request::get(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    f.cleanup().await;
}

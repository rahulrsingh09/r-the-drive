use super::*;

pub(super) fn retain_unauthorized_session_costs(expected: &mut Value) {
    let mut corrected = 0;
    for event in expected.as_array_mut().unwrap() {
        for session in event["persisted"]["charge_sessions"]
            .as_array_mut()
            .unwrap()
        {
            if session["id"] == id(103).to_string() {
                assert_eq!(session["vehicle_id"], id(11).to_string());
                session["cost_profile_id"] = json!("generated-1");
                session["cost_method"] = Value::Null;
                session["cost_usd"] = json!(1);
                corrected += 1;
            }
        }
    }
    assert_eq!(corrected, 95);
}

async fn row(pool: &sqlx::PgPool, session: u128) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(cs) FROM panda.charge_sessions cs WHERE id=$1")
        .bind(id(session))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn checked(
    state: &AppState,
    evidence: &mut Evidence,
    label: &str,
    user: u128,
    method: &str,
    path: &str,
    body: Value,
) -> Value {
    let response = request(state, user, method, path, body.to_string()).await;
    evidence.record(state, label, response.clone()).await;
    assert_eq!(response["status"], 200, "{label}: {response}");
    response["body"].clone()
}

#[tokio::test]
#[ignore = "requires unique disposable PLACES_TEST_DATABASE_URL; authentic migrations and restricted app role"]
async fn membership_revocation_stops_place_repricing_and_profile_resolution() {
    let db = database::DatabaseFixture::new().await;
    for user in [1, 2] {
        sqlx::query("INSERT INTO panda.users(id,email,role) VALUES ($1,$2,'user')")
            .bind(id(user))
            .bind(format!("revocation-{user}@example.invalid"))
            .execute(&db.pool)
            .await
            .unwrap();
    }
    for (vehicle, owner, session) in [(10, 1, 100), (11, 1, 101), (12, 2, 102)] {
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES ($1,$2,$3,'synthetic')")
            .bind(id(vehicle)).bind(id(owner)).bind(format!("synthetic-revocation-{vehicle}"))
            .execute(&db.pool).await.unwrap();
        sqlx::query(
            "INSERT INTO panda.vehicle_memberships(user_id,vehicle_id,role) VALUES ($1,$2,'owner')",
        )
        .bind(id(owner))
        .bind(id(vehicle))
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,kwh_added,cost_override_mode) VALUES ($1,$2,'2026-01-01T01:00:00Z','2026-01-01T02:00:00Z',10,'automatic')")
            .bind(id(session)).bind(id(vehicle)).execute(&db.pool).await.unwrap();
    }
    for (vehicle, session) in [(10, 103), (11, 104)] {
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,kwh_added,cost_override_mode) VALUES ($1,$2,'2026-01-02T01:00:00Z','2026-01-02T02:00:00Z',10,'automatic')")
            .bind(id(session)).bind(id(vehicle)).execute(&db.pool).await.unwrap();
    }
    let keys = crate::keys::generate_keys().unwrap();
    let state = AppState {
        pool: db.pool.clone(),
        redis: redis::Client::open("redis://redis:6379/").unwrap(),
        jwt_keys: Arc::new(
            crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem)
                .unwrap(),
        ),
        age_key: keys.age_key,
        config: serde_json::from_value(json!({
            "database_url":"postgres://synthetic-local-only",
            "redis_url":"redis://127.0.0.1:1/",
            "panda_env":"production"
        }))
        .unwrap(),
        resources: Default::default(),
        nominatim_cache: Default::default(),
        supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
    };
    let mut evidence = Evidence::default();
    for vehicle in [10, 11] {
        checked(
            &state,
            &mut evidence,
            "owner-grants-manager",
            1,
            "POST",
            &format!("/v1/vehicles/{}/members", id(vehicle)),
            json!({"email":"revocation-2@example.invalid","role":"manager"}),
        )
        .await;
    }
    let created = checked(
        &state,
        &mut evidence,
        "manager-creates-own-place",
        2,
        "POST",
        "/v1/places",
        place("Revocation fixture"),
    )
    .await;
    let place_id = created["id"].as_str().unwrap();
    let place_path = format!("/v1/places/{place_id}");
    let profile_id = created["charging"]["id"].as_str().unwrap();
    for (vehicle, session) in [(10, 100), (11, 101), (12, 102)] {
        checked(
            &state,
            &mut evidence,
            "authorized-manager-or-owner-attaches-place",
            2,
            "PATCH",
            &format!(
                "/v1/vehicles/{}/charging-sessions/{}",
                id(vehicle),
                id(session)
            ),
            json!({"place_id":place_id}),
        )
        .await;
        assert_eq!(row(&db.pool, session).await["cost_usd"], 2.5);
    }
    for (vehicle, session, body) in [
        (
            10,
            103,
            json!({"place_id":place_id,"cost_mode":"manual","cost_usd":17.0}),
        ),
        (11, 104, json!({"place_id":place_id,"cost_mode":"free"})),
    ] {
        checked(
            &state,
            &mut evidence,
            "manager-sets-fixed-cost",
            2,
            "PATCH",
            &format!(
                "/v1/vehicles/{}/charging-sessions/{}",
                id(vehicle),
                id(session)
            ),
            body,
        )
        .await;
    }
    let mut edited = place("Active manager repricing");
    edited["charging"]["rate"] = json!(0.4);
    checked(
        &state,
        &mut evidence,
        "active-manager-and-owner-repricing",
        2,
        "PUT",
        &place_path,
        edited.clone(),
    )
    .await;
    for session in [100, 101, 102] {
        assert_eq!(row(&db.pool, session).await["cost_usd"], 4.0);
    }
    checked(
        &state,
        &mut evidence,
        "owner-demotes-manager",
        1,
        "PUT",
        &format!("/v1/vehicles/{}/members/{}", id(10), id(2)),
        json!({"role":"viewer"}),
    )
    .await;
    checked(
        &state,
        &mut evidence,
        "owner-removes-manager",
        1,
        "DELETE",
        &format!("/v1/vehicles/{}/members/{}", id(11), id(2)),
        Value::Null,
    )
    .await;
    let mut before = Vec::new();
    for session in [100, 101, 103, 104] {
        before.push(row(&db.pool, session).await);
    }
    for (vehicle, session) in [(10, 100), (11, 101)] {
        let response = request(
            &state,
            2,
            "PATCH",
            &format!(
                "/v1/vehicles/{}/charging-sessions/{}",
                id(vehicle),
                id(session)
            ),
            json!({}).to_string(),
        )
        .await;
        evidence
            .record(
                &state,
                "former-manager-cannot-directly-reprice",
                response.clone(),
            )
            .await;
        assert_eq!(response["status"], 403);
    }
    edited["charging"]["rate"] = json!(0.9);
    checked(
        &state,
        &mut evidence,
        "former-manager-edits-own-place",
        2,
        "PUT",
        &place_path,
        edited,
    )
    .await;
    let mut after = Vec::new();
    for session in [100, 101, 103, 104] {
        after.push(row(&db.pool, session).await);
    }
    assert_eq!(row(&db.pool, 102).await["cost_usd"], 9.0);
    checked(
        &state,
        &mut evidence,
        "former-manager-can-still-edit-own-profile",
        2,
        "PUT",
        &format!("/v1/cost-profiles/{profile_id}"),
        json!({"rate":1.1}),
    )
    .await;
    let mut recomputed = Vec::new();
    for (vehicle, session) in [(10, 100), (11, 101), (10, 103), (11, 104)] {
        checked(
            &state,
            &mut evidence,
            "legitimate-owner-recomputes-after-revocation",
            1,
            "PATCH",
            &format!(
                "/v1/vehicles/{}/charging-sessions/{}",
                id(vehicle),
                id(session)
            ),
            json!({}),
        )
        .await;
        recomputed.push(row(&db.pool, session).await);
    }
    if let Some(dir) = std::env::var_os("PLACES_EVIDENCE_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("revocation-http-state.json"),
            serde_json::to_string_pretty(&evidence.raw).unwrap(),
        )
        .unwrap();
    }
    db.cleanup().await;
    assert_eq!(before, after, "place edit must not mutate revoked vehicles");
    for session in &recomputed[..2] {
        assert_eq!(session["cost_method"], "unknown");
        assert!(session["cost_profile_id"].is_null());
        assert!(session["cost_usd"].is_null());
    }
    assert_eq!(recomputed[2]["cost_method"], "user_manual");
    assert_eq!(recomputed[2]["cost_usd"], 17.0);
    assert_eq!(recomputed[3]["cost_method"], "user_free");
    assert_eq!(recomputed[3]["cost_usd"], 0.0);
}

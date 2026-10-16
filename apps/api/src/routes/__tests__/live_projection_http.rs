use super::*;
use axum::body::{to_bytes, Body};
use http::Request;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

#[path = "live_projection_fixture.rs"]
mod fixture;

async fn snapshot(pool: &PgPool) -> Value {
    let mut result = serde_json::Map::new();
    for table in [
        "users",
        "vehicles",
        "vehicle_memberships",
        "charge_sessions",
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY id"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        result.insert(table.into(), json!(rows));
    }
    json!(result)
}

#[tokio::test]
#[ignore = "requires isolated ENERGY_CONTRACT_TEST_DATABASE_URL and IDLE_LIVE_REDIS_URL"]
async fn live_projection_http_database_contract() {
    let f = fixture::Fixture::new().await;
    let user = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let vehicle = Uuid::from_u128(10);
    for id in [user, other] {
        sqlx::query("INSERT INTO panda.users(id,email,created_at,updated_at) VALUES ($1,$2,'2026-05-01','2026-05-01')")
            .bind(id).bind(format!("{id}@example.invalid")).execute(&f.pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES ($1,$2,'synthetic-idle-live','R1T','2026-05-01','2026-05-01')")
        .bind(vehicle).bind(user).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicle_memberships(id,user_id,vehicle_id,role,created_at,updated_at) VALUES ($1,$2,$3,'owner','2026-05-01','2026-05-01')")
        .bind(Uuid::from_u128(20)).bind(user).bind(vehicle).execute(&f.pool).await.unwrap();
    let redis_url = std::env::var("IDLE_LIVE_REDIS_URL").unwrap();
    assert_eq!(
        url::Url::parse(&redis_url).unwrap().host_str(),
        Some("redis")
    );
    let redis = redis::Client::open(redis_url.as_str()).unwrap();
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let key = format!("vehicle:{vehicle}:live_session");
    let keys = crate::keys::generate_keys().unwrap();
    let jwt_keys = std::sync::Arc::new(
        crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).unwrap(),
    );
    let token =
        crate::middleware::auth::issue_access_token(user, Some(vehicle), &jwt_keys).unwrap();
    let foreign_token =
        crate::middleware::auth::issue_access_token(other, None, &jwt_keys).unwrap();
    let state = AppState {
        pool: f.pool.clone(),
        redis,
        jwt_keys,
        age_key: "synthetic-idle-live-only".into(),
        config: serde_json::from_value(json!({
            "database_url":f.database_url,
            "redis_url":redis_url,
            "rate_limit":{"auth_read_per_minute":100000,"auth_read_burst":100000}
        }))
        .unwrap(),
        nominatim_cache: Default::default(),
        supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
        resources: Default::default(),
    };
    let app = crate::routes::build_router(state);
    let mut outputs = Vec::new();
    for (index, raw) in [
        None,
        Some(r#"{"power_kw":7.2,"ts":"2026-05-01T00:00:00Z"}"#),
        None,
        Some("{broken"),
        Some(r#"{"power_kw":7.2,"energy_kwh":3.0,"ts":"2026-05-01T00:00:00Z","extra":1}"#),
        Some(r#"{"provenance":{"old":"kept"},"power_kw":null}"#),
        Some(r#"{"power_kw":7.2,"time_remaining_min":40}"#),
    ]
    .into_iter()
    .enumerate()
    {
        if index == 2 {
            sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,created_at) VALUES ($1,$2,'2026-05-01','2026-05-01')")
                .bind(Uuid::from_u128(30)).bind(vehicle).execute(&f.pool).await.unwrap();
        }
        if index == 5 {
            sqlx::query("UPDATE panda.charge_sessions SET parallax_live_power_kw=11.4,parallax_total_charged_kwh=4.3,parallax_pack_energy_kwh=4.0,parallax_thermal_energy_kwh=0.3,parallax_time_remaining_minutes=42,parallax_power_observed_at='2099-01-01',parallax_total_energy_observed_at='2000-01-01',parallax_pack_energy_observed_at='2099-01-01',parallax_thermal_energy_observed_at='2099-01-01',parallax_time_observed_at='2099-01-01' WHERE vehicle_id=$1")
                .bind(vehicle).execute(&f.pool).await.unwrap();
        }
        if index == 6 {
            sqlx::query(
                "UPDATE panda.charge_sessions SET ended_at='2026-05-01 12:00Z' WHERE vehicle_id=$1",
            )
            .bind(vehicle)
            .execute(&f.pool)
            .await
            .unwrap();
        }
        if let Some(raw) = raw {
            let _: () = redis::AsyncCommands::set(&mut conn, &key, raw)
                .await
                .unwrap();
        } else {
            let _: i64 = redis::AsyncCommands::del(&mut conn, &key).await.unwrap();
        }
        for (actor, credential, target) in [
            ("owner", Some(&token), vehicle),
            ("foreign", Some(&foreign_token), vehicle),
            ("anonymous", None, vehicle),
            ("missing", Some(&token), Uuid::from_u128(99)),
        ] {
            let before = snapshot(&f.pool).await;
            let mut request = Request::builder()
                .uri(format!("/v1/vehicles/{target}/live-session"))
                .header("x-request-id", "synthetic-idle-live-request");
            if let Some(token) = credential {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status().as_u16();
            let headers: std::collections::BTreeMap<_, _> = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    let value = if name.as_str().starts_with("x-ratelimit-") {
                        "<limiter>"
                    } else {
                        value.to_str().unwrap()
                    };
                    (name.to_string(), value.to_string())
                })
                .collect();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let after = snapshot(&f.pool).await;
            assert_eq!(before, after);
            let cached: Option<String> = redis::AsyncCommands::get(&mut conn, &key).await.unwrap();
            assert_eq!(cached.as_deref(), raw);
            outputs.push(json!({
                "case":format!("{index}/{actor}"),"status":status,"headers":headers,
                "body":String::from_utf8(body.to_vec()).unwrap(),
                "before":before,"after":after,"redis":cached
            }));
        }
    }
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM public._sqlx_migrations ORDER BY version")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(versions.len(), 32);
    assert!(versions.contains(&1000031));
    let protected: bool = sqlx::query_scalar("SELECT NOT rolsuper AND NOT rolbypassrls AND NOT has_table_privilege(current_user,'panda.trips','DELETE') AND NOT has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE') FROM pg_roles WHERE rolname=current_user")
        .fetch_one(&f.pool).await.unwrap();
    assert!(protected);
    let actual = serde_json::to_string_pretty(&json!({
        "responses":outputs,"migration_versions":versions,"history_protected":protected
    }))
    .unwrap();
    if let Ok(directory) = std::env::var("IDLE_LIVE_CAPTURE") {
        std::fs::write(format!("{directory}/live_projection_http.json"), &actual).unwrap();
    } else {
        assert_eq!(actual, include_str!("live_projection_http.json").trim());
        if let Ok(directory) = std::env::var("IDLE_LIVE_EXPORT") {
            std::fs::write(format!("{directory}/live_projection_http.json"), &actual).unwrap();
        }
    }
    let _: i64 = redis::AsyncCommands::del(&mut conn, &key).await.unwrap();
    f.cleanup().await;
}

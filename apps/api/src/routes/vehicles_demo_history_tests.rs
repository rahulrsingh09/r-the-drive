//! Real migrated databases, restricted application roles and synthetic demo history.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Extension, Router,
};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

use crate::middleware::auth::{issue_access_token, AppState, JwtKeys};

struct DemoFixture {
    state: AppState,
    admin: PgPool,
    database: String,
    role: String,
    owner: Uuid,
}

impl DemoFixture {
    async fn new(production: bool) -> Self {
        let mut url = url::Url::parse(
            &std::env::var("DATABASE_URL").expect("disposable TimescaleDB DATABASE_URL"),
        )
        .unwrap();
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let id = Uuid::new_v4().simple().to_string();
        let role = format!("demo_role_{id}");
        let database = format!("demo_history_{id}");
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE"
            )))
            .await
            .unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        // Only extension installation uses the disposable bootstrap admin.
        // Migrations, policy installation and every route run as the app role.
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA panda AUTHORIZATION {role};
                 CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
                 CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
                 CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
                 CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        let migrated = Self::app_pool(url.as_str(), &role).await;
        crate::db::migrations::run_current_migrations(&migrated)
            .await
            .unwrap();
        migrated.close().await;
        let pool = Self::app_pool(url.as_str(), &role).await;
        let generated = crate::keys::generate_keys().unwrap();
        let jwt_keys = std::sync::Arc::new(
            JwtKeys::new(&generated.jwt_private_pem, &generated.jwt_public_pem).unwrap(),
        );
        let redis_url = std::env::var("REDIS_URL").expect("disposable REDIS_URL");
        let config = serde_json::from_value(json!({
            "database_url": url.as_str(),
            "redis_url": redis_url,
            "panda_env": if production { "production" } else { "development" },
        }))
        .unwrap();
        if production {
            crate::private_deployment::history::protect_history(&pool, &config)
                .await
                .unwrap();
            let restricted: bool = sqlx::query_scalar(
                "SELECT NOT rolsuper
                    AND NOT has_table_privilege(current_user, 'timeseries.telemetry', 'DELETE')
                    AND NOT has_table_privilege(current_user, 'panda.trips', 'DELETE')
                 FROM pg_roles WHERE rolname=current_user",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(
                restricted,
                "fixture must exercise actual production history privileges"
            );
        }
        let owner = sqlx::query_scalar(
            "INSERT INTO panda.users(email, role) VALUES ($1, 'super_user') RETURNING id",
        )
        .bind(format!("demo-owner-{id}@example.invalid"))
        .fetch_one(&pool)
        .await
        .unwrap();
        Self {
            state: AppState {
                pool,
                redis: redis::Client::open(redis_url).unwrap(),
                jwt_keys,
                age_key: generated.age_key,
                config,
                resources: Default::default(),
                nominatim_cache: Default::default(),
                supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            },
            admin,
            database,
            role,
            owner,
        }
    }

    async fn app_pool(url: &str, role: &str) -> PgPool {
        let role = role.to_owned();
        PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |connection, _| {
                let role = role.clone();
                Box::pin(async move {
                    connection
                        .execute(sqlx::AssertSqlSafe(format!("SET ROLE {role}")))
                        .await?;
                    connection
                        .execute("SET search_path = panda,timeseries,public")
                        .await?;
                    Ok(())
                })
            })
            .connect(url)
            .await
            .unwrap()
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let token = issue_access_token(self.owner, None, &self.state.jwt_keys).unwrap();
        let response = Router::new()
            .nest("/v1", super::router())
            .layer(Extension(self.state.jwt_keys.decoding.clone()))
            .with_state(self.state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn cleanup(self) {
        self.state.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER DATABASE {} ALLOW_CONNECTIONS false",
                self.database
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH (FORCE)",
                self.database
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
    }

    async fn history_snapshot(&self, vehicle_id: Uuid) -> Vec<(i64, String)> {
        let mut snapshot = Vec::new();
        for (table, key) in [
            ("panda.vehicles", "id"),
            ("panda.vehicle_memberships", "vehicle_id"),
            ("panda.vehicle_user_settings", "vehicle_id"),
            ("panda.trips", "vehicle_id"),
            ("panda.charge_sessions", "vehicle_id"),
            ("timeseries.telemetry", "vehicle_id"),
            ("panda.vehicle_state_periods", "vehicle_id"),
            ("panda.software_versions", "vehicle_id"),
            ("panda.vehicle_latest_status", "vehicle_id"),
            ("panda.vehicle_runtime_state", "vehicle_id"),
            ("panda.vehicle_images", "vehicle_id"),
            ("panda.rivian_charge_curve_points", "vehicle_id"),
        ] {
            snapshot.push(
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*), md5(coalesce(string_agg(to_jsonb(t)::text, ''
                     ORDER BY to_jsonb(t)::text), '')) FROM {table} t WHERE {key}=$1"
                )))
                .bind(vehicle_id)
                .fetch_one(&self.state.pool)
                .await
                .unwrap(),
            );
        }
        snapshot
    }

    async fn trip_ids(&self, vehicle_id: Uuid) -> Vec<Uuid> {
        sqlx::query_scalar("SELECT id FROM panda.trips WHERE vehicle_id=$1 ORDER BY id")
            .bind(vehicle_id)
            .fetch_all(&self.state.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn fresh_production_demo_does_not_require_history_delete_privileges() {
    let fixture = DemoFixture::new(true).await;
    let mut outcomes = Vec::new();
    for model in ["R1T", "R1S", "R2"] {
        outcomes.push(
            fixture
                .post("/v1/vehicles/demo", json!({ "model": model }))
                .await,
        );
    }
    fixture.cleanup().await;
    for (status, body) in outcomes {
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["created"], true);
        assert_eq!(body["seeded"], true);
        assert_eq!(body["refreshed"], false);
        assert_eq!(body["counts"]["telemetry"], 5_664);
        assert_eq!(body["counts"]["trips"], 31);
        assert_eq!(body["counts"]["charges"], 4);
        assert_eq!(body["counts"]["weather_samples"], 80);
    }
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn production_refresh_rejects_before_any_history_changes() {
    let fixture = DemoFixture::new(true).await;
    let (status, created) = fixture
        .post("/v1/vehicles/demo", json!({"model":"R1T"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let vehicle_id = Uuid::parse_str(created["vehicle_id"].as_str().unwrap()).unwrap();
    let before = fixture.history_snapshot(vehicle_id).await;
    let (status, body) = fixture
        .post(
            &format!("/v1/vehicles/{vehicle_id}/demo/refresh"),
            json!({}),
        )
        .await;
    let after = fixture.history_snapshot(vehicle_id).await;
    fixture.cleanup().await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("protected"));
    assert_eq!(before, after);
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn fresh_seed_refuses_to_overwrite_existing_history() {
    let fixture = DemoFixture::new(true).await;
    let (status, created) = fixture
        .post("/v1/vehicles/demo", json!({"model":"R1T"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let vehicle_id = Uuid::parse_str(created["vehicle_id"].as_str().unwrap()).unwrap();
    let before = fixture.history_snapshot(vehicle_id).await;
    let mut tx = fixture.state.pool.begin().await.unwrap();
    let result = crate::services::demo_seed::seed_new_demo_vehicle(
        &mut tx,
        vehicle_id,
        "R1T",
        chrono::Utc::now(),
    )
    .await;
    assert!(matches!(result, Err(crate::errors::AppError::Conflict(_))));
    tx.commit().await.unwrap();
    let after = fixture.history_snapshot(vehicle_id).await;
    fixture.cleanup().await;
    assert_eq!(before, after);
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn failed_seed_rolls_back_vehicle_memberships_and_partial_history() {
    let fixture = DemoFixture::new(true).await;
    fixture
        .state
        .pool
        .execute("REVOKE INSERT ON timeseries.telemetry FROM CURRENT_USER")
        .await
        .unwrap();
    let (status, body) = fixture
        .post("/v1/vehicles/demo", json!({"model":"R1T"}))
        .await;
    let counts: Vec<i64> = sqlx::query_scalar(
        "SELECT ARRAY[
            (SELECT count(*) FROM panda.vehicles),
            (SELECT count(*) FROM panda.vehicle_memberships),
            (SELECT count(*) FROM panda.vehicle_user_settings),
            (SELECT count(*) FROM panda.trips),
            (SELECT count(*) FROM panda.charge_sessions),
            (SELECT count(*) FROM timeseries.telemetry),
            (SELECT count(*) FROM panda.addresses)
        ]",
    )
    .fetch_one(&fixture.state.pool)
    .await
    .unwrap();
    fixture.cleanup().await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(
        counts,
        vec![0; 7],
        "failed seeding must leave no orphaned state"
    );
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn concurrent_production_demo_creation_returns_one_vehicle() {
    let fixture = DemoFixture::new(true).await;
    let (first, second) = tokio::join!(
        fixture.post("/v1/vehicles/demo", json!({"model":"R1T"})),
        fixture.post("/v1/vehicles/demo", json!({"model":"R1T"})),
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.vehicles")
        .fetch_one(&fixture.state.pool)
        .await
        .unwrap();
    fixture.cleanup().await;
    assert_eq!(first.0, StatusCode::OK, "{}", first.1);
    assert_eq!(second.0, StatusCode::OK, "{}", second.1);
    assert_eq!(first.1["vehicle_id"], second.1["vehicle_id"]);
    assert_ne!(first.1["created"], second.1["created"]);
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn development_refresh_replaces_history_and_preserves_vehicle_settings() {
    let fixture = DemoFixture::new(false).await;
    let (status, created) = fixture
        .post("/v1/vehicles/demo", json!({"model":"R1T"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let vehicle_id = Uuid::parse_str(created["vehicle_id"].as_str().unwrap()).unwrap();
    fixture
        .state
        .pool
        .execute(
            "UPDATE panda.vehicles SET name='Keep this demo name';
         UPDATE panda.vehicle_memberships SET is_default=true",
        )
        .await
        .unwrap();
    let before = fixture.history_snapshot(vehicle_id).await;
    let original_trips = fixture.trip_ids(vehicle_id).await;
    let (status, refreshed) = fixture
        .post(
            &format!("/v1/vehicles/{vehicle_id}/demo/refresh"),
            json!({}),
        )
        .await;
    let after = fixture.history_snapshot(vehicle_id).await;
    let replacement_trips = fixture.trip_ids(vehicle_id).await;
    fixture.cleanup().await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    assert_eq!(refreshed["refreshed"], true);
    assert_eq!(refreshed["vehicle_id"], created["vehicle_id"]);
    assert_eq!(refreshed["counts"], created["counts"]);
    assert_eq!(
        &before[..3],
        &after[..3],
        "vehicle, membership and settings must persist"
    );
    assert_eq!(replacement_trips.len(), 31);
    assert!(replacement_trips
        .iter()
        .all(|id| !original_trips.contains(id)));
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn invalid_refresh_input_preserves_history_even_if_caller_commits() {
    let fixture = DemoFixture::new(false).await;
    let (status, created) = fixture
        .post("/v1/vehicles/demo", json!({"model":"R1T"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let vehicle_id = Uuid::parse_str(created["vehicle_id"].as_str().unwrap()).unwrap();
    let before = fixture.history_snapshot(vehicle_id).await;
    let mut tx = fixture.state.pool.begin().await.unwrap();
    let result = crate::services::demo_seed::seed_demo_vehicle(
        &mut tx,
        vehicle_id,
        "unsupported",
        chrono::Utc::now(),
    )
    .await;
    assert!(matches!(
        result,
        Err(crate::errors::AppError::Validation(_))
    ));
    tx.commit().await.unwrap();
    let after = fixture.history_snapshot(vehicle_id).await;
    fixture.cleanup().await;
    assert_eq!(before, after);
}

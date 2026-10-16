//! Disposable, fully migrated databases and synthetic identities for authorization regressions.

use axum::{body::Body, http::Request, Extension, Router};
use sqlx::{Executor, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

use crate::middleware::auth::{issue_access_token, AppState, AuthUser, JwtKeys};

pub(crate) struct Fixture {
    pub state: AppState,
    admin: PgPool,
    database_name: String,
}

impl Fixture {
    pub async fn new() -> Self {
        let mut url = url::Url::parse(
            &std::env::var("DATABASE_URL").expect("disposable TimescaleDB DATABASE_URL"),
        )
        .unwrap();
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let database_name = format!("authorization_test_{}", Uuid::new_v4().simple());
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE \"{database_name}\""
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database_name}"));
        let pool = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&pool)
            .await
            .unwrap();
        let redis_url = std::env::var("REDIS_URL").expect("disposable REDIS_URL");
        let generated = crate::keys::generate_keys().unwrap();
        let jwt_keys = std::sync::Arc::new(
            JwtKeys::new(&generated.jwt_private_pem, &generated.jwt_public_pem).unwrap(),
        );
        let config = serde_json::from_value(serde_json::json!({
            "database_url": url.as_str(),
            "redis_url": redis_url,
            "panda_env": "development"
        }))
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
            database_name,
        }
    }

    pub async fn user(&self, role: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO panda.users (email, role) VALUES ($1, $2) RETURNING id")
            .bind(format!("{}@example.invalid", Uuid::new_v4()))
            .bind(role)
            .fetch_one(&self.state.pool)
            .await
            .unwrap()
    }

    pub async fn vehicle(&self, owner: Uuid, remote_id: &str) -> Uuid {
        let vehicle_id = sqlx::query_scalar(
            "INSERT INTO panda.vehicles (user_id, rivian_vehicle_id, model)
             VALUES ($1, $2, 'R1T') RETURNING id",
        )
        .bind(owner)
        .bind(remote_id)
        .fetch_one(&self.state.pool)
        .await
        .unwrap();
        self.member(owner, vehicle_id, "owner").await;
        vehicle_id
    }

    pub async fn member(&self, user_id: Uuid, vehicle_id: Uuid, role: &str) {
        sqlx::query(
            "INSERT INTO panda.vehicle_memberships (user_id, vehicle_id, role)
             VALUES ($1, $2, $3)",
        )
        .bind(user_id)
        .bind(vehicle_id)
        .bind(role)
        .execute(&self.state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO panda.vehicle_user_settings (user_id, vehicle_id) VALUES ($1, $2)",
        )
        .bind(user_id)
        .bind(vehicle_id)
        .execute(&self.state.pool)
        .await
        .unwrap();
    }

    pub fn auth(&self, user_id: Uuid) -> AuthUser {
        AuthUser {
            user_id,
            default_vehicle_id: None,
            api_access_level: None,
            api_vehicle_id: None,
        }
    }

    pub async fn sessions(&self, user_id: Uuid, vehicle_id: Uuid) -> (String, String) {
        let api_key = format!("panda_{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO panda.api_keys (user_id, vehicle_id, key_hash, access_level, name)
             VALUES ($1, $2, $3, 'read', 'Synthetic test key')",
        )
        .bind(user_id)
        .bind(vehicle_id)
        .bind(crate::routes::api_keys::hash_api_key(&api_key))
        .execute(&self.state.pool)
        .await
        .unwrap();
        let refresh = format!("synthetic-refresh-{}", Uuid::new_v4());
        use sha2::{Digest, Sha256};
        sqlx::query(
            "WITH family AS (
                 INSERT INTO panda.session_families(user_id) VALUES ($1) RETURNING id
             ) INSERT INTO panda.refresh_tokens (user_id, token_hash, expires_at, family_id)
               SELECT $1, $2, now() + interval '1 day', id FROM family",
        )
        .bind(user_id)
        .bind(Sha256::digest(refresh.as_bytes()).to_vec())
        .execute(&self.state.pool)
        .await
        .unwrap();
        (api_key, refresh)
    }

    pub async fn refresh(&self, token: &str) -> axum::response::Response {
        self.router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/refresh")
                    .header("cookie", format!("refresh_token={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    pub fn token(&self, user_id: Uuid) -> String {
        issue_access_token(user_id, None, &self.state.jwt_keys).unwrap()
    }

    pub fn router(&self) -> Router {
        Router::new()
            .nest(
                "/v1",
                crate::routes::vehicles::router()
                    .merge(crate::routes::users::router())
                    .merge(crate::routes::live::router())
                    .merge(crate::routes::auth::router())
                    .merge(crate::routes::auth::metadata_router()),
            )
            .layer(Extension(self.state.jwt_keys.decoding.clone()))
            .with_state(self.state.clone())
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        self.router()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    pub async fn snapshot(&self) -> Vec<serde_json::Value> {
        let mut rows = Vec::new();
        for table in [
            "users",
            "vehicles",
            "vehicle_memberships",
            "vehicle_user_settings",
            "vehicle_credentials",
            "vehicle_runtime_state",
            "refresh_tokens",
            "session_families",
            "api_keys",
            "vehicle_invites",
            "account_invitations",
            "user_preferences",
        ] {
            rows.push(
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb)
                     FROM panda.{table} t"
                )))
                .fetch_one(&self.state.pool)
                .await
                .unwrap(),
            );
        }
        rows
    }

    pub async fn cleanup(self) {
        self.state.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE \"{}\" WITH (FORCE)",
                self.database_name
            )))
            .await
            .unwrap();
        self.admin.close().await;
    }
}

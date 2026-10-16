use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
pub(super) struct DatabaseFixture {
    admin: PgPool,
    pub(super) pool: PgPool,
    database: String,
    role: String,
}

impl DatabaseFixture {
    pub(super) async fn new() -> Self {
        Self::with_connections(6).await
    }

    pub(super) async fn with_connections(max_connections: u32) -> Self {
        use sqlx::Executor;
        let mut url = url::Url::parse(
            &std::env::var("METRICS_CONTRACT_TEST_DATABASE_URL")
                .expect("explicit disposable METRICS_CONTRACT_TEST_DATABASE_URL required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "metrics_bootstrap");
        assert_eq!(url.path(), "/metrics_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("metrics_role_{suffix}");
        let database = format!("metrics_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_metrics_app'"
        ))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA panda AUTHORIZATION {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SCHEMA panda FROM PUBLIC;
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_metrics_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(max_connections)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({
            "database_url":url.as_str(), "redis_url":"redis://unused", "panda_env":"production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        Self {
            admin,
            pool,
            database,
            role,
        }
    }

    pub(super) async fn cleanup(self) {
        use sqlx::Executor;
        self.pool.close().await;
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
}

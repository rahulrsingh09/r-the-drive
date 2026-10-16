use super::*;
use sqlx::{postgres::PgPoolOptions, Executor};
pub(super) struct Fixture {
    admin: PgPool,
    pub(super) pool: PgPool,
    pub(super) database_url: String,
    database: String,
    role: String,
}

impl Fixture {
    pub(super) async fn new() -> Self {
        let configured = std::env::var("ENERGY_CONTRACT_TEST_DATABASE_URL")
            .expect("explicit disposable ENERGY_CONTRACT_TEST_DATABASE_URL is required");
        let mut url = url::Url::parse(&configured).unwrap();
        assert_eq!(
            url.host_str(),
            Some("db"),
            "use the isolated fixture Docker network"
        );
        assert_eq!(url.path(), "/energy_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let database = format!("energy_contract_{suffix}");
        let role = format!("energy_role_{suffix}");
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {role} LOGIN PASSWORD 'synthetic_energy_role_only'
             NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"
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
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_energy_role_only"))
            .unwrap();
        let migration_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({
            "database_url": url.as_str(), "redis_url": "redis://synthetic.invalid",
            "panda_env": "production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        let protected: bool = sqlx::query_scalar(
            "SELECT NOT rolsuper AND NOT rolbypassrls
             AND NOT has_table_privilege(current_user,'panda.trips','DELETE')
             AND NOT has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE')
             FROM pg_roles WHERE rolname=current_user",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(protected);
        Self {
            database_url: url.to_string(),
            admin,
            pool,
            database,
            role,
        }
    }

    pub(super) async fn cleanup(self) {
        self.pool.close().await;
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

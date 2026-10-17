use anyhow::{bail, Context};
use sqlx::PgPool;

use crate::{config::Config, errors::AppError};

pub const HISTORY_POLICY: &str = include_str!("history.sql");

pub async fn protect_history(pool: &PgPool, config: &Config) -> anyhow::Result<()> {
    if config.is_production() {
        let mut transaction = pool.begin().await?;
        sqlx::raw_sql(HISTORY_POLICY)
            .execute(&mut *transaction)
            .await
            .context("installing vehicle history protection")?;
        let forbidden: bool = sqlx::query_scalar(
            "SELECT bool_or(has_table_privilege(current_user, relation, privilege))
             FROM (VALUES
               ('panda.vehicles', ARRAY['DELETE','TRUNCATE']),
               ('panda.trips', ARRAY['DELETE','TRUNCATE']),
               ('timeseries.telemetry', ARRAY['DELETE','TRUNCATE']),
               ('panda.vehicle_state_periods', ARRAY['DELETE','TRUNCATE']),
               ('panda.software_versions', ARRAY['DELETE','TRUNCATE']),
               ('panda.battery_capacity_snapshots', ARRAY['DELETE','TRUNCATE']),
               ('panda.charge_sessions', ARRAY['TRUNCATE'])
             ) AS policy(relation, privileges)
             CROSS JOIN LATERAL unnest(privileges) AS denied(privilege)",
        )
        .fetch_one(&mut *transaction)
        .await?;
        if forbidden {
            bail!("vehicle history protection requires a database role without overriding delete privileges");
        }
        transaction.commit().await?;
    }
    Ok(())
}

pub fn require_vehicle_deletion_allowed(config: &Config) -> Result<(), AppError> {
    if config.is_production() {
        return Err(AppError::Conflict(
            "Vehicle history is protected on this installation. Vehicle deletion is disabled."
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "__tests__/history_policy_differential.rs"]
mod history_policy_differential;

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{postgres::PgPoolOptions, Executor};

    fn config(environment: &str) -> Config {
        serde_json::from_value(serde_json::json!({
            "database_url": "postgresql://synthetic",
            "redis_url": "redis://synthetic",
            "panda_env": environment
        }))
        .unwrap()
    }

    #[test]
    fn deletion_policy_preserves_development_and_blocks_production() {
        assert!(require_vehicle_deletion_allowed(&config("development")).is_ok());
        assert!(matches!(
            require_vehicle_deletion_allowed(&config("production")),
            Err(AppError::Conflict(_))
        ));
    }

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn authorization_history_owner_cannot_delete_vehicle() {
        let mut fixture = crate::authorization_test_support::Fixture::new().await;
        fixture.state.config.panda_env = Some("production".into());
        let owner = fixture.user("user").await;
        let vehicle = fixture.vehicle(owner, "synthetic-history-vehicle").await;
        let before = fixture.snapshot().await;
        let response = fixture
            .request(
                "DELETE",
                &format!("/v1/vehicles/{vehicle}"),
                &fixture.token(owner),
                serde_json::json!({}),
            )
            .await;
        assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
        assert_eq!(before, fixture.snapshot().await);
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires HISTORY_TEST_DATABASE_URL pointing to disposable TimescaleDB"]
    async fn database_history_blocks_deletion_but_preserves_recording_and_charge_repair() {
        let mut url =
            url::Url::parse(&std::env::var("HISTORY_TEST_DATABASE_URL").unwrap()).unwrap();
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let name = format!("history_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {name} LOGIN NOSUPERUSER"
            )))
            .await
            .unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {name} OWNER {name}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{name}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute("CREATE EXTENSION IF NOT EXISTS timescaledb")
            .await
            .unwrap();
        let role = name.clone();
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .after_connect(move |connection, _| {
                let sql = format!("SET ROLE {role}");
                Box::pin(async move {
                    connection.execute(sqlx::AssertSqlSafe(sql)).await?;
                    Ok(())
                })
            })
            .connect(url.as_str())
            .await
            .unwrap();
        pool.execute("CREATE SCHEMA panda; CREATE SCHEMA timeseries;")
            .await
            .unwrap();
        for table in [
            "vehicles",
            "trips",
            "vehicle_state_periods",
            "software_versions",
            "battery_capacity_snapshots",
            "charge_sessions",
        ] {
            pool.execute(sqlx::AssertSqlSafe(format!(
                "CREATE TABLE panda.{table} (id integer PRIMARY KEY, efficiency numeric);
                 INSERT INTO panda.{table} VALUES (1, 2.73);"
            )))
            .await
            .unwrap();
        }
        pool.execute(
            "ALTER TABLE panda.trips ADD FOREIGN KEY (id) REFERENCES panda.vehicles(id) ON DELETE CASCADE;
             CREATE TABLE timeseries.telemetry (time timestamptz NOT NULL, efficiency numeric);
             SELECT create_hypertable('timeseries.telemetry', 'time');
             INSERT INTO timeseries.telemetry VALUES (now(), 2.73);
             CREATE TABLE panda.rivian_charge_payloads (time timestamptz NOT NULL);
             SELECT create_hypertable('panda.rivian_charge_payloads', 'time');
             SELECT add_retention_policy('panda.rivian_charge_payloads', INTERVAL '90 days');"
        ).await.unwrap();
        assert!(protect_history(&bootstrap, &config("production"))
            .await
            .is_err());
        protect_history(&pool, &config("production")).await.unwrap();
        protect_history(&pool, &config("production")).await.unwrap();
        for table in [
            "panda.vehicles",
            "panda.trips",
            "timeseries.telemetry",
            "panda.vehicle_state_periods",
            "panda.software_versions",
            "panda.battery_capacity_snapshots",
        ] {
            for verb in ["DELETE FROM", "TRUNCATE"] {
                let error = pool
                    .execute(sqlx::AssertSqlSafe(format!("{verb} {table}")))
                    .await
                    .unwrap_err();
                assert_eq!(
                    error.as_database_error().unwrap().code().as_deref(),
                    Some("42501"),
                    "{table}"
                );
            }
            pool.execute(sqlx::AssertSqlSafe(format!(
                "UPDATE {table} SET efficiency = 2.91"
            )))
            .await
            .unwrap();
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table} WHERE efficiency = 2.91"
            )))
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(count, 1);
        }
        pool.execute("INSERT INTO panda.vehicles VALUES (2, 3.01); INSERT INTO panda.trips VALUES (2, 3.01); INSERT INTO timeseries.telemetry VALUES (now(), 3.01)").await.unwrap();
        pool.execute("INSERT INTO panda.charge_sessions VALUES (2, 2.73); DELETE FROM panda.charge_sessions WHERE id = 2;").await.unwrap();
        assert!(pool
            .execute("TRUNCATE panda.charge_sessions")
            .await
            .is_err());
        let scheduled: bool = sqlx::query_scalar("SELECT scheduled FROM timescaledb_information.jobs WHERE hypertable_name = 'rivian_charge_payloads' AND proc_name = 'policy_retention'").fetch_one(&pool).await.unwrap();
        assert!(!scheduled);
        pool.close().await;
        bootstrap.close().await;
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {name} WITH (FORCE)"
            )))
            .await
            .unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {name}")))
            .await
            .unwrap();
    }
}

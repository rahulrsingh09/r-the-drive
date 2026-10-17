#[path = "maintenance_replay/snapshot_scan.rs"]
mod snapshot_scan;
use anyhow::Result;
use chrono::{DateTime, Utc};
use snapshot_scan::SnapshotScan;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    let outcome = async { backfill_software_versions(&pool).await }.await;
    pool.close().await;
    outcome
}

async fn backfill_software_versions(pool: &PgPool) -> Result<()> {
    let mut scan = SnapshotScan::open(pool, "SELECT id FROM panda.vehicles", None).await?;
    loop {
        let page = scan.next::<(uuid::Uuid,)>().await?;
        if page.is_empty() {
            break;
        }
        for (id,) in page {
            rebuild_vehicle_versions(pool, id).await?;
        }
    }
    scan.finish().await?;

    Ok(())
}

async fn rebuild_vehicle_versions(pool: &PgPool, vehicle_id: Uuid) -> Result<()> {
    info!(%vehicle_id, "rebuilding software version history");
    sqlx::query("DELETE FROM panda.software_versions WHERE vehicle_id = $1")
        .bind(vehicle_id)
        .execute(pool)
        .await?;
    let mut scan = SnapshotScan::open(pool,
        "SELECT ts, ota_current_version FROM timeseries.telemetry WHERE vehicle_id = $1 AND ota_current_version IS NOT NULL ORDER BY ts ASC", Some(vehicle_id)).await?;
    let mut active: Option<(String, DateTime<Utc>)> = None;
    loop {
        let page = scan.next::<(DateTime<Utc>, String)>().await?;
        if page.is_empty() {
            break;
        }
        for (timestamp, version) in page {
            if active
                .as_ref()
                .is_some_and(|(current, _)| current == &version)
            {
                continue;
            }
            if let Some((previous, installed)) = active.replace((version, timestamp)) {
                sqlx::query("INSERT INTO panda.software_versions (vehicle_id, version, installed_at, observed_until) VALUES ($1, $2, $3, $4)")
                    .bind(vehicle_id).bind(previous).bind(installed).bind(timestamp).execute(pool).await?;
            }
        }
    }
    scan.finish().await?;
    if let Some((version, installed)) = active {
        sqlx::query("INSERT INTO panda.software_versions (vehicle_id, version, installed_at, observed_until) VALUES ($1, $2, $3, NULL)")
            .bind(vehicle_id).bind(version).bind(installed).execute(pool).await?;
    }
    Ok(())
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires explicit isolated synthetic TimescaleDB"]
    async fn version_runs_preserve_returning_versions_nulls_and_open_tail() {
        let database = snapshot_scan::full_replacement_db::Database::new(
            "CREATE SCHEMA panda; CREATE SCHEMA timeseries;
             CREATE TABLE panda.vehicles(id uuid PRIMARY KEY);
             CREATE TABLE timeseries.telemetry(ts timestamptz, vehicle_id uuid, ota_current_version text);
             CREATE TABLE panda.software_versions(vehicle_id uuid, version text, installed_at timestamptz, observed_until timestamptz);
             INSERT INTO panda.vehicles VALUES('00000000-0000-0000-0000-000000000001');
             INSERT INTO timeseries.telemetry SELECT '2025-01-01Z'::timestamptz+make_interval(secs=>n),'00000000-0000-0000-0000-000000000001',version FROM (VALUES(0,'a'),(1,'a'),(2,NULL),(3,'b'),(4,'a')) x(n,version);"
        ).await;
        let expected = vec![
            ("a".to_owned(), 0, Some(3)),
            ("b".to_owned(), 3, Some(4)),
            ("a".to_owned(), 4, None),
        ];
        for _ in 0..2 {
            backfill_software_versions(&database.pool).await.unwrap();
            let rows: Vec<(String,i32,Option<i32>)> = sqlx::query_as(
                "SELECT version,EXTRACT(EPOCH FROM installed_at-'2025-01-01Z'::timestamptz)::int,EXTRACT(EPOCH FROM observed_until-'2025-01-01Z'::timestamptz)::int FROM panda.software_versions ORDER BY installed_at"
            ).fetch_all(&database.pool).await.unwrap();
            assert_eq!(rows, expected);
        }
        database.cleanup().await;
    }
}

//! Backfill vehicle_state_periods from historical telemetry.
//!
//! Performs a run-length encode over `power_state` (ordered by ts) for each
//! vehicle and inserts the resulting contiguous periods.
//!
//! Run once after deploying migration 0009:
//!   cargo run --bin backfill_state_periods

use anyhow::Result;
use chrono::{DateTime, Utc};
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
    let outcome = async {
        backfill_state_periods(&pool).await?;
        Ok(())
    }
    .await;
    pool.close().await;
    outcome
}

async fn backfill_state_periods(pool: &PgPool) -> Result<()> {
    let mut scan = snapshot_scan::SnapshotScan::open(
        pool,
        "SELECT id, count(*) OVER() AS total FROM panda.vehicles",
        None,
    )
    .await?;
    let mut page = scan.next::<(Uuid, i64)>().await?;
    sqlx::query("DELETE FROM panda.vehicle_state_periods")
        .execute(pool)
        .await?;
    info!(
        "Backfilling state periods for {} vehicles",
        page.first().map_or(0, |(_, total)| *total)
    );
    while !page.is_empty() {
        for (vehicle_id, _) in page {
            process_vehicle(pool, vehicle_id).await?;
        }
        page = scan.next::<(Uuid, i64)>().await?;
    }
    scan.finish().await?;
    info!("State period backfill complete");
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TelemetryRow {
    ts: DateTime<Utc>,
    power_state: Option<String>,
}

fn normalize_state(power_state: Option<&str>) -> String {
    panda_api::db::maintenance::normalized_state(power_state)
}

async fn process_vehicle(pool: &PgPool, vehicle_id: Uuid) -> Result<()> {
    use futures::TryStreamExt;
    use panda_api::db::maintenance::extend_period;
    let mut rows = sqlx::query_as::<_, TelemetryRow>(
        "SELECT ts, power_state FROM timeseries.telemetry WHERE vehicle_id = $1 ORDER BY ts",
    )
    .bind(vehicle_id)
    .fetch(pool);
    let mut active = None;
    let mut periods = Vec::new();
    while let Some(row) = rows.try_next().await? {
        if let Some(closed) = extend_period(
            &mut active,
            row.ts,
            normalize_state(row.power_state.as_deref()),
        ) {
            periods.push(closed);
        }
    }
    periods.extend(active);
    if periods.is_empty() {
        return Ok(());
    }
    info!(vehicle_id = %vehicle_id, periods = periods.len(), "Inserting state periods");
    for period in periods {
        sqlx::query("INSERT INTO panda.vehicle_state_periods (vehicle_id, state, started_at, ended_at) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
            .bind(vehicle_id).bind(period.state).bind(period.start).bind(period.end).execute(pool).await?;
    }
    Ok(())
}

#[allow(dead_code)]
#[path = "maintenance_replay/snapshot_scan.rs"]
mod snapshot_scan;

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires explicit isolated synthetic TimescaleDB"]
    async fn state_runs_preserve_null_samples_aliases_and_gap_boundaries() {
        let database = snapshot_scan::full_replacement_db::Database::new(
            "CREATE SCHEMA panda; CREATE SCHEMA timeseries;
             CREATE TABLE panda.vehicles(id uuid PRIMARY KEY);
             CREATE TABLE timeseries.telemetry(ts timestamptz, vehicle_id uuid, power_state text);
             CREATE TABLE panda.vehicle_state_periods(vehicle_id uuid, state text, started_at timestamptz, ended_at timestamptz);
             INSERT INTO panda.vehicles VALUES('00000000-0000-0000-0000-000000000001');
             INSERT INTO timeseries.telemetry SELECT '2025-01-01Z'::timestamptz+make_interval(secs=>n),'00000000-0000-0000-0000-000000000001',state FROM (VALUES(0,'go'),(600,' DRIVE '),(1201,'go'),(1202,NULL),(1203,'future'),(1204,'sleep')) x(n,state);"
        ).await;
        let expected = vec![
            ("drive".to_owned(), 0, 600),
            ("drive".to_owned(), 1201, 1201),
            ("unknown".to_owned(), 1202, 1203),
            ("sleep".to_owned(), 1204, 1204),
        ];
        for _ in 0..2 {
            backfill_state_periods(&database.pool).await.unwrap();
            let rows: Vec<(String,i32,i32)> = sqlx::query_as(
                "SELECT state,EXTRACT(EPOCH FROM started_at-'2025-01-01Z'::timestamptz)::int,EXTRACT(EPOCH FROM ended_at-'2025-01-01Z'::timestamptz)::int FROM panda.vehicle_state_periods ORDER BY started_at"
            ).fetch_all(&database.pool).await.unwrap();
            assert_eq!(rows, expected);
        }
        database.cleanup().await;
    }
}

//! Backfill trip_id on historical telemetry rows.
//!
//! For each completed trip, stamps all telemetry rows that fall within
//! [started_at, ended_at] and belong to the same vehicle with that trip's UUID.
//!
//! Run once after deploying migration 0011:
//!   cargo run --bin backfill_trip_ids

#[path = "maintenance_replay/snapshot_scan.rs"]
mod snapshot_scan;
use chrono::{DateTime, Utc};
use snapshot_scan::SnapshotScan;
use uuid::Uuid;

use anyhow::Result;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    let outcome = async {
        backfill_trip_ids(&pool).await?;
        Ok(())
    }
    .await;
    pool.close().await;
    outcome
}

async fn backfill_trip_ids(pool: &PgPool) -> Result<()> {
    let mut scan = SnapshotScan::open(pool,
        "SELECT id, vehicle_id, started_at, ended_at, count(*) OVER() AS total FROM panda.trips WHERE ended_at IS NOT NULL ORDER BY started_at", None).await?;
    let mut batch = scan.next::<CompletedWindow>().await?;
    info!(
        "Found {} completed trips to backfill",
        batch.first().map_or(0, |row| row.total)
    );
    while !batch.is_empty() {
        for window in batch {
            let affected = sqlx::query(
                "UPDATE timeseries.telemetry SET trip_id = $1 WHERE vehicle_id = $2 AND ts >= $3 AND ts <= $4 AND trip_id IS NULL")
                .bind(window.id).bind(window.vehicle_id).bind(window.started_at).bind(window.ended_at)
                .execute(pool).await?.rows_affected();
            if affected != 0 {
                info!(trip_id = %window.id, rows = affected, "Stamped telemetry rows");
            }
        }
        batch = scan.next::<CompletedWindow>().await?;
    }
    scan.finish().await?;
    info!("Backfill complete");
    Ok(())
}

#[derive(sqlx::FromRow)]
struct CompletedWindow {
    id: Uuid,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    total: i64,
}

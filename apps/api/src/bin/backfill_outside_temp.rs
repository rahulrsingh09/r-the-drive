//! Queue idempotent route-aware weather enrichment for existing trips.
//!
//! Safe to re-run: completed or already-pending jobs are left untouched.
//!
//! Usage:
//!   DATABASE_URL=... cargo run --bin backfill_outside_temp -- [--vehicle <uuid>]

use anyhow::Result;
use panda_api::services::trip_enrichment::backfill_trip_outside_temps;
use reqwest::Client;
use sqlx::postgres::PgPoolOptions;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
struct Args {
    vehicle_id: Option<Uuid>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    let outcome = async {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;

        let stats = backfill_trip_outside_temps(&pool, &http_client, args.vehicle_id).await?;
        info!(
            vehicle_id = ?args.vehicle_id,
            scanned = stats.scanned,
            queued = stats.filled,
            failed = stats.failed,
            skipped = stats.skipped,
            "trip outside temp backfill complete"
        );
        Ok(())
    }
    .await;
    pool.close().await;
    outcome
}

fn parse_args() -> Result<Args> {
    use panda_api::db::maintenance::{command_options, OptionKind};
    let options = command_options(
        &[OptionKind::Vehicle],
        "Usage: cargo run --bin backfill_outside_temp -- [--vehicle <uuid>]",
        "unknown argument",
    )?;
    Ok(Args {
        vehicle_id: options.vehicle_id,
    })
}

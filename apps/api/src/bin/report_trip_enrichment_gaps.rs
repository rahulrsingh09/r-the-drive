//! Report trip enrichment gaps per vehicle and per day.
//!
//! Usage:
//!   DATABASE_URL=... cargo run --bin report_trip_enrichment_gaps -- [--vehicle <uuid>]

use anyhow::Result;
use panda_api::db::pool::application_pool_options;
use panda_api::services::trip_enrichment::report_trip_enrichment_gaps;
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
    let pool = application_pool_options()
        .max_connections(3)
        .connect(&database_url)
        .await?;
    let outcome = async {
        let rows = report_trip_enrichment_gaps(&pool, args.vehicle_id).await?;
        println!("{}", serde_json::to_string_pretty(&rows)?);
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
        "Usage: cargo run --bin report_trip_enrichment_gaps -- [--vehicle <uuid>]",
        "unknown argument",
    )?;
    Ok(Args {
        vehicle_id: options.vehicle_id,
    })
}

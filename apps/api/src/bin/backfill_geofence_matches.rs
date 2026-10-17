use anyhow::Result;
use panda_api::db::pool::application_pool_options;
use panda_api::services::trip_enrichment::{
    backfill_charge_session_geofence_matches, backfill_trip_geofence_matches,
};
use sqlx::PgPool;
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
    let pool = application_pool_options()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    let outcome = async { backfill_geofence_matches(&pool, args.vehicle_id).await }.await;
    pool.close().await;
    outcome
}

fn parse_args() -> Result<Args> {
    use panda_api::db::maintenance::{command_options, OptionKind};
    let options = command_options(
        &[OptionKind::Vehicle],
        "Usage: cargo run --bin backfill_geofence_matches -- [--vehicle <uuid>]",
        "unknown argument",
    )?;
    Ok(Args {
        vehicle_id: options.vehicle_id,
    })
}

async fn backfill_geofence_matches(pool: &PgPool, vehicle_id: Option<Uuid>) -> Result<()> {
    let trip_stats = backfill_trip_geofence_matches(pool, vehicle_id).await?;
    let session_stats = backfill_charge_session_geofence_matches(pool, vehicle_id).await?;
    info!(
        vehicle_id = ?vehicle_id,
        trip_scanned = trip_stats.scanned,
        trip_filled = trip_stats.filled,
        trip_failed = trip_stats.failed,
        trip_skipped = trip_stats.skipped,
        session_scanned = session_stats.scanned,
        session_filled = session_stats.filled,
        session_failed = session_stats.failed,
        session_skipped = session_stats.skipped,
        "geofence backfill complete"
    );
    Ok(())
}

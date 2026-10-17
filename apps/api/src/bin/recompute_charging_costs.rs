#[path = "maintenance_replay/snapshot_scan.rs"]
mod snapshot_scan;
use anyhow::Result;
use panda_api::services::cost::recompute_charge_session_cost;
use snapshot_scan::SnapshotScan;
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
    let outcome = async { recompute_charging_costs(&pool).await }.await;
    pool.close().await;
    outcome
}

async fn recompute_charging_costs(pool: &PgPool) -> Result<()> {
    let mut scan = SnapshotScan::open(pool, "SELECT id FROM panda.charge_sessions", None).await?;
    loop {
        let page = scan.next::<(uuid::Uuid,)>().await?;
        if page.is_empty() {
            break;
        }
        for (id,) in page {
            recompute_charge_session_cost(pool, id).await?;
        }
    }
    scan.finish().await?;
    info!("charging cost recompute complete");
    Ok(())
}

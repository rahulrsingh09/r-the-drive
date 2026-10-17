#[path = "maintenance_replay/snapshot_scan.rs"]
mod snapshot_scan;
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
    let outcome = async { recompute_trip_energy(&pool).await }.await;
    pool.close().await;
    outcome
}

async fn recompute_trip_energy(pool: &PgPool) -> Result<()> {
    let mut scan = SnapshotScan::open(
        pool,
        r#"SELECT t.id, t.distance_miles, t.soc_start, t.soc_end, t.range_start_mi, t.range_end_mi,
                  t.efficiency_wh_per_mile,
                  v.battery_capacity_wh
           FROM panda.trips t
           JOIN panda.vehicles v ON v.id = t.vehicle_id"#,
        None,
    )
    .await?;

    loop {
        let trips = scan.next::<TripEnergyRow>().await?;
        if trips.is_empty() {
            break;
        }
        for trip in trips {
            let distance = trip.distance_miles.unwrap_or(0.0);
            let (energy_wh, strategy, efficiency_wh_per_mile) =
                panda_api::db::maintenance::TripEnergyInputs {
                    soc_start: trip.soc_start,
                    soc_end: trip.soc_end,
                    capacity: trip.battery_capacity_wh,
                    range_start: trip.range_start_mi,
                    range_end: trip.range_end_mi,
                    distance,
                    efficiency: trip.efficiency_wh_per_mile,
                }
                .columns();

            sqlx::query!(
                r#"UPDATE panda.trips
               SET energy_wh = $2,
                   energy_strategy = $3,
                   efficiency_wh_per_mile = COALESCE($4, efficiency_wh_per_mile)
               WHERE id = $1"#,
                trip.id,
                energy_wh,
                strategy,
                efficiency_wh_per_mile,
            )
            .execute(pool)
            .await?;
        }
    }
    scan.finish().await?;
    info!("trip energy recompute complete");
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TripEnergyRow {
    id: Uuid,
    distance_miles: Option<f64>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    range_start_mi: Option<f64>,
    range_end_mi: Option<f64>,
    efficiency_wh_per_mile: Option<f64>,
    battery_capacity_wh: Option<f64>,
}

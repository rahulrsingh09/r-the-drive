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
    let outcome = async { backfill_trip_odometer_endpoints(&pool).await }.await;
    pool.close().await;
    outcome
}

async fn backfill_trip_odometer_endpoints(pool: &PgPool) -> Result<()> {
    let trips = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        "SELECT id, vehicle_id FROM panda.trips ORDER BY started_at",
    )
    .fetch_all(pool)
    .await?;
    info!(count = trips.len(), "backfilling trip endpoints");
    for (trip, vehicle) in trips {
        sqlx::query(r#"
            WITH first_sample AS (
                SELECT ts, odometer_miles, distance_to_empty_mi FROM timeseries.telemetry
                WHERE trip_id = $1 AND vehicle_id = $2 ORDER BY ts ASC LIMIT 1
            ), last_sample AS (
                SELECT ts, odometer_miles, distance_to_empty_mi FROM timeseries.telemetry
                WHERE trip_id = $1 AND vehicle_id = $2 ORDER BY ts DESC LIMIT 1
            )
            UPDATE panda.trips SET
                start_odometer_mi = COALESCE((SELECT odometer_miles FROM first_sample), start_odometer_mi),
                end_odometer_mi = COALESCE((SELECT odometer_miles FROM last_sample), end_odometer_mi),
                start_position_ts = COALESCE((SELECT ts FROM first_sample), start_position_ts),
                end_position_ts = COALESCE((SELECT ts FROM last_sample), end_position_ts),
                range_start_mi = COALESCE((SELECT distance_to_empty_mi FROM first_sample), range_start_mi),
                range_end_mi = COALESCE((SELECT distance_to_empty_mi FROM last_sample), range_end_mi)
            WHERE id = $1
        "#).bind(trip).bind(vehicle).execute(pool).await?;
    }
    Ok(())
}

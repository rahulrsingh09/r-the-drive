use super::*;

pub(super) async fn last_battery_level_at(
    pool: &PgPool,
    vehicle_id: Uuid,
    at: DateTime<Utc>,
) -> Option<f64> {
    sqlx::query_scalar(
        r#"SELECT battery_level
           FROM timeseries.telemetry
           WHERE vehicle_id = $1 AND ts <= $2 AND battery_level IS NOT NULL
           ORDER BY ts DESC
           LIMIT 1"#,
    )
    .bind(vehicle_id)
    .bind(at)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub(super) async fn persist_trip(
    pool: &PgPool,
    trip: &crate::ingestion::trip_detector::CompletedTripData,
    distance: f64,
) -> anyhow::Result<()> {
    let soc = TripSoc::resolve(pool, trip).await;
    let projection = TripProjection::derive(trip, distance, soc)?;
    let start = trip.points.first();
    let end = trip.points.last();
    let mut tx = pool.begin().await?;
    if distance >= MIN_TRIP_DISTANCE_MILES {
        let mut row = AcquisitionWrite::new("panda.trips")
            .field("id", Stored::Id(trip.trip_id), MergeWrite::Keep)
            .field("vehicle_id", Stored::Id(trip.vehicle_id), MergeWrite::Keep)
            .field(
                "started_at",
                Stored::Time(Some(trip.started_at)),
                MergeWrite::Keep,
            )
            .field(
                "ended_at",
                Stored::Time(Some(trip.ended_at)),
                MergeWrite::Keep,
            )
            .field(
                "duration_seconds",
                Stored::Int(Some(projection.duration)),
                MergeWrite::Keep,
            )
            .field(
                "drive_mode",
                Stored::Text(trip.dominant_drive_mode.as_deref()),
                MergeWrite::Keep,
            )
            .field(
                "energy_strategy",
                Stored::Text(projection.energy_strategy.as_deref()),
                MergeWrite::Keep,
            )
            .field(
                "start_position_ts",
                Stored::Time(start.map(|point| point.ts)),
                MergeWrite::Keep,
            )
            .field(
                "end_position_ts",
                Stored::Time(end.map(|point| point.ts)),
                MergeWrite::Keep,
            )
            .field(
                "route_preview",
                Stored::Json(Some(&projection.route_preview)),
                MergeWrite::Keep,
            )
            .field(
                "route_preview_version",
                Stored::Int(Some(1)),
                MergeWrite::Keep,
            );
        for (column, value) in [
            ("start_lat", start.map(|point| point.lat)),
            ("start_lng", start.map(|point| point.lng)),
            ("end_lat", end.map(|point| point.lat)),
            ("end_lng", end.map(|point| point.lng)),
            ("distance_miles", Some(distance)),
            ("soc_start", soc.start),
            ("soc_end", soc.end),
            ("efficiency_wh_per_mile", projection.efficiency_wh_per_mi),
            ("max_speed_mph", Some(projection.max_speed)),
            ("avg_speed_mph", projection.avg_speed),
            ("start_odometer_mi", trip.start_odometer_mi),
            ("end_odometer_mi", trip.end_odometer_mi),
            ("range_start_mi", trip.range_start_mi),
            ("range_end_mi", trip.range_end_mi),
            ("power_max_kw", trip.power_max_kw),
            ("power_min_kw", trip.power_min_kw),
            ("elevation_gain_m", trip.elevation_gain_m),
            ("elevation_loss_m", trip.elevation_loss_m),
            ("inside_temp_avg_c", trip.inside_temp_avg_c),
            ("outside_temp_c", None),
            ("regen_wh", trip.regen_wh),
            ("energy_wh", projection.energy_wh),
        ] {
            row = row.field(column, Stored::Float(value), MergeWrite::Keep);
        }
        for column in [
            "start_geofence_id",
            "end_geofence_id",
            "start_address_id",
            "end_address_id",
        ] {
            row = row.field(column, Stored::Sql("NULL"), MergeWrite::Keep);
        }
        row.upsert(&mut *tx, "(id)", "").await?;
        let identity: (Uuid, DateTime<Utc>, Option<DateTime<Utc>>) =
            sqlx::query_as("SELECT vehicle_id,started_at,ended_at FROM panda.trips WHERE id=$1")
                .bind(trip.trip_id)
                .fetch_one(&mut *tx)
                .await?;
        anyhow::ensure!(
            (
                identity.0,
                identity.1.timestamp_micros(),
                identity.2.map(|at| at.timestamp_micros())
            ) == (
                trip.vehicle_id,
                trip.started_at.timestamp_micros(),
                Some(trip.ended_at.timestamp_micros())
            ),
            "trip identity collision; retaining completion for investigation"
        );
        AcquisitionWrite::new("panda.weather_enrichment_jobs")
            .field("trip_id", Stored::Id(trip.trip_id), MergeWrite::Keep)
            .upsert(&mut *tx, "(trip_id)", "")
            .await?;
    }
    sqlx::query("DELETE FROM panda.pending_trip_completions WHERE trip_id=$1 AND vehicle_id=$2")
        .bind(trip.trip_id)
        .bind(trip.vehicle_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Clone, Copy)]
struct TripSoc {
    start: Option<f64>,
    end: Option<f64>,
}

impl TripSoc {
    async fn resolve(
        pool: &PgPool,
        trip: &crate::ingestion::trip_detector::CompletedTripData,
    ) -> Self {
        let mut values = [trip.soc_start, trip.soc_end];
        for (value, at) in values.iter_mut().zip([trip.started_at, trip.ended_at]) {
            if value.is_none() {
                *value = last_battery_level_at(pool, trip.vehicle_id, at).await;
            }
        }
        Self {
            start: values[0],
            end: values[1],
        }
    }
}

struct TripProjection {
    duration: i32,
    max_speed: f64,
    avg_speed: Option<f64>,
    energy_wh: Option<f64>,
    energy_strategy: Option<String>,
    efficiency_wh_per_mi: Option<f64>,
    route_preview: serde_json::Value,
}

impl TripProjection {
    fn derive(
        trip: &crate::ingestion::trip_detector::CompletedTripData,
        distance: f64,
        soc: TripSoc,
    ) -> anyhow::Result<Self> {
        let duration = (trip.ended_at - trip.started_at).num_seconds() as i32;
        let energy = compute_trip_energy(
            soc.start,
            soc.end,
            trip.battery_capacity_wh,
            trip.range_start_mi,
            trip.range_end_mi,
            distance,
            None,
        );
        let efficiency = if let Some((wh, _)) = &energy {
            (distance > 0.0).then(|| wh / distance)
        } else {
            match (soc.start, soc.end, trip.battery_capacity_wh) {
                (Some(start), Some(end), Some(capacity)) if distance > 0.0 && start > end => {
                    Some((start - end) / 100.0 * capacity / distance)
                }
                _ => None,
            }
        };
        let coordinates: Vec<_> = trip
            .points
            .iter()
            .map(|point| (point.lat, point.lng))
            .collect();
        Ok(Self {
            duration,
            max_speed: trip
                .points
                .iter()
                .fold(0.0_f64, |max, point| max.max(point.speed_mph)),
            avg_speed: (duration > 0).then(|| distance / (duration as f64 / 3600.0)),
            energy_wh: energy.as_ref().map(|(wh, _)| *wh),
            energy_strategy: energy.map(|(_, strategy)| strategy.to_string()),
            efficiency_wh_per_mi: efficiency,
            route_preview: serde_json::to_value(build_route_preview(&coordinates))?,
        })
    }
}

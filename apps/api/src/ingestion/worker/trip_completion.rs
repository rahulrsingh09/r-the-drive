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
    let TripProjection {
        duration,
        max_speed,
        avg_speed,
        energy_wh,
        energy_strategy,
        efficiency_wh_per_mi,
        route_preview,
    } = TripProjection::derive(trip, distance, soc)?;
    let TripSoc {
        start: soc_start,
        end: soc_end,
    } = soc;
    let start = trip.points.first();
    let end = trip.points.last();
    let start_match = MatchedLocation::none();
    let end_match = MatchedLocation::none();

    let mut tx = pool.begin().await?;
    if distance >= MIN_TRIP_DISTANCE_MILES {
        sqlx::query(
        r#"INSERT INTO panda.trips
           (id, vehicle_id, started_at, ended_at,
            start_lat, start_lng, end_lat, end_lng,
            distance_miles, duration_seconds,
            soc_start, soc_end,
            efficiency_wh_per_mile, max_speed_mph, avg_speed_mph, drive_mode,
            start_odometer_mi, end_odometer_mi,
            start_position_ts, end_position_ts,
            start_geofence_id, end_geofence_id,
            start_address_id, end_address_id,
            range_start_mi, range_end_mi,
            power_max_kw, power_min_kw,
            elevation_gain_m, elevation_loss_m,
            inside_temp_avg_c, outside_temp_c, regen_wh, energy_wh, energy_strategy,
            route_preview, route_preview_version)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,1) ON CONFLICT(id) DO NOTHING"#,
    )
        .bind(trip.trip_id)
        .bind(trip.vehicle_id)
        .bind(trip.started_at)
        .bind(trip.ended_at)
        .bind(start.map(|p| p.lat))
        .bind(start.map(|p| p.lng))
        .bind(end.map(|p| p.lat))
        .bind(end.map(|p| p.lng))
        .bind(distance)
        .bind(duration)
        .bind(soc_start)
        .bind(soc_end)
        .bind(efficiency_wh_per_mi)
        .bind(max_speed)
        .bind(avg_speed)
        .bind(trip.dominant_drive_mode.as_deref())
        .bind(trip.start_odometer_mi)
        .bind(trip.end_odometer_mi)
        .bind(start.map(|p| p.ts))
        .bind(end.map(|p| p.ts))
        .bind(start_match.geofence_id)
        .bind(end_match.geofence_id)
        .bind(start_match.address_id)
        .bind(end_match.address_id)
        .bind(trip.range_start_mi)
        .bind(trip.range_end_mi)
        .bind(trip.power_max_kw)
        .bind(trip.power_min_kw)
        .bind(trip.elevation_gain_m)
        .bind(trip.elevation_loss_m)
        .bind(trip.inside_temp_avg_c)
        .bind(Option::<f64>::None)
        .bind(trip.regen_wh)
        .bind(energy_wh)
        .bind(energy_strategy.as_deref())
        .bind(route_preview)
        .execute(&mut *tx)
        .await?;

        let identity: (Uuid, DateTime<Utc>, Option<DateTime<Utc>>) =
            sqlx::query_as("SELECT vehicle_id,started_at,ended_at FROM panda.trips WHERE id=$1")
                .bind(trip.trip_id)
                .fetch_one(&mut *tx)
                .await?;
        anyhow::ensure!(
            identity.0 == trip.vehicle_id
                && identity.1.timestamp_micros() == trip.started_at.timestamp_micros()
                && identity.2.map(|at| at.timestamp_micros())
                    == Some(trip.ended_at.timestamp_micros()),
            "trip identity collision; retaining completion for investigation"
        );
        sqlx::query(
            "INSERT INTO panda.weather_enrichment_jobs(trip_id) VALUES($1) ON CONFLICT(trip_id) DO NOTHING",
        ).bind(trip.trip_id).execute(&mut *tx).await?;
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

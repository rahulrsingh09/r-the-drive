use super::*;

#[derive(Clone, Copy)]
enum Stage {
    TripCoordinates,
    TripGeofences,
    ChargeCoordinates,
    ChargeLocations,
    TripAddresses,
    WeatherQueue,
}

impl Stage {
    const LOCATIONS: [Self; 4] = [
        Self::TripCoordinates,
        Self::TripGeofences,
        Self::ChargeCoordinates,
        Self::ChargeLocations,
    ];

    async fn execute(self, pool: &PgPool, client: &Client) -> Result<BackfillStats> {
        match self {
            Self::TripCoordinates => backfill_trip_coordinates(pool, None).await,
            Self::TripGeofences => backfill_trip_geofence_matches(pool, None).await,
            Self::ChargeCoordinates => backfill_charge_session_coordinates(pool, None).await,
            Self::ChargeLocations => backfill_charge_session_locations(pool, client, None).await,
            Self::TripAddresses => backfill_trip_addresses(pool, client, None).await,
            Self::WeatherQueue => enqueue_trip_weather_enrichment(pool, None).await,
        }
    }

    fn report_failure(self, error: anyhow::Error) {
        match self {
            Self::TripCoordinates => {
                warn!(error=%error, "trip_enrichment.reconciler_trip_coordinates_failed")
            }
            Self::TripGeofences => {
                warn!(error=%error, "trip_enrichment.reconciler_trip_geofences_failed")
            }
            Self::ChargeCoordinates => {
                warn!(error=%error, "trip_enrichment.reconciler_charge_coordinates_failed")
            }
            Self::ChargeLocations => {
                warn!(error=%error, "trip_enrichment.reconciler_charge_locations_failed")
            }
            Self::TripAddresses | Self::WeatherQueue => {
                unreachable!("summary stages retain their errors")
            }
        }
    }
}

#[cfg(test)]
pub(super) async fn locations(pool: &PgPool, client: &Client) {
    for stage in Stage::LOCATIONS {
        if let Err(error) = stage.execute(pool, client).await {
            stage.report_failure(error);
        }
    }
}

pub(super) async fn cycle(pool: &PgPool, client: &Client) {
    let mut addresses = Ok(BackfillStats::default());
    let mut weather = Ok(BackfillStats::default());
    for stage in Stage::LOCATIONS
        .into_iter()
        .chain([Stage::TripAddresses, Stage::WeatherQueue])
    {
        let result = stage.execute(pool, client).await;
        match stage {
            Stage::TripAddresses => addresses = result,
            Stage::WeatherQueue => weather = result,
            location => {
                if let Err(error) = result {
                    location.report_failure(error);
                }
            }
        }
    }
    match (addresses, weather) {
        (Ok(addresses), Ok(weather)) => info!(
            address_scanned = addresses.scanned,
            address_filled = addresses.filled,
            weather_scanned = weather.scanned,
            weather_queued = weather.filled,
            "trip_enrichment.reconciler_complete"
        ),
        (addresses, weather) => warn!(
            address_error=?addresses.err(), weather_error=?weather.err(),
            "trip_enrichment.reconciler_failed"
        ),
    }
}

#[derive(FromRow)]
struct CoordinateCandidate {
    id: Uuid,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

enum CoordinateDecision {
    Unresolved,
    Repair {
        session_id: Uuid,
        latitude: f64,
        longitude: f64,
    },
}

impl From<CoordinateCandidate> for CoordinateDecision {
    fn from(row: CoordinateCandidate) -> Self {
        match (row.latitude, row.longitude) {
            (Some(latitude), Some(longitude)) => Self::Repair {
                session_id: row.id,
                latitude,
                longitude,
            },
            _ => Self::Unresolved,
        }
    }
}

pub(super) async fn charge_coordinates(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let candidates = sqlx::query_as::<_, CoordinateCandidate>(
        "WITH candidates AS MATERIALIZED (
             SELECT id, vehicle_id, started_at FROM panda.charge_sessions
             WHERE ($1::uuid IS NULL OR vehicle_id=$1)
               AND location_override_mode='automatic'
               AND (location_lat IS NULL OR location_lng IS NULL)
             ORDER BY started_at DESC LIMIT $2
         ) SELECT c.id, fix.latitude, fix.longitude FROM candidates c
         LEFT JOIN LATERAL (
             SELECT latitude, longitude FROM timeseries.telemetry
             WHERE vehicle_id=c.vehicle_id
               AND ts BETWEEN c.started_at-make_interval(secs=>$3) AND c.started_at+make_interval(secs=>$3)
               AND latitude IS NOT NULL AND longitude IS NOT NULL
               AND NOT (latitude=0 AND longitude=0)
             ORDER BY ABS(EXTRACT(EPOCH FROM (ts-c.started_at))) LIMIT 1
         ) fix ON true ORDER BY c.started_at DESC",
    ).bind(vehicle_id).bind(RECONCILIATION_COORDINATE_BATCH_SIZE).bind(TELEMETRY_FIX_WINDOW_SECS)
        .fetch_all(pool).await?;
    let mut stats = BackfillStats {
        scanned: candidates.len(),
        ..BackfillStats::default()
    };
    for decision in candidates.into_iter().map(CoordinateDecision::from) {
        match decision {
            CoordinateDecision::Unresolved => stats.skipped += 1,
            CoordinateDecision::Repair {
                session_id,
                latitude,
                longitude,
            } => {
                sqlx::query(
                    "UPDATE panda.charge_sessions
                     SET location_lat=$2, location_lng=$3,
                         source_location_lat=COALESCE(source_location_lat,$2),
                         source_location_lng=COALESCE(source_location_lng,$3)
                     WHERE id=$1 AND location_override_mode='automatic'
                       AND (location_lat IS NULL OR location_lng IS NULL)",
                )
                .bind(session_id)
                .bind(latitude)
                .bind(longitude)
                .execute(pool)
                .await?;
                // Count a resolved candidate even when a concurrent edit makes its write a no-op.
                stats.filled += 1;
            }
        }
    }
    Ok(stats)
}

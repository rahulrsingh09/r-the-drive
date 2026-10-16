use super::*;

struct RouteCandidate {
    trip: TripMapRow,
    coordinates: Option<Vec<[f64; 2]>>,
}

pub(super) struct RouteAssembly {
    candidates: Vec<RouteCandidate>,
}

impl RouteAssembly {
    pub fn new(rows: Vec<TripMapRow>) -> Self {
        Self {
            candidates: rows
                .into_iter()
                .map(|trip| {
                    let coordinates = trip
                        .route_preview
                        .as_ref()
                        .and_then(|v| serde_json::from_value::<Vec<[f64; 2]>>(v.clone()).ok())
                        .filter(|v| v.len() > 1);
                    RouteCandidate { trip, coordinates }
                })
                .collect(),
        }
    }

    pub async fn resolve(mut self, pool: &sqlx::PgPool) -> Result<Vec<TripMapRoute>, AppError> {
        let missing: Vec<_> = self
            .candidates
            .iter()
            .filter(|c| c.coordinates.is_none())
            .map(|c| c.trip.id)
            .collect();
        if !missing.is_empty() {
            let linked=sqlx::query_as::<_,RouteTelemetryRow>(
                "SELECT trip_id,latitude AS lat,longitude AS lng FROM timeseries.telemetry WHERE trip_id=ANY($1) AND latitude IS NOT NULL AND longitude IS NOT NULL AND NOT (latitude=0 AND longitude=0) ORDER BY trip_id,ts")
                .bind(&missing).fetch_all(pool).await?;
            let mut by_trip = std::collections::HashMap::<Uuid, Vec<(f64, f64)>>::new();
            for point in linked {
                by_trip
                    .entry(point.trip_id)
                    .or_default()
                    .push((point.lat, point.lng));
            }
            for candidate in self
                .candidates
                .iter_mut()
                .filter(|c| c.coordinates.is_none())
            {
                let mut evidence = by_trip.remove(&candidate.trip.id).unwrap_or_default();
                if evidence.len() < 2 {
                    evidence=sqlx::query_as::<_,(f64,f64)>(
                        "SELECT latitude,longitude FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts>=$2 AND ts<=$3 AND latitude IS NOT NULL AND longitude IS NOT NULL AND NOT (latitude=0 AND longitude=0) ORDER BY ts LIMIT 5000")
                        .bind(candidate.trip.vehicle_id).bind(candidate.trip.started_at).bind(candidate.trip.ended_at).fetch_all(pool).await?;
                }
                let preview = build_route_preview(&evidence);
                if preview.len() > 1 {
                    let value = serde_json::to_value(&preview)
                        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
                    sqlx::query("UPDATE panda.trips SET route_preview=$2,route_preview_version=1 WHERE id=$1")
                        .bind(candidate.trip.id).bind(value).execute(pool).await?;
                    candidate.coordinates = Some(preview);
                }
            }
        }
        Ok(self
            .candidates
            .into_iter()
            .filter_map(|c| {
                c.coordinates.map(|coordinates| TripMapRoute {
                    trip_id: c.trip.id,
                    coordinates,
                    tags: c.trip.tags,
                })
            })
            .collect())
    }
}

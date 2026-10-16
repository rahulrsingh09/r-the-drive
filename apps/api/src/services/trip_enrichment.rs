use std::{future::Future, time::Duration};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    db::vehicles::get_vehicle_owner_id,
    services::{cost::recompute_charge_session_cost, geofences::match_geofence, nominatim},
};

const ADDRESS_CACHE_RADIUS_METERS: f64 = 100.0;
const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RECONCILIATION_ADDRESS_BATCH_SIZE: i64 = 100;
const RECONCILIATION_COORDINATE_BATCH_SIZE: i64 = 500;
/// How far from a trip or session boundary a telemetry GPS fix may be and
/// still stand in for the missing coordinates.
const TELEMETRY_FIX_WINDOW_SECS: f64 = 600.0;

/// Recover enrichment skipped while a sanitized restore or provider outage left
/// completed trips without weather or address data. The existing provider
/// lanes still own outbound rate limiting.
pub fn start_reconciliation_worker(pool: PgPool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = match Client::builder().timeout(Duration::from_secs(20)).build() {
            Ok(client) => client,
            Err(error) => {
                warn!(error = %error, "trip_enrichment.reconciler_client_build_failed");
                return;
            }
        };
        loop {
            reconcile_locations(&pool, &client).await;
            let addresses = backfill_trip_addresses(&pool, &client, None).await;
            let weather = enqueue_trip_weather_enrichment(&pool, None).await;
            match (addresses, weather) {
                (Ok(addresses), Ok(weather)) => info!(
                    address_scanned = addresses.scanned,
                    address_filled = addresses.filled,
                    weather_scanned = weather.scanned,
                    weather_queued = weather.filled,
                    "trip_enrichment.reconciler_complete"
                ),
                (addresses, weather) => warn!(
                    address_error = ?addresses.err(),
                    weather_error = ?weather.err(),
                    "trip_enrichment.reconciler_failed"
                ),
            }
            tokio::time::sleep(RECONCILIATION_INTERVAL).await;
        }
    })
}

/// Fill missing coordinates, then saved-place matches, for trips and charge
/// sessions. Each step logs its own failure so one bad step cannot starve the
/// others; the address pass that follows picks up any trips still unresolved.
async fn reconcile_locations(pool: &PgPool, client: &Client) {
    if let Err(error) = backfill_trip_coordinates(pool, None).await {
        warn!(error = %error, "trip_enrichment.reconciler_trip_coordinates_failed");
    }
    if let Err(error) = backfill_trip_geofence_matches(pool, None).await {
        warn!(error = %error, "trip_enrichment.reconciler_trip_geofences_failed");
    }
    if let Err(error) = backfill_charge_session_coordinates(pool, None).await {
        warn!(error = %error, "trip_enrichment.reconciler_charge_coordinates_failed");
    }
    if let Err(error) = backfill_charge_session_locations(pool, client, None).await {
        warn!(error = %error, "trip_enrichment.reconciler_charge_locations_failed");
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillStats {
    pub scanned: usize,
    pub filled: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TripEnrichmentReport {
    pub geofence_matches: BackfillStats,
    pub address_matches: BackfillStats,
    pub outside_temps: BackfillStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchedLocation {
    pub geofence_id: Option<Uuid>,
    pub address_id: Option<Uuid>,
    pub is_home: Option<bool>,
}

impl MatchedLocation {
    pub const fn none() -> Self {
        Self {
            geofence_id: None,
            address_id: None,
            is_home: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct TripEnrichmentDiagnosticsRow {
    pub vehicle_id: Uuid,
    pub trip_day: NaiveDate,
    pub total_trips: i64,
    pub missing_start_address_id: i64,
    pub missing_end_address_id: i64,
    pub missing_start_geofence_id: i64,
    pub missing_end_geofence_id: i64,
    pub missing_outside_temp_c: i64,
    pub missing_start_address_with_coordinates: i64,
    pub missing_end_address_with_coordinates: i64,
    pub start_address_cached_matches: i64,
    pub end_address_cached_matches: i64,
    pub missing_outside_temp_with_coordinates: i64,
    pub missing_outside_temp_unrecoverable_no_start_coordinates: i64,
}

#[derive(Debug, FromRow)]
struct TripLocationRow {
    id: Uuid,
    vehicle_id: Uuid,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
    end_lat: Option<f64>,
    end_lng: Option<f64>,
}

#[derive(Debug, FromRow)]
struct TripAddressRow {
    id: Uuid,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
    start_address_id: Option<Uuid>,
    end_lat: Option<f64>,
    end_lng: Option<f64>,
    end_address_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct TripBoundaryRow {
    id: Uuid,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
    end_lat: Option<f64>,
    end_lng: Option<f64>,
}

#[derive(Debug, FromRow)]
struct ChargeSessionLocationRow {
    id: Uuid,
    vehicle_id: Uuid,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
}

pub async fn enrich_trip_history_for_vehicle(
    pool: &PgPool,
    client: &Client,
    vehicle_id: Uuid,
) -> Result<TripEnrichmentReport> {
    let geofence_matches = backfill_trip_geofence_matches(pool, Some(vehicle_id)).await?;
    let address_matches = backfill_trip_addresses(pool, client, Some(vehicle_id)).await?;
    let outside_temps = enqueue_trip_weather_enrichment(pool, Some(vehicle_id)).await?;

    Ok(TripEnrichmentReport {
        geofence_matches,
        address_matches,
        outside_temps,
    })
}

pub async fn enrich_trip_history_for_vehicle_with_lookup<F, Fut>(
    pool: &PgPool,
    client: &Client,
    vehicle_id: Uuid,
    lookup: F,
) -> Result<TripEnrichmentReport>
where
    F: FnMut(f64, f64, DateTime<Utc>) -> Fut,
    Fut: Future<Output = Option<f64>>,
{
    let geofence_matches = backfill_trip_geofence_matches(pool, Some(vehicle_id)).await?;
    let address_matches = backfill_trip_addresses(pool, client, Some(vehicle_id)).await?;
    let outside_temps =
        backfill_trip_outside_temps_with_lookup(pool, Some(vehicle_id), lookup, None).await?;

    Ok(TripEnrichmentReport {
        geofence_matches,
        address_matches,
        outside_temps,
    })
}

pub async fn report_trip_enrichment_gaps(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<Vec<TripEnrichmentDiagnosticsRow>> {
    let rows = sqlx::query_as::<_, TripEnrichmentDiagnosticsRow>(
        r#"SELECT
               t.vehicle_id,
               (t.started_at AT TIME ZONE 'UTC')::date AS trip_day,
               COUNT(*)::int8 AS total_trips,
               COUNT(*) FILTER (WHERE t.start_address_id IS NULL)::int8 AS missing_start_address_id,
               COUNT(*) FILTER (WHERE t.end_address_id IS NULL)::int8 AS missing_end_address_id,
               COUNT(*) FILTER (WHERE t.start_geofence_id IS NULL)::int8 AS missing_start_geofence_id,
               COUNT(*) FILTER (WHERE t.end_geofence_id IS NULL)::int8 AS missing_end_geofence_id,
               COUNT(*) FILTER (WHERE t.outside_temp_c IS NULL)::int8 AS missing_outside_temp_c,
               COUNT(*) FILTER (
                   WHERE t.start_address_id IS NULL
                     AND t.start_lat IS NOT NULL
                     AND t.start_lng IS NOT NULL
                     AND NOT (t.start_lat = 0 AND t.start_lng = 0)
               )::int8 AS missing_start_address_with_coordinates,
               COUNT(*) FILTER (
                   WHERE t.end_address_id IS NULL
                     AND t.end_lat IS NOT NULL
                     AND t.end_lng IS NOT NULL
                     AND NOT (t.end_lat = 0 AND t.end_lng = 0)
               )::int8 AS missing_end_address_with_coordinates,
               COUNT(*) FILTER (
                   WHERE t.start_address_id IS NULL
                     AND t.start_lat IS NOT NULL
                     AND t.start_lng IS NOT NULL
                     AND NOT (t.start_lat = 0 AND t.start_lng = 0)
                     AND EXISTS (
                         SELECT 1
                         FROM panda.addresses a
                         WHERE earth_distance(
                                   ll_to_earth(a.latitude, a.longitude),
                                   ll_to_earth(t.start_lat, t.start_lng)
                               ) < $2
                     )
               )::int8 AS start_address_cached_matches,
               COUNT(*) FILTER (
                   WHERE t.end_address_id IS NULL
                     AND t.end_lat IS NOT NULL
                     AND t.end_lng IS NOT NULL
                     AND NOT (t.end_lat = 0 AND t.end_lng = 0)
                     AND EXISTS (
                         SELECT 1
                         FROM panda.addresses a
                         WHERE earth_distance(
                                   ll_to_earth(a.latitude, a.longitude),
                                   ll_to_earth(t.end_lat, t.end_lng)
                               ) < $2
                     )
               )::int8 AS end_address_cached_matches,
               COUNT(*) FILTER (
                   WHERE t.outside_temp_c IS NULL
                     AND t.start_lat IS NOT NULL
                     AND t.start_lng IS NOT NULL
                     AND NOT (t.start_lat = 0 AND t.start_lng = 0)
               )::int8 AS missing_outside_temp_with_coordinates,
               COUNT(*) FILTER (
                   WHERE t.outside_temp_c IS NULL
                     AND (
                         t.start_lat IS NULL
                         OR t.start_lng IS NULL
                         OR (t.start_lat = 0 AND t.start_lng = 0)
                     )
               )::int8 AS missing_outside_temp_unrecoverable_no_start_coordinates
           FROM panda.trips t
           WHERE ($1::uuid IS NULL OR t.vehicle_id = $1)
           GROUP BY t.vehicle_id, trip_day
           ORDER BY trip_day DESC, t.vehicle_id"#,
    )
    .bind(vehicle_id)
    .bind(ADDRESS_CACHE_RADIUS_METERS)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn match_geofence_location(
    pool: &PgPool,
    user_id: Uuid,
    lat: f64,
    lon: f64,
) -> Result<MatchedLocation> {
    let matched = match_geofence(pool, user_id, lat, lon).await?;

    Ok(match matched {
        Some(geofence) => MatchedLocation {
            geofence_id: Some(geofence.id),
            address_id: geofence.address_id,
            is_home: Some(geofence.is_home),
        },
        None => MatchedLocation::none(),
    })
}

pub async fn resolve_trip_location(
    pool: &PgPool,
    client: &Client,
    user_id: Uuid,
    lat: f64,
    lon: f64,
) -> Result<MatchedLocation> {
    let geofence_match = match_geofence_location(pool, user_id, lat, lon).await?;
    if geofence_match.geofence_id.is_some() || geofence_match.address_id.is_some() {
        return Ok(geofence_match);
    }

    Ok(MatchedLocation {
        address_id: resolve_address_id(pool, client, lat, lon).await,
        ..MatchedLocation::none()
    })
}

pub async fn resolve_address_id(
    pool: &PgPool,
    client: &Client,
    lat: f64,
    lon: f64,
) -> Option<Uuid> {
    if !has_usable_coordinate_pair(Some(lat), Some(lon)) {
        return None;
    }

    if let Some(id) = find_cached_address_id(pool, lat, lon).await {
        debug!(lat, lon, address_id = %id, "trip_enrichment.address.cache_hit");
        return Some(id);
    }

    reverse_geocode_and_store(pool, client, lat, lon).await
}

pub async fn backfill_trip_geofence_matches(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let trips = sqlx::query_as::<_, TripLocationRow>(
        r#"SELECT id, vehicle_id, start_lat, start_lng, end_lat, end_lng
           FROM panda.trips
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND (
               (start_geofence_id IS NULL AND start_lat IS NOT NULL AND start_lng IS NOT NULL AND NOT (start_lat = 0 AND start_lng = 0))
               OR
               (end_geofence_id IS NULL AND end_lat IS NOT NULL AND end_lng IS NOT NULL AND NOT (end_lat = 0 AND end_lng = 0))
             )"#,
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: trips.len(),
        ..BackfillStats::default()
    };

    for trip in &trips {
        let owner_id = match get_vehicle_owner_id(pool, trip.vehicle_id).await {
            Ok(Some(user_id)) => user_id,
            Ok(None) => {
                warn!(trip_id = %trip.id, "trip_enrichment.geofence.owner_missing");
                stats.failed += 1;
                continue;
            }
            Err(err) => {
                warn!(trip_id = %trip.id, error = %err, "trip_enrichment.geofence.owner_lookup_failed");
                stats.failed += 1;
                continue;
            }
        };

        let start_match = if has_usable_coordinate_pair(trip.start_lat, trip.start_lng) {
            match_geofence_location(
                pool,
                owner_id,
                trip.start_lat.expect("checked"),
                trip.start_lng.expect("checked"),
            )
            .await?
        } else {
            MatchedLocation::none()
        };

        let end_match = if has_usable_coordinate_pair(trip.end_lat, trip.end_lng) {
            match_geofence_location(
                pool,
                owner_id,
                trip.end_lat.expect("checked"),
                trip.end_lng.expect("checked"),
            )
            .await?
        } else {
            MatchedLocation::none()
        };

        if start_match.geofence_id.is_none() && end_match.geofence_id.is_none() {
            stats.skipped += 1;
            continue;
        }

        if let Err(err) = sqlx::query(
            r#"UPDATE panda.trips
               SET start_geofence_id = COALESCE(start_geofence_id, $2),
                   end_geofence_id   = COALESCE(end_geofence_id,   $3),
                   start_address_id  = COALESCE(start_address_id,  $4),
                   end_address_id    = COALESCE(end_address_id,    $5)
               WHERE id = $1"#,
        )
        .bind(trip.id)
        .bind(start_match.geofence_id)
        .bind(end_match.geofence_id)
        .bind(start_match.address_id)
        .bind(end_match.address_id)
        .execute(pool)
        .await
        {
            warn!(trip_id = %trip.id, error = %err, "trip_enrichment.geofence.trip_update_failed");
            stats.failed += 1;
            continue;
        }

        upsert_trip_user_annotation(
            pool,
            trip.id,
            owner_id,
            start_match.geofence_id,
            end_match.geofence_id,
            start_match.address_id,
            end_match.address_id,
        )
        .await?;

        stats.filled += 1;
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        failed = stats.failed,
        skipped = stats.skipped,
        "trip_enrichment.geofence.complete"
    );
    Ok(stats)
}

pub async fn backfill_charge_session_geofence_matches(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let sessions = sqlx::query_as::<_, ChargeSessionLocationRow>(
        r#"SELECT id, vehicle_id, location_lat, location_lng
           FROM panda.charge_sessions
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND geofence_id IS NULL
             AND location_lat IS NOT NULL
             AND location_lng IS NOT NULL
             AND NOT (location_lat = 0 AND location_lng = 0)"#,
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: sessions.len(),
        ..BackfillStats::default()
    };

    for session in &sessions {
        let owner_id = match get_vehicle_owner_id(pool, session.vehicle_id).await {
            Ok(Some(user_id)) => user_id,
            Ok(None) => {
                warn!(session_id = %session.id, "trip_enrichment.charge_geofence.owner_missing");
                stats.failed += 1;
                continue;
            }
            Err(err) => {
                warn!(session_id = %session.id, error = %err, "trip_enrichment.charge_geofence.owner_lookup_failed");
                stats.failed += 1;
                continue;
            }
        };

        let matched = match_geofence_location(
            pool,
            owner_id,
            session.location_lat.expect("checked"),
            session.location_lng.expect("checked"),
        )
        .await?;

        let Some(geofence_id) = matched.geofence_id else {
            stats.skipped += 1;
            continue;
        };

        if let Err(err) = sqlx::query(
            r#"UPDATE panda.charge_sessions
               SET geofence_id = COALESCE(geofence_id, $2),
                   address_id  = COALESCE(address_id,  $3),
                   is_home     = COALESCE(is_home,     $4)
               WHERE id = $1"#,
        )
        .bind(session.id)
        .bind(geofence_id)
        .bind(matched.address_id)
        .bind(matched.is_home)
        .execute(pool)
        .await
        {
            warn!(session_id = %session.id, error = %err, "trip_enrichment.charge_geofence.session_update_failed");
            stats.failed += 1;
            continue;
        }

        upsert_charge_session_user_annotation(
            pool,
            session.id,
            owner_id,
            Some(geofence_id),
            matched.address_id,
            matched.is_home,
        )
        .await?;

        stats.filled += 1;
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        failed = stats.failed,
        skipped = stats.skipped,
        "trip_enrichment.charge_geofence.complete"
    );
    Ok(stats)
}

pub async fn backfill_trip_addresses(
    pool: &PgPool,
    client: &Client,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let trips = sqlx::query_as::<_, TripAddressRow>(
        r#"SELECT id, vehicle_id, started_at, start_lat, start_lng, start_address_id, end_lat, end_lng, end_address_id
           FROM panda.trips
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND (
               (start_address_id IS NULL AND start_lat IS NOT NULL AND start_lng IS NOT NULL AND NOT (start_lat = 0 AND start_lng = 0))
               OR
               (end_address_id IS NULL AND end_lat IS NOT NULL AND end_lng IS NOT NULL AND NOT (end_lat = 0 AND end_lng = 0))
             )
           ORDER BY started_at DESC
           LIMIT $2"#,
    )
    .bind(vehicle_id)
    .bind(RECONCILIATION_ADDRESS_BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: trips.len(),
        ..BackfillStats::default()
    };

    for trip in &trips {
        let owner_id = get_vehicle_owner_id(pool, trip.vehicle_id).await?;
        let mut start_addr = trip.start_address_id;
        let mut end_addr = trip.end_address_id;
        let mut filled_any = false;
        let mut failed_any = false;

        if trip.start_address_id.is_none()
            && has_usable_coordinate_pair(trip.start_lat, trip.start_lng)
        {
            start_addr = resolve_address_id(
                pool,
                client,
                trip.start_lat.expect("checked"),
                trip.start_lng.expect("checked"),
            )
            .await;
            if start_addr.is_some() {
                filled_any = true;
            } else {
                warn!(
                    trip_id = %trip.id,
                    lat = trip.start_lat,
                    lng = trip.start_lng,
                    started_at = %trip.started_at,
                    "trip_enrichment.address.start_lookup_failed"
                );
                failed_any = true;
            }
        }

        if trip.end_address_id.is_none() && has_usable_coordinate_pair(trip.end_lat, trip.end_lng) {
            end_addr = resolve_address_id(
                pool,
                client,
                trip.end_lat.expect("checked"),
                trip.end_lng.expect("checked"),
            )
            .await;
            if end_addr.is_some() {
                filled_any = true;
            } else {
                warn!(
                    trip_id = %trip.id,
                    lat = trip.end_lat,
                    lng = trip.end_lng,
                    started_at = %trip.started_at,
                    "trip_enrichment.address.end_lookup_failed"
                );
                failed_any = true;
            }
        }

        if !filled_any {
            if failed_any {
                stats.failed += 1;
            } else {
                stats.skipped += 1;
            }
            continue;
        }

        if let Err(err) = sqlx::query(
            r#"UPDATE panda.trips
               SET start_address_id = COALESCE($2, start_address_id),
                   end_address_id   = COALESCE($3, end_address_id)
               WHERE id = $1"#,
        )
        .bind(trip.id)
        .bind(start_addr)
        .bind(end_addr)
        .execute(pool)
        .await
        {
            warn!(trip_id = %trip.id, error = %err, "trip_enrichment.address.trip_update_failed");
            stats.failed += 1;
            continue;
        }

        if let Some(user_id) = owner_id {
            upsert_trip_user_annotation(pool, trip.id, user_id, None, None, start_addr, end_addr)
                .await?;
        }

        stats.filled += 1;
        if failed_any {
            stats.failed += 1;
        }
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        failed = stats.failed,
        skipped = stats.skipped,
        "trip_enrichment.address.complete"
    );
    Ok(stats)
}

/// Closest usable GPS fix the vehicle reported within
/// `TELEMETRY_FIX_WINDOW_SECS` of `at`. Charging frames usually carry no GPS,
/// so a session that opens without coordinates borrows the surrounding fix.
pub async fn nearest_telemetry_fix(
    pool: &PgPool,
    vehicle_id: Uuid,
    at: DateTime<Utc>,
) -> Result<Option<(f64, f64)>> {
    let fix = sqlx::query_as::<_, (f64, f64)>(
        r#"SELECT latitude, longitude
           FROM timeseries.telemetry
           WHERE vehicle_id = $1
             AND ts BETWEEN $2 - make_interval(secs => $3) AND $2 + make_interval(secs => $3)
             AND latitude IS NOT NULL
             AND longitude IS NOT NULL
             AND NOT (latitude = 0 AND longitude = 0)
           ORDER BY ABS(EXTRACT(EPOCH FROM (ts - $2)))
           LIMIT 1"#,
    )
    .bind(vehicle_id)
    .bind(at)
    .bind(TELEMETRY_FIX_WINDOW_SECS)
    .fetch_optional(pool)
    .await?;
    Ok(fix)
}

/// Recover trip start/end coordinates that were never recorded from the
/// telemetry fixes around the trip boundaries.
pub async fn backfill_trip_coordinates(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let trips = sqlx::query_as::<_, TripBoundaryRow>(
        r#"SELECT id, vehicle_id, started_at, ended_at, start_lat, start_lng, end_lat, end_lng
           FROM panda.trips
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND (start_lat IS NULL OR start_lng IS NULL OR end_lat IS NULL OR end_lng IS NULL)
           ORDER BY started_at DESC
           LIMIT $2"#,
    )
    .bind(vehicle_id)
    .bind(RECONCILIATION_COORDINATE_BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: trips.len(),
        ..BackfillStats::default()
    };

    for trip in &trips {
        let start = if trip.start_lat.is_none() || trip.start_lng.is_none() {
            nearest_telemetry_fix(pool, trip.vehicle_id, trip.started_at).await?
        } else {
            None
        };
        let end = if trip.end_lat.is_none() || trip.end_lng.is_none() {
            nearest_telemetry_fix(pool, trip.vehicle_id, trip.ended_at).await?
        } else {
            None
        };

        if start.is_none() && end.is_none() {
            stats.skipped += 1;
            continue;
        }

        sqlx::query(
            r#"UPDATE panda.trips
               SET start_lat = CASE WHEN start_lat IS NULL OR start_lng IS NULL THEN COALESCE($2, start_lat) ELSE start_lat END,
                   start_lng = CASE WHEN start_lat IS NULL OR start_lng IS NULL THEN COALESCE($3, start_lng) ELSE start_lng END,
                   end_lat   = CASE WHEN end_lat IS NULL OR end_lng IS NULL THEN COALESCE($4, end_lat) ELSE end_lat END,
                   end_lng   = CASE WHEN end_lat IS NULL OR end_lng IS NULL THEN COALESCE($5, end_lng) ELSE end_lng END
               WHERE id = $1"#,
        )
        .bind(trip.id)
        .bind(start.map(|(lat, _)| lat))
        .bind(start.map(|(_, lng)| lng))
        .bind(end.map(|(lat, _)| lat))
        .bind(end.map(|(_, lng)| lng))
        .execute(pool)
        .await?;
        stats.filled += 1;
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        skipped = stats.skipped,
        "trip_enrichment.trip_coordinates.complete"
    );
    Ok(stats)
}

/// Give automatic-location charge sessions without coordinates the telemetry
/// fix recorded around their start.
pub async fn backfill_charge_session_coordinates(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let sessions = sqlx::query_as::<_, (Uuid, Uuid, DateTime<Utc>)>(
        r#"SELECT id, vehicle_id, started_at
           FROM panda.charge_sessions
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND location_override_mode = 'automatic'
             AND (location_lat IS NULL OR location_lng IS NULL)
           ORDER BY started_at DESC
           LIMIT $2"#,
    )
    .bind(vehicle_id)
    .bind(RECONCILIATION_COORDINATE_BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: sessions.len(),
        ..BackfillStats::default()
    };

    for (session_id, session_vehicle_id, started_at) in sessions {
        let Some((lat, lng)) = nearest_telemetry_fix(pool, session_vehicle_id, started_at).await?
        else {
            stats.skipped += 1;
            continue;
        };

        sqlx::query(
            r#"UPDATE panda.charge_sessions
               SET location_lat = $2,
                   location_lng = $3,
                   source_location_lat = COALESCE(source_location_lat, $2),
                   source_location_lng = COALESCE(source_location_lng, $3)
               WHERE id = $1
                 AND location_override_mode = 'automatic'
                 AND (location_lat IS NULL OR location_lng IS NULL)"#,
        )
        .bind(session_id)
        .bind(lat)
        .bind(lng)
        .execute(pool)
        .await?;
        stats.filled += 1;
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        skipped = stats.skipped,
        "trip_enrichment.charge_coordinates.complete"
    );
    Ok(stats)
}

/// Resolve the saved place (geofence, home flag, address) for automatic-location
/// charge sessions that have coordinates but no place yet, reverse-geocoding
/// when no geofence matches. Sessions the user edited by hand are left alone.
pub async fn backfill_charge_session_locations(
    pool: &PgPool,
    client: &Client,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let sessions = sqlx::query_as::<_, ChargeSessionLocationRow>(
        r#"SELECT id, vehicle_id, location_lat, location_lng
           FROM panda.charge_sessions
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND location_override_mode = 'automatic'
             AND geofence_id IS NULL
             AND address_id IS NULL
             AND location_lat IS NOT NULL
             AND location_lng IS NOT NULL
             AND NOT (location_lat = 0 AND location_lng = 0)
           ORDER BY started_at DESC
           LIMIT $2"#,
    )
    .bind(vehicle_id)
    .bind(RECONCILIATION_ADDRESS_BATCH_SIZE)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: sessions.len(),
        ..BackfillStats::default()
    };

    for session in &sessions {
        let owner_id = match get_vehicle_owner_id(pool, session.vehicle_id).await {
            Ok(Some(user_id)) => user_id,
            Ok(None) => {
                warn!(session_id = %session.id, "trip_enrichment.charge_location.owner_missing");
                stats.failed += 1;
                continue;
            }
            Err(err) => {
                warn!(session_id = %session.id, error = %err, "trip_enrichment.charge_location.owner_lookup_failed");
                stats.failed += 1;
                continue;
            }
        };

        let matched = resolve_trip_location(
            pool,
            client,
            owner_id,
            session.location_lat.expect("checked"),
            session.location_lng.expect("checked"),
        )
        .await?;

        if matched.geofence_id.is_none() && matched.address_id.is_none() {
            warn!(
                session_id = %session.id,
                lat = session.location_lat,
                lng = session.location_lng,
                "trip_enrichment.charge_location.lookup_failed"
            );
            stats.failed += 1;
            continue;
        }

        if let Err(err) = sqlx::query(
            r#"UPDATE panda.charge_sessions
               SET geofence_id = COALESCE(geofence_id, $2),
                   address_id  = COALESCE(address_id,  $3),
                   is_home     = COALESCE(is_home,     $4)
               WHERE id = $1
                 AND location_override_mode = 'automatic'"#,
        )
        .bind(session.id)
        .bind(matched.geofence_id)
        .bind(matched.address_id)
        .bind(matched.is_home)
        .execute(pool)
        .await
        {
            warn!(session_id = %session.id, error = %err, "trip_enrichment.charge_location.session_update_failed");
            stats.failed += 1;
            continue;
        }

        upsert_charge_session_user_annotation(
            pool,
            session.id,
            owner_id,
            matched.geofence_id,
            matched.address_id,
            matched.is_home,
        )
        .await?;

        // A newly matched geofence can carry a cost profile (e.g. home rates).
        if matched.geofence_id.is_some() {
            if let Err(err) = recompute_charge_session_cost(pool, session.id).await {
                warn!(session_id = %session.id, error = %err, "trip_enrichment.charge_location.cost_recompute_failed");
            }
        }

        stats.filled += 1;
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        failed = stats.failed,
        skipped = stats.skipped,
        "trip_enrichment.charge_location.complete"
    );
    Ok(stats)
}

pub async fn backfill_trip_outside_temps(
    pool: &PgPool,
    _client: &Client,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    enqueue_trip_weather_enrichment(pool, vehicle_id).await
}

pub async fn enqueue_trip_weather_enrichment(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
) -> Result<BackfillStats> {
    let trip_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id
           FROM panda.trips
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND started_at IS NOT NULL
             AND ended_at IS NOT NULL
           ORDER BY started_at DESC"#,
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: trip_ids.len(),
        ..BackfillStats::default()
    };
    for trip_id in trip_ids {
        let result = sqlx::query(
            r#"INSERT INTO panda.weather_enrichment_jobs
                 (trip_id, status, attempts, next_attempt_at, last_error, updated_at, completed_at)
               VALUES ($1, 'pending', 0, now(), NULL, now(), NULL)
               ON CONFLICT (trip_id) DO UPDATE SET
                 status = 'pending', attempts = 0, next_attempt_at = now(),
                 last_error = NULL, updated_at = now(), completed_at = NULL
               WHERE panda.weather_enrichment_jobs.status IN ('failed', 'unavailable')"#,
        )
        .bind(trip_id)
        .execute(pool)
        .await?;
        if result.rows_affected() > 0 {
            stats.filled += 1;
        } else {
            stats.skipped += 1;
        }
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        queued = stats.filled,
        skipped = stats.skipped,
        "trip_enrichment.weather_jobs.queued"
    );
    Ok(stats)
}

pub async fn backfill_trip_outside_temps_with_lookup<F, Fut>(
    pool: &PgPool,
    vehicle_id: Option<Uuid>,
    mut lookup: F,
    polite_delay: Option<Duration>,
) -> Result<BackfillStats>
where
    F: FnMut(f64, f64, DateTime<Utc>) -> Fut,
    Fut: Future<Output = Option<f64>>,
{
    let trips = sqlx::query_as::<_, TripAddressRow>(
        r#"SELECT id, vehicle_id, started_at, start_lat, start_lng, start_address_id, end_lat, end_lng, end_address_id
           FROM panda.trips
           WHERE ($1::uuid IS NULL OR vehicle_id = $1)
             AND outside_temp_c IS NULL
             AND start_lat IS NOT NULL
             AND start_lng IS NOT NULL
             AND NOT (start_lat = 0 AND start_lng = 0)
           ORDER BY started_at DESC"#,
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;

    let mut stats = BackfillStats {
        scanned: trips.len(),
        ..BackfillStats::default()
    };

    for trip in &trips {
        let temp_c = lookup(
            trip.start_lat.expect("checked"),
            trip.start_lng.expect("checked"),
            trip.started_at,
        )
        .await;

        match temp_c {
            Some(value) => {
                if let Err(err) =
                    sqlx::query("UPDATE panda.trips SET outside_temp_c = $2 WHERE id = $1")
                        .bind(trip.id)
                        .bind(value)
                        .execute(pool)
                        .await
                {
                    warn!(trip_id = %trip.id, error = %err, "trip_enrichment.outside_temp.update_failed");
                    stats.failed += 1;
                } else {
                    stats.filled += 1;
                    debug!(trip_id = %trip.id, outside_temp_c = value, "trip_enrichment.outside_temp.updated");
                }
            }
            None => {
                warn!(
                    trip_id = %trip.id,
                    lat = trip.start_lat,
                    lng = trip.start_lng,
                    started_at = %trip.started_at,
                    "trip_enrichment.outside_temp.lookup_failed"
                );
                stats.failed += 1;
            }
        }

        if let Some(delay) = polite_delay {
            tokio::time::sleep(delay).await;
        }
    }

    info!(
        vehicle_id = ?vehicle_id,
        scanned = stats.scanned,
        filled = stats.filled,
        failed = stats.failed,
        skipped = stats.skipped,
        "trip_enrichment.outside_temp.complete"
    );
    Ok(stats)
}

async fn find_cached_address_id(pool: &PgPool, lat: f64, lon: f64) -> Option<Uuid> {
    sqlx::query_scalar(
        r#"SELECT id
           FROM panda.addresses
           WHERE earth_distance(
                     ll_to_earth(latitude, longitude),
                     ll_to_earth($1, $2)
                 ) < $3
           ORDER BY earth_distance(
                     ll_to_earth(latitude, longitude),
                     ll_to_earth($1, $2)
                 )
           LIMIT 1"#,
    )
    .bind(lat)
    .bind(lon)
    .bind(ADDRESS_CACHE_RADIUS_METERS)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

async fn reverse_geocode_and_store(
    pool: &PgPool,
    client: &Client,
    lat: f64,
    lon: f64,
) -> Option<Uuid> {
    let raw = match nominatim::reverse(pool, client, lat, lon).await {
        Ok(Some(raw)) => raw,
        Ok(None) => return None,
        Err(crate::errors::AppError::ExternalConnectionDisabled(_)) => return None,
        Err(err) => {
            warn!(error = %err, "trip_enrichment.address.request_failed");
            return None;
        }
    };

    let display_name = raw.get("display_name")?.as_str()?.to_string();
    let osm_id = raw.get("osm_id").and_then(|value| value.as_i64());
    let address = raw.get("address").and_then(|value| value.as_object());

    let road = address
        .and_then(|row| {
            row.get("road")
                .or_else(|| row.get("pedestrian"))
                .or_else(|| row.get("footway"))
        })
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let city = address
        .and_then(|row| {
            row.get("city")
                .or_else(|| row.get("town"))
                .or_else(|| row.get("village"))
        })
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let state = address
        .and_then(|row| row.get("state"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let postcode = address
        .and_then(|row| row.get("postcode"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let country = address
        .and_then(|row| row.get("country"))
        .and_then(|value| value.as_str())
        .map(str::to_string);

    let result: Result<Uuid, sqlx::Error> = if let Some(osm_id) = osm_id {
        sqlx::query_scalar(
            r#"INSERT INTO panda.addresses
               (display_name, osm_id, latitude, longitude, road, city, state, postcode, country, raw)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (osm_id) DO UPDATE SET
                 display_name = EXCLUDED.display_name,
                 road         = EXCLUDED.road,
                 city         = EXCLUDED.city,
                 state        = EXCLUDED.state,
                 postcode     = EXCLUDED.postcode,
                 country      = EXCLUDED.country,
                 raw          = EXCLUDED.raw
               RETURNING id"#,
        )
        .bind(display_name)
        .bind(osm_id)
        .bind(lat)
        .bind(lon)
        .bind(road)
        .bind(city)
        .bind(state)
        .bind(postcode)
        .bind(country)
        .bind(raw)
        .fetch_one(pool)
        .await
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO panda.addresses
               (display_name, latitude, longitude, road, city, state, postcode, country, raw)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id"#,
        )
        .bind(display_name)
        .bind(lat)
        .bind(lon)
        .bind(road)
        .bind(city)
        .bind(state)
        .bind(postcode)
        .bind(country)
        .bind(raw)
        .fetch_one(pool)
        .await
    };

    match result {
        Ok(id) => {
            info!(lat, lon, address_id = %id, "trip_enrichment.address.stored");
            Some(id)
        }
        Err(err) => {
            warn!(error = %err, lat, lon, "trip_enrichment.address.insert_failed");
            None
        }
    }
}

async fn upsert_trip_user_annotation(
    pool: &PgPool,
    trip_id: Uuid,
    user_id: Uuid,
    start_geofence_id: Option<Uuid>,
    end_geofence_id: Option<Uuid>,
    start_address_id: Option<Uuid>,
    end_address_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO panda.trip_user_annotations
           (trip_id, user_id, start_geofence_id, end_geofence_id, start_address_id, end_address_id, matched_at)
           VALUES ($1, $2, $3, $4, $5, $6, now())
           ON CONFLICT (trip_id, user_id) DO UPDATE
           SET start_geofence_id = COALESCE(EXCLUDED.start_geofence_id, panda.trip_user_annotations.start_geofence_id),
               end_geofence_id = COALESCE(EXCLUDED.end_geofence_id, panda.trip_user_annotations.end_geofence_id),
               start_address_id = COALESCE(EXCLUDED.start_address_id, panda.trip_user_annotations.start_address_id),
               end_address_id = COALESCE(EXCLUDED.end_address_id, panda.trip_user_annotations.end_address_id),
               matched_at = now(),
               updated_at = now()"#,
    )
    .bind(trip_id)
    .bind(user_id)
    .bind(start_geofence_id)
    .bind(end_geofence_id)
    .bind(start_address_id)
    .bind(end_address_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn upsert_charge_session_user_annotation(
    pool: &PgPool,
    charge_session_id: Uuid,
    user_id: Uuid,
    geofence_id: Option<Uuid>,
    address_id: Option<Uuid>,
    is_home: Option<bool>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO panda.charge_session_user_annotations
           (charge_session_id, user_id, geofence_id, address_id, is_home, computed_at)
           VALUES ($1, $2, $3, $4, $5, now())
           ON CONFLICT (charge_session_id, user_id) DO UPDATE
           SET geofence_id = COALESCE(EXCLUDED.geofence_id, panda.charge_session_user_annotations.geofence_id),
               address_id = COALESCE(EXCLUDED.address_id, panda.charge_session_user_annotations.address_id),
               is_home = COALESCE(EXCLUDED.is_home, panda.charge_session_user_annotations.is_home),
               computed_at = now(),
               updated_at = now()"#,
    )
    .bind(charge_session_id)
    .bind(user_id)
    .bind(geofence_id)
    .bind(address_id)
    .bind(is_home)
    .execute(pool)
    .await?;

    Ok(())
}

fn has_usable_coordinate_pair(lat: Option<f64>, lng: Option<f64>) -> bool {
    matches!((lat, lng), (Some(lat), Some(lng)) if !(lat == 0.0 && lng == 0.0))
}

#[cfg(test)]
mod history_enrichment_contract_tests {
    use super::*;
    use crate::authorization_test_support::Fixture;
    use chrono::TimeZone;

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap()
    }
    async fn history_fixture() -> Fixture {
        let mut fixture = Fixture::new().await;
        let options = fixture
            .state
            .pool
            .connect_options()
            .as_ref()
            .clone()
            .options([("search_path", "panda,timeseries,public")]);
        fixture.state.pool.close().await;
        fixture.state.pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        fixture
    }
    async fn trip(pool: &PgPool, vehicle: Uuid, at: DateTime<Utc>) -> Uuid {
        sqlx::query_scalar("INSERT INTO panda.trips (vehicle_id,started_at,ended_at,distance_miles,energy_wh,route_preview) VALUES ($1,$2,$2+interval '30 minutes',12.5,5000,'[[1,2],[3,4]]') RETURNING id").bind(vehicle).bind(at).fetch_one(pool).await.unwrap()
    }
    async fn address(pool: &PgPool, lat: f64, lng: f64) -> Uuid {
        sqlx::query_scalar("INSERT INTO panda.addresses (display_name,latitude,longitude) VALUES ('Synthetic address',$1,$2) RETURNING id").bind(lat).bind(lng).fetch_one(pool).await.unwrap()
    }
    async fn geofence(pool: &PgPool, owner: Uuid, addr: Uuid) -> Uuid {
        sqlx::query_scalar("INSERT INTO panda.geofences (user_id,name,latitude,longitude,radius_m,address_id,is_home,is_work) VALUES ($1,'Synthetic place',30,-97,100,$2,true,false) RETURNING id").bind(owner).bind(addr).fetch_one(pool).await.unwrap()
    }

    #[test]
    fn history_coordinate_contract_distinguishes_absence_from_legacy_invalids() {
        for (lat, lng, expected) in [
            (None, Some(1.), false),
            (Some(1.), None, false),
            (Some(0.), Some(0.), false),
            (Some(0.), Some(1.), true),
            (Some(91.), Some(181.), true),
            (Some(f64::NAN), Some(f64::INFINITY), true),
        ] {
            assert_eq!(has_usable_coordinate_pair(lat, lng), expected);
        }
        assert_eq!(
            MatchedLocation::none(),
            MatchedLocation {
                geofence_id: None,
                address_id: None,
                is_home: None
            }
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated disposable DATABASE_URL and REDIS_URL"]
    async fn history_addresses_batch_cache_miss_and_partial_stats() {
        let fixture = history_fixture().await;
        let pool = &fixture.state.pool;
        let owner = fixture.user("admin").await;
        let vehicle = fixture.vehicle(owner, "history-addresses").await;
        let other = fixture.vehicle(owner, "history-addresses-other").await;
        let cached = address(pool, 30., -97.).await;
        let other_trip = trip(pool, other, start()).await;
        sqlx::query("UPDATE panda.trips SET start_lat=30,start_lng=-97 WHERE id=$1")
            .bind(other_trip)
            .execute(pool)
            .await
            .unwrap();
        for i in 0..101 {
            let id = trip(pool, vehicle, start() + chrono::Duration::hours(i)).await;
            sqlx::query("UPDATE panda.trips SET start_lat=30,start_lng=-97,end_lat=40,end_lng=-100 WHERE id=$1").bind(id).execute(pool).await.unwrap();
        }
        let client = Client::new();
        assert_eq!(
            resolve_address_id(pool, &client, 30., -97.).await,
            Some(cached)
        );
        assert_eq!(resolve_address_id(pool, &client, 0., 0.).await, None);
        assert_eq!(resolve_address_id(pool, &client, 40., -100.).await, None);
        let stats = backfill_trip_addresses(pool, &client, Some(vehicle))
            .await
            .unwrap();
        assert_eq!(
            stats,
            BackfillStats {
                scanned: 100,
                filled: 100,
                failed: 100,
                skipped: 0
            }
        );
        let written:i64=sqlx::query_scalar("SELECT count(*) FROM panda.trip_user_annotations WHERE user_id=$1 AND start_address_id=$2").bind(owner).bind(cached).fetch_one(pool).await.unwrap();
        assert_eq!(written, 100);
        let untouched: Option<Uuid> =
            sqlx::query_scalar("SELECT start_address_id FROM panda.trips WHERE id=$1")
                .bind(other_trip)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(untouched, None);
        let again = backfill_trip_addresses(pool, &client, Some(vehicle))
            .await
            .unwrap();
        assert_eq!(
            again,
            BackfillStats {
                scanned: 100,
                filled: 0,
                failed: 100,
                skipped: 0
            }
        );
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires isolated disposable DATABASE_URL and REDIS_URL"]
    async fn history_geofences_follow_membership_owner_and_preserve_existing_fields() {
        let fixture = history_fixture().await;
        let pool = &fixture.state.pool;
        let owner = fixture.user("admin").await;
        let stranger = fixture.user("user").await;
        let vehicle = fixture.vehicle(owner, "history-geofences").await;
        let cached = address(pool, 30., -97.).await;
        let own = geofence(pool, owner, cached).await;
        let foreign = geofence(pool, stranger, cached).await;
        let id = trip(pool, vehicle, start()).await;
        sqlx::query("UPDATE panda.trips SET start_lat=30,start_lng=-97,end_lat=30,end_lng=-97,end_geofence_id=$2 WHERE id=$1").bind(id).bind(foreign).execute(pool).await.unwrap();
        let stats = backfill_trip_geofence_matches(pool, Some(vehicle))
            .await
            .unwrap();
        assert_eq!(
            stats,
            BackfillStats {
                scanned: 1,
                filled: 1,
                failed: 0,
                skipped: 0
            }
        );
        let row:(Uuid,Uuid,Uuid)=sqlx::query_as("SELECT start_geofence_id,end_geofence_id,start_address_id FROM panda.trips WHERE id=$1").bind(id).fetch_one(pool).await.unwrap();
        assert_eq!(row, (own, foreign, cached));
        let annotation:(Uuid,Uuid)=sqlx::query_as("SELECT start_geofence_id,end_geofence_id FROM panda.trip_user_annotations WHERE trip_id=$1 AND user_id=$2").bind(id).bind(owner).fetch_one(pool).await.unwrap();
        assert_eq!(annotation, (own, own));
        sqlx::query("DELETE FROM panda.vehicle_memberships WHERE vehicle_id=$1")
            .bind(vehicle)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE panda.trips SET start_geofence_id=NULL WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            backfill_trip_geofence_matches(pool, Some(vehicle))
                .await
                .unwrap(),
            BackfillStats {
                scanned: 1,
                filled: 0,
                failed: 1,
                skipped: 0
            }
        );
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires isolated disposable DATABASE_URL and REDIS_URL"]
    async fn history_interrupted_trip_sparse_coordinates_and_batch_boundary() {
        let fixture = history_fixture().await;
        let pool = &fixture.state.pool;
        let owner = fixture.user("admin").await;
        let vehicle = fixture.vehicle(owner, "history-coordinates").await;
        let other = fixture.vehicle(owner, "history-coordinates-other").await;
        sqlx::query("INSERT INTO timeseries.telemetry (vehicle_id,ts,latitude,longitude) VALUES ($1,$3+interval '10 minutes',30,-97),($2,$3,40,-100),($1,$3+interval '40 minutes 1 second',31,-98)").bind(vehicle).bind(other).bind(start()).execute(pool).await.unwrap();
        assert_eq!(
            nearest_telemetry_fix(pool, vehicle, start()).await.unwrap(),
            Some((30., -97.))
        );
        assert_eq!(
            nearest_telemetry_fix(pool, vehicle, start() + chrono::Duration::minutes(30))
                .await
                .unwrap(),
            None
        );
        for _ in 0..501 {
            let id = trip(pool, vehicle, start()).await;
            sqlx::query("UPDATE panda.trips SET start_lat=99,end_lat=0,end_lng=0 WHERE id=$1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        assert_eq!(
            backfill_trip_coordinates(pool, Some(vehicle))
                .await
                .unwrap(),
            BackfillStats {
                scanned: 500,
                filled: 500,
                failed: 0,
                skipped: 0
            }
        );
        assert_eq!(
            backfill_trip_coordinates(pool, Some(vehicle))
                .await
                .unwrap(),
            BackfillStats {
                scanned: 1,
                filled: 1,
                failed: 0,
                skipped: 0
            }
        );
        let unchanged:i64=sqlx::query_scalar("SELECT count(*) FROM panda.trips WHERE vehicle_id=$1 AND start_lat=30 AND start_lng=-97 AND end_lat=0 AND end_lng=0 AND distance_miles=12.5 AND energy_wh=5000 AND route_preview='[[1,2],[3,4]]'").bind(vehicle).fetch_one(pool).await.unwrap();
        assert_eq!(unchanged, 501);
        let id = trip(pool, vehicle, start() + chrono::Duration::days(2)).await;
        assert_eq!(
            backfill_trip_coordinates(pool, Some(vehicle))
                .await
                .unwrap(),
            BackfillStats {
                scanned: 1,
                filled: 0,
                failed: 0,
                skipped: 1
            }
        );
        let day = report_trip_enrichment_gaps(pool, Some(vehicle))
            .await
            .unwrap();
        assert_eq!(day[0].total_trips, 1);
        assert_eq!(
            day[0].missing_outside_temp_unrecoverable_no_start_coordinates,
            1
        );
        let session:Uuid=sqlx::query_scalar("INSERT INTO panda.charge_sessions (vehicle_id,started_at,source) VALUES ($1,$2,'telemetry') RETURNING id").bind(vehicle).bind(start()).fetch_one(pool).await.unwrap();
        assert_eq!(
            backfill_charge_session_coordinates(pool, Some(vehicle))
                .await
                .unwrap()
                .filled,
            1
        );
        sqlx::query("UPDATE panda.charge_sessions SET location_override_mode='none',location_lat=NULL,location_lng=NULL WHERE id=$1").bind(session).execute(pool).await.unwrap();
        assert_eq!(
            backfill_charge_session_coordinates(pool, Some(vehicle))
                .await
                .unwrap()
                .scanned,
            0
        );
        assert_eq!(
            backfill_charge_session_locations(pool, &Client::new(), Some(vehicle))
                .await
                .unwrap()
                .scanned,
            0
        );
        assert!(sqlx::query("DELETE FROM panda.trips WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await
            .is_ok());
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires isolated disposable DATABASE_URL and REDIS_URL"]
    async fn history_weather_queue_and_lookup_error_contracts() {
        let fixture = history_fixture().await;
        let pool = &fixture.state.pool;
        let owner = fixture.user("admin").await;
        let vehicle = fixture.vehicle(owner, "history-queue").await;
        for i in 0..501 {
            trip(pool, vehicle, start() + chrono::Duration::hours(i)).await;
        }
        let other = fixture.vehicle(owner, "history-queue-other").await;
        trip(pool, other, start()).await;
        assert_eq!(
            enqueue_trip_weather_enrichment(pool, Some(vehicle))
                .await
                .unwrap(),
            BackfillStats {
                scanned: 501,
                filled: 501,
                failed: 0,
                skipped: 0
            }
        );
        assert_eq!(
            enqueue_trip_weather_enrichment(pool, Some(vehicle))
                .await
                .unwrap(),
            BackfillStats {
                scanned: 501,
                filled: 0,
                failed: 0,
                skipped: 501
            }
        );
        sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='failed',attempts=4,last_error='synthetic'").execute(pool).await.unwrap();
        assert_eq!(
            enqueue_trip_weather_enrichment(pool, Some(vehicle))
                .await
                .unwrap()
                .filled,
            501
        );
        let reset:i64=sqlx::query_scalar("SELECT count(*) FROM panda.weather_enrichment_jobs WHERE status='pending' AND attempts=0 AND last_error IS NULL").fetch_one(pool).await.unwrap();
        assert_eq!(reset, 501);
        let id = trip(pool, vehicle, start() - chrono::Duration::days(1)).await;
        sqlx::query("UPDATE panda.trips SET start_lat=30,start_lng=-97 WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            backfill_trip_outside_temps_with_lookup(
                pool,
                Some(vehicle),
                |_, _, _| async { None },
                None
            )
            .await
            .unwrap(),
            BackfillStats {
                scanned: 1,
                filled: 0,
                failed: 1,
                skipped: 0
            }
        );
        assert_eq!(
            backfill_trip_outside_temps_with_lookup(
                pool,
                Some(vehicle),
                |lat, lng, at| {
                    assert_eq!(
                        (lat, lng, at),
                        (30., -97., start() - chrono::Duration::days(1))
                    );
                    async { Some(-12.5) }
                },
                Some(Duration::ZERO)
            )
            .await
            .unwrap()
            .filled,
            1
        );
        assert_eq!(
            backfill_trip_outside_temps_with_lookup(
                pool,
                Some(vehicle),
                |_, _, _| async { panic!("populated value must not be queried") },
                None
            )
            .await
            .unwrap()
            .scanned,
            0
        );
        pool.close().await;
        assert!(enqueue_trip_weather_enrichment(pool, Some(vehicle))
            .await
            .is_err());
        fixture.cleanup().await;
    }
}

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    private_deployment::weather::{
        self, WeatherBudgetError, WeatherBusyError, WeatherFetchError, WeatherPausedError,
        DAILY_REQUEST_BUDGET,
    },
    services::external_connections::{self as connections, ConnectionSettingsRow},
};

const SAMPLE_INTERVAL_SECONDS: i64 = 15 * 60;
const MAX_LOCATIONS_PER_REQUEST: usize = 1;

#[derive(Debug, FromRow)]
struct TripInfo {
    id: Uuid,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
struct RoutePoint {
    ts: DateTime<Utc>,
    lat: f64,
    lng: f64,
}

#[derive(Debug, Clone)]
struct TargetSample {
    sampled_at: DateTime<Utc>,
    elapsed_seconds: i32,
    cell_key: String,
    provider_lat: f64,
    provider_lng: f64,
}

#[derive(Debug, Clone)]
struct WeatherCell {
    key: String,
    lat: f64,
    lng: f64,
}

#[derive(Debug, Clone, FromRow)]
struct RawOutsideTemp {
    ts: DateTime<Utc>,
    outside_temp_c: f64,
}

enum EnrichTripOutcome {
    Unavailable,
    Complete { provider_called: bool },
}

pub fn start_worker(pool: PgPool, age_key: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(error = %error, "weather_enrichment.client_build_failed");
                return;
            }
        };

        loop {
            match process_next_job(&pool, &client, &age_key).await {
                Ok(true) => tokio::time::sleep(Duration::from_secs(1)).await,
                Ok(false) => tokio::time::sleep(Duration::from_secs(30)).await,
                Err(error) => {
                    tracing::warn!(error = %error, "weather_enrichment.worker_iteration_failed");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        }
    })
}

pub async fn enqueue(pool: &PgPool, trip_id: Uuid) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO panda.weather_enrichment_jobs
             (trip_id, status, attempts, next_attempt_at, last_error, updated_at, completed_at)
           VALUES ($1, 'pending', 0, now(), NULL, now(), NULL)
           ON CONFLICT (trip_id) DO UPDATE SET
             status = CASE WHEN panda.weather_enrichment_jobs.status = 'succeeded' THEN 'succeeded' ELSE 'pending' END,
             next_attempt_at = now(), last_error = NULL, updated_at = now()"#,
    )
    .bind(trip_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn process_next_job(pool: &PgPool, client: &reqwest::Client, age_key: &str) -> Result<bool> {
    weather::recover_interrupted_jobs(pool).await?;
    let settings = match connections::require_enabled(pool, connections::OPEN_METEO).await {
        Ok(settings) => settings,
        Err(crate::errors::AppError::ExternalConnectionDisabled(_)) => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    let request_count: i32 = sqlx::query_scalar(
        r#"SELECT CASE WHEN usage_date = (now() AT TIME ZONE 'UTC')::date THEN request_count ELSE 0 END
           FROM panda.external_connection_activity WHERE connection_id = $1"#,
    )
    .bind(connections::OPEN_METEO)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0);
    if request_count >= DAILY_REQUEST_BUDGET {
        return Ok(false);
    }

    let trip_id = sqlx::query_scalar::<_, Uuid>(
        r#"UPDATE panda.weather_enrichment_jobs SET
             status = 'running', attempts = attempts + 1, updated_at = now()
           WHERE trip_id = (
             SELECT trip_id FROM panda.weather_enrichment_jobs
             WHERE status IN ('pending', 'failed') AND attempts < 5 AND next_attempt_at <= now()
             ORDER BY next_attempt_at, created_at
             FOR UPDATE SKIP LOCKED LIMIT 1
           )
           RETURNING trip_id"#,
    )
    .fetch_optional(pool)
    .await?;
    let Some(trip_id) = trip_id else {
        return Ok(false);
    };

    match enrich_trip(pool, client, age_key, &settings, trip_id).await {
        Ok(EnrichTripOutcome::Complete { provider_called }) => {
            sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='succeeded', last_error=NULL, completed_at=now(), updated_at=now() WHERE trip_id=$1")
                .bind(trip_id).execute(pool).await?;
            if provider_called {
                connections::record_success(pool, connections::OPEN_METEO).await;
            }
        }
        Ok(EnrichTripOutcome::Unavailable) => {
            sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='unavailable', last_error=NULL, completed_at=now(), updated_at=now() WHERE trip_id=$1")
                .bind(trip_id).execute(pool).await?;
        }
        Err(error) => {
            if error.downcast_ref::<WeatherPausedError>().is_some() {
                sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='pending', attempts=GREATEST(attempts-1, 0), next_attempt_at=now(), last_error=NULL, updated_at=now() WHERE trip_id=$1")
                    .bind(trip_id).execute(pool).await?;
                return Ok(true);
            }
            if error.downcast_ref::<WeatherBusyError>().is_some() {
                sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='pending', attempts=GREATEST(attempts-1,0), next_attempt_at=now()+interval '1 minute', last_error=NULL, updated_at=now() WHERE trip_id=$1")
                    .bind(trip_id).execute(pool).await?;
                return Ok(true);
            }
            if error.downcast_ref::<WeatherBudgetError>().is_some() {
                sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='failed', attempts=GREATEST(attempts-1, 0), next_attempt_at=date_trunc('day', now()) + interval '1 day', last_error='Daily weather request budget reached', updated_at=now() WHERE trip_id=$1")
                    .bind(trip_id).execute(pool).await?;
                return Ok(true);
            }
            connections::record_failure(pool, connections::OPEN_METEO, &error.to_string()).await;
            let attempts: i32 = sqlx::query_scalar(
                "SELECT attempts FROM panda.weather_enrichment_jobs WHERE trip_id=$1",
            )
            .bind(trip_id)
            .fetch_one(pool)
            .await?;
            let exponential_backoff = 60_i64
                .saturating_mul(2_i64.pow(attempts.min(8) as u32))
                .min(21_600);
            let retry_after = error
                .downcast_ref::<WeatherFetchError>()
                .and_then(|error| error.retry_after_seconds)
                .unwrap_or(0);
            let backoff_seconds = exponential_backoff.max(retry_after).min(86_400);
            sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='failed', next_attempt_at=now()+make_interval(secs=>$2), last_error='Weather provider request failed', updated_at=now() WHERE trip_id=$1")
                .bind(trip_id).bind(backoff_seconds as f64).execute(pool).await?;
            tracing::warn!(trip_id = %trip_id, attempts, "weather_enrichment.trip_failed");
        }
    }
    Ok(true)
}

async fn enrich_trip(
    pool: &PgPool,
    client: &reqwest::Client,
    _age_key: &str,
    _settings: &ConnectionSettingsRow,
    trip_id: Uuid,
) -> Result<EnrichTripOutcome> {
    let trip = sqlx::query_as::<_, TripInfo>(
        "SELECT id, vehicle_id, started_at, ended_at, start_lat, start_lng FROM panda.trips WHERE id=$1",
    )
    .bind(trip_id)
    .fetch_optional(pool)
    .await?
    .context("trip not found")?;

    let route = sqlx::query_as::<_, RoutePoint>(
        r#"SELECT ts, latitude AS lat, longitude AS lng
           FROM timeseries.telemetry
           WHERE vehicle_id=$1 AND ts >= $2 AND ts <= $3
             AND latitude IS NOT NULL AND longitude IS NOT NULL
             AND latitude <> 0 AND longitude <> 0
             AND (trip_id=$4 OR trip_id IS NULL)
           ORDER BY ts"#,
    )
    .bind(trip.vehicle_id)
    .bind(trip.started_at)
    .bind(trip.ended_at)
    .bind(trip.id)
    .fetch_all(pool)
    .await?;

    if trip.ended_at < trip.started_at
        || trip.ended_at - trip.started_at > chrono::Duration::days(1)
    {
        anyhow::bail!("trip exceeds weather sample bounds");
    }
    let targets = build_targets(&trip, &route, false);
    if targets.is_empty() {
        return Ok(EnrichTripOutcome::Unavailable);
    }

    let raw = sqlx::query_as::<_, RawOutsideTemp>(
        r#"SELECT ts, outside_temp_c FROM timeseries.telemetry
           WHERE vehicle_id=$1 AND ts >= $2 AND ts <= $3 AND outside_temp_c IS NOT NULL
           ORDER BY ts"#,
    )
    .bind(trip.vehicle_id)
    .bind(trip.started_at)
    .bind(trip.ended_at)
    .fetch_all(pool)
    .await?;

    let weather_targets = targets
        .iter()
        .filter(|target| nearest_raw_temp(&raw, target.sampled_at).is_none())
        .cloned()
        .collect::<Vec<_>>();
    let mut cells = unique_weather_cells(&weather_targets);
    let provider_called = !cells.is_empty();
    cells.shuffle(&mut rand::thread_rng());

    let mut hourly_by_cell = HashMap::<String, Value>::new();
    for chunk in cells.chunks(MAX_LOCATIONS_PER_REQUEST) {
        match connections::require_enabled(pool, connections::OPEN_METEO).await {
            Ok(_) => {}
            Err(crate::errors::AppError::ExternalConnectionDisabled(_)) => {
                return Err(WeatherPausedError.into());
            }
            Err(error) => return Err(error.into()),
        }
        let payloads =
            fetch_weather_chunk(pool, client, chunk, trip.started_at, trip.ended_at).await?;
        sqlx::query("UPDATE panda.weather_enrichment_jobs SET updated_at=now() WHERE trip_id=$1 AND status='running'")
            .bind(trip_id).execute(pool).await?;
        for (cell, payload) in chunk.iter().zip(payloads) {
            hourly_by_cell.insert(cell.key.clone(), payload);
        }
        if cells.len() > MAX_LOCATIONS_PER_REQUEST {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    let mut enriched = Vec::with_capacity(targets.len());
    for target in weather_targets {
        let Some(payload) = hourly_by_cell.get(&target.cell_key) else {
            continue;
        };
        let Some(temp) = pick_closest_temp(payload, target.sampled_at) else {
            continue;
        };
        enriched.push((target, temp));
    }
    if enriched.is_empty() && raw.is_empty() {
        anyhow::bail!("weather provider returned no usable temperatures");
    }

    let weather_by_time = enriched
        .iter()
        .map(|(target, temperature)| (target.sampled_at, *temperature))
        .collect::<HashMap<_, _>>();
    let mut summary_values = Vec::with_capacity(targets.len());
    let mut raw_used = 0usize;
    let mut weather_used = 0usize;
    for target in &targets {
        if let Some(raw_temp) = nearest_raw_temp(&raw, target.sampled_at) {
            raw_used += 1;
            summary_values.push((target.sampled_at, raw_temp));
        } else if let Some(weather_temp) = weather_by_time.get(&target.sampled_at) {
            weather_used += 1;
            summary_values.push((target.sampled_at, *weather_temp));
        }
    }
    let source = if raw_used == 0 && weather_used > 0 {
        "open_meteo"
    } else if weather_used == 0 && raw_used > 0 {
        "vehicle"
    } else {
        "mixed"
    };
    let summary = time_weighted_average(&summary_values).context("weather summary unavailable")?;

    store_estimates(pool, trip.id, enriched, summary, source).await?;
    Ok(EnrichTripOutcome::Complete { provider_called })
}

async fn store_estimates(
    pool: &PgPool,
    trip_id: Uuid,
    enriched: Vec<(TargetSample, f64)>,
    summary: f64,
    source: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM panda.trip_weather_samples WHERE trip_id=$1")
        .bind(trip_id)
        .execute(&mut *tx)
        .await?;
    for (target, temp) in enriched {
        sqlx::query(
            r#"INSERT INTO panda.trip_weather_samples
                 (trip_id, sampled_at, elapsed_seconds, provider_latitude, provider_longitude, temperature_c)
               VALUES ($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(trip_id).bind(target.sampled_at).bind(target.elapsed_seconds)
        .bind(target.provider_lat).bind(target.provider_lng).bind(temp)
        .execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE panda.trips SET outside_temp_c=$2, outside_temp_source=$3 WHERE id=$1")
        .bind(trip_id)
        .bind(summary)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn nearest_raw_temp(raw: &[RawOutsideTemp], sampled_at: DateTime<Utc>) -> Option<f64> {
    raw.iter()
        .min_by_key(|row| (row.ts - sampled_at).num_seconds().abs())
        .filter(|row| (row.ts - sampled_at).num_seconds().abs() <= SAMPLE_INTERVAL_SECONDS / 2)
        .map(|row| row.outside_temp_c)
}

fn build_targets(trip: &TripInfo, route: &[RoutePoint], exact: bool) -> Vec<TargetSample> {
    let mut times = Vec::new();
    let mut cursor = trip.started_at;
    while cursor < trip.ended_at {
        times.push(cursor);
        cursor += chrono::Duration::seconds(SAMPLE_INTERVAL_SECONDS);
    }
    if times.last().copied() != Some(trip.ended_at) {
        times.push(trip.ended_at);
    }

    times
        .into_iter()
        .filter_map(|sampled_at| {
            let point = route
                .iter()
                .min_by_key(|point| (point.ts - sampled_at).num_seconds().abs());
            let (lat, lng) = point
                .map(|point| (point.lat, point.lng))
                .or_else(|| trip.start_lat.zip(trip.start_lng))?;
            let decimals: usize = if exact { 5 } else { 2 };
            let factor = 10_f64.powi(decimals as i32);
            let provider_lat = (lat * factor).round() / factor;
            let provider_lng = (lng * factor).round() / factor;
            Some(TargetSample {
                sampled_at,
                elapsed_seconds: (sampled_at - trip.started_at).num_seconds().max(0) as i32,
                cell_key: format!("{provider_lat:.decimals$},{provider_lng:.decimals$}"),
                provider_lat,
                provider_lng,
            })
        })
        .collect()
}

fn unique_weather_cells(targets: &[TargetSample]) -> Vec<WeatherCell> {
    let mut cells_by_key = HashMap::<String, WeatherCell>::new();
    for target in targets {
        cells_by_key
            .entry(target.cell_key.clone())
            .or_insert_with(|| WeatherCell {
                key: target.cell_key.clone(),
                lat: target.provider_lat,
                lng: target.provider_lng,
            });
    }
    cells_by_key.into_values().collect()
}

async fn fetch_weather_chunk(
    pool: &PgPool,
    _client: &reqwest::Client,
    cells: &[WeatherCell],
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<Vec<Value>> {
    let [cell] = cells else {
        anyhow::bail!("Weather requests require one location");
    };
    Ok(vec![
        weather::fetch(pool, cell.lat, cell.lng, started_at, ended_at).await?,
    ])
}

fn pick_closest_temp(body: &Value, target: DateTime<Utc>) -> Option<f64> {
    let times = body.pointer("/hourly/time")?.as_array()?;
    let temps = body.pointer("/hourly/temperature_2m")?.as_array()?;
    times
        .iter()
        .zip(temps)
        .filter_map(|(time, temp)| {
            let raw = time.as_str()?;
            let parsed = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
                .ok()?
                .and_utc();
            Some(((parsed - target).num_seconds().abs(), temp.as_f64()?))
        })
        .min_by_key(|(distance, _)| *distance)
        .filter(|(distance, _)| *distance <= 3600)
        .map(|(_, temp)| temp)
}

fn time_weighted_average(values: &[(DateTime<Utc>, f64)]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    if values.len() == 1 {
        return Some(values[0].1);
    }
    let mut weighted = 0.0;
    let mut seconds = 0.0;
    for pair in values.windows(2) {
        let duration = (pair[1].0 - pair[0].0).num_seconds().max(0) as f64;
        weighted += ((pair[0].1 + pair[1].1) / 2.0) * duration;
        seconds += duration;
    }
    if seconds > 0.0 {
        Some(weighted / seconds)
    } else {
        Some(values[0].1)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_targets, pick_closest_temp, time_weighted_average, unique_weather_cells, RoutePoint,
        TripInfo,
    };
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn authorization_weather_estimates_preserve_driving_history_and_sensor_priority() {
        use super::{enrich_trip, store_estimates, EnrichTripOutcome, TargetSample};
        use crate::services::external_connections as connections;
        let fixture = crate::authorization_test_support::Fixture::new().await;
        let pool = &fixture.state.pool;
        connections::ensure_defaults(pool).await.unwrap();
        let owner = fixture.user("admin").await;
        let vehicle = fixture.vehicle(owner, "synthetic-weather-vehicle").await;
        let start = Utc::now() - chrono::Duration::days(2);
        let trip: Uuid = sqlx::query_scalar(
            "INSERT INTO panda.trips (vehicle_id, started_at, ended_at, start_lat, start_lng,
             distance_miles, energy_wh, efficiency_wh_per_mile, soc_start, soc_end, route_preview)
             VALUES ($1,$2,$2+interval '5 minutes',30.26721,-97.74311,12.5,5000,400,80,75,'[[1,2],[3,4]]') RETURNING id",
        ).bind(vehicle).bind(start).fetch_one(pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.telemetry
            (vehicle_id, ts, trip_id, latitude, longitude, outside_temp_c, odometer_miles, battery_level)
            VALUES ($1,$2,$3,30.26721,-97.74311,18.5,1000,80)")
            .bind(vehicle).bind(start).bind(trip).execute(pool).await.unwrap();
        let trip_snapshot = || {
            sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT to_jsonb(t)-'outside_temp_c'-'outside_temp_source' FROM panda.trips t WHERE id=$1",
        ).bind(trip).fetch_one(pool)
        };
        let raw_snapshot = || {
            sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT jsonb_agg(to_jsonb(t) ORDER BY ts) FROM timeseries.telemetry t WHERE trip_id=$1",
        ).bind(trip).fetch_one(pool)
        };
        let before_trip = trip_snapshot().await.unwrap();
        let before_raw = raw_snapshot().await.unwrap();
        store_estimates(
            pool,
            trip,
            vec![(
                TargetSample {
                    sampled_at: start,
                    elapsed_seconds: 0,
                    cell_key: "30.27,-97.74".into(),
                    provider_lat: 30.27,
                    provider_lng: -97.74,
                },
                12.5,
            )],
            12.5,
            "open_meteo",
        )
        .await
        .unwrap();
        assert_eq!(trip_snapshot().await.unwrap(), before_trip);
        assert_eq!(raw_snapshot().await.unwrap(), before_raw);
        let settings = connections::load(pool, connections::OPEN_METEO)
            .await
            .unwrap();
        let outcome = enrich_trip(pool, &reqwest::Client::new(), "", &settings, trip)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            EnrichTripOutcome::Complete {
                provider_called: false
            }
        ));
        let summary: (f64, String) = sqlx::query_as(
            "SELECT outside_temp_c, outside_temp_source FROM panda.trips WHERE id=$1",
        )
        .bind(trip)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(summary, (18.5, "vehicle".into()));
        assert_eq!(trip_snapshot().await.unwrap(), before_trip);
        assert_eq!(raw_snapshot().await.unwrap(), before_raw);
        sqlx::query(
            "INSERT INTO panda.weather_enrichment_jobs (trip_id,status,attempts,updated_at)
            VALUES ($1,'running',2,now()-interval '20 minutes')",
        )
        .bind(trip)
        .execute(pool)
        .await
        .unwrap();
        crate::private_deployment::weather::recover_interrupted_jobs(pool)
            .await
            .unwrap();
        let recovered: (String, i32) = sqlx::query_as(
            "SELECT status, attempts FROM panda.weather_enrichment_jobs WHERE trip_id=$1",
        )
        .bind(trip)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(recovered, ("pending".into(), 1));
        sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='running',updated_at=now() WHERE trip_id=$1")
            .bind(trip).execute(pool).await.unwrap();
        crate::private_deployment::weather::recover_interrupted_jobs(pool)
            .await
            .unwrap();
        let active: String =
            sqlx::query_scalar("SELECT status FROM panda.weather_enrichment_jobs WHERE trip_id=$1")
                .bind(trip)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(active, "running");
        fixture.cleanup().await;
    }

    #[test]
    fn samples_endpoints_and_fifteen_minute_boundaries_with_rounded_cells() {
        let start = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let trip = TripInfo {
            id: Uuid::new_v4(),
            vehicle_id: Uuid::new_v4(),
            started_at: start,
            ended_at: start + chrono::Duration::minutes(31),
            start_lat: None,
            start_lng: None,
        };
        let route = vec![
            RoutePoint {
                ts: start,
                lat: 30.26721,
                lng: -97.74311,
            },
            RoutePoint {
                ts: start + chrono::Duration::minutes(31),
                lat: 30.301,
                lng: -97.701,
            },
        ];
        let targets = build_targets(&trip, &route, false);
        assert_eq!(targets.len(), 4);
        assert_eq!(targets[0].provider_lat, 30.27);
        assert_eq!(targets.last().unwrap().elapsed_seconds, 31 * 60);
    }

    #[test]
    fn calculates_time_weighted_temperature() {
        let start = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let values = vec![(start, 10.0), (start + chrono::Duration::hours(1), 20.0)];
        assert_eq!(time_weighted_average(&values), Some(15.0));
    }

    #[test]
    fn deduplicates_rounded_cells_but_keeps_the_exact_local_timeline() {
        let start = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let trip = TripInfo {
            id: Uuid::new_v4(),
            vehicle_id: Uuid::new_v4(),
            started_at: start,
            ended_at: start + chrono::Duration::minutes(30),
            start_lat: Some(30.2672),
            start_lng: Some(-97.7431),
        };
        let targets = build_targets(&trip, &[], false);
        assert_eq!(targets.len(), 3);
        assert_eq!(unique_weather_cells(&targets).len(), 1);
        assert_eq!(
            targets
                .iter()
                .map(|target| target.sampled_at)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn exact_mode_preserves_five_decimal_provider_coordinates() {
        let start = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let trip = TripInfo {
            id: Uuid::new_v4(),
            vehicle_id: Uuid::new_v4(),
            started_at: start,
            ended_at: start + chrono::Duration::minutes(1),
            start_lat: Some(30.267214),
            start_lng: Some(-97.743117),
        };
        let targets = build_targets(&trip, &[], true);
        assert_eq!(targets[0].provider_lat, 30.26721);
        assert_eq!(targets[0].provider_lng, -97.74312);
    }

    #[test]
    fn maps_hourly_provider_values_to_the_nearest_local_sample_time() {
        let target = Utc.with_ymd_and_hms(2026, 7, 14, 12, 40, 0).unwrap();
        let payload = serde_json::json!({
            "hourly": {
                "time": ["2026-07-14T12:00", "2026-07-14T13:00"],
                "temperature_2m": [20.0, 24.0]
            }
        });
        assert_eq!(pick_closest_temp(&payload, target), Some(24.0));
    }
}

#[cfg(test)]
mod history_weather_contract_tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap()
    }
    fn trip(seconds: i64) -> TripInfo {
        TripInfo {
            id: Uuid::new_v4(),
            vehicle_id: Uuid::new_v4(),
            started_at: start(),
            ended_at: start() + chrono::Duration::seconds(seconds),
            start_lat: Some(30.26721),
            start_lng: Some(-97.74311),
        }
    }

    #[test]
    fn history_weather_sparse_targets_and_day_boundary() {
        for (seconds, expected) in [
            (0, vec![0]),
            (1, vec![0, 1]),
            (900, vec![0, 900]),
            (901, vec![0, 900, 901]),
        ] {
            assert_eq!(
                build_targets(&trip(seconds), &[], false)
                    .iter()
                    .map(|t| t.elapsed_seconds)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        assert_eq!(build_targets(&trip(86_400), &[], false).len(), 97);
        let mut missing = trip(900);
        missing.start_lng = None;
        assert!(build_targets(&missing, &[], false).is_empty());
        let route = [
            RoutePoint {
                ts: start(),
                lat: 10.,
                lng: 20.,
            },
            RoutePoint {
                ts: start() + chrono::Duration::seconds(1800),
                lat: 11.,
                lng: 21.,
            },
        ];
        let targets = build_targets(&trip(1800), &route, false);
        assert_eq!(
            targets
                .iter()
                .map(|t| (t.provider_lat, t.provider_lng))
                .collect::<Vec<_>>(),
            vec![(10., 20.), (10., 20.), (11., 21.)]
        );
    }

    #[test]
    fn history_weather_raw_nearest_window_ties_and_nonfinite_values() {
        let raw = [
            RawOutsideTemp {
                ts: start(),
                outside_temp_c: -5.,
            },
            RawOutsideTemp {
                ts: start() + chrono::Duration::seconds(900),
                outside_temp_c: 25.,
            },
        ];
        assert_eq!(
            nearest_raw_temp(&raw, start() + chrono::Duration::seconds(450)),
            Some(-5.)
        );
        assert_eq!(
            nearest_raw_temp(&raw[..1], start() + chrono::Duration::seconds(451)),
            None
        );
        assert_eq!(nearest_raw_temp(&[], start()), None);
        let invalid = [RawOutsideTemp {
            ts: start(),
            outside_temp_c: f64::NAN,
        }];
        assert!(nearest_raw_temp(&invalid, start()).unwrap().is_nan());
    }

    #[test]
    fn history_weather_payload_nulls_malformed_times_ties_and_zip_boundary() {
        let payload = json!({"hourly":{"time":["bad","2026-07-14T11:00","2026-07-14T12:00","2026-07-14T13:00","2026-07-14T14:00"],"temperature_2m":[900,10,null,30]}});
        assert_eq!(pick_closest_temp(&payload, start()), Some(10.));
        assert_eq!(
            pick_closest_temp(&payload, start() + chrono::Duration::seconds(1)),
            Some(30.)
        );
        assert_eq!(
            pick_closest_temp(&payload, start() + chrono::Duration::hours(3)),
            None
        );
        assert_eq!(pick_closest_temp(&json!({}), start()), None);
        assert_eq!(
            pick_closest_temp(
                &json!({"hourly":{"time":["2026-07-14T12:00"],"temperature_2m":["12"]}}),
                start()
            ),
            None
        );
    }

    #[test]
    fn history_weather_average_preserves_celsius_and_elapsed_weighting() {
        assert_eq!(time_weighted_average(&[]), None);
        assert_eq!(time_weighted_average(&[(start(), -12.5)]), Some(-12.5));
        assert_eq!(
            time_weighted_average(&[(start(), 10.), (start(), 20.)]),
            Some(10.)
        );
        assert_eq!(
            time_weighted_average(&[
                (start(), 10.),
                (start() + chrono::Duration::seconds(900), 20.),
                (start() + chrono::Duration::seconds(1800), 40.)
            ]),
            Some(22.5)
        );
        assert_eq!(
            time_weighted_average(&[
                (start(), 10.),
                (start() - chrono::Duration::seconds(1), 20.)
            ]),
            Some(10.)
        );
    }

    #[test]
    fn history_weather_coordinate_validation_stays_at_provider_boundary() {
        for (lat, lng) in [(0., 0.), (91., 181.)] {
            let mut input = trip(0);
            input.start_lat = Some(lat);
            input.start_lng = Some(lng);
            let targets = build_targets(&input, &[], false);
            assert_eq!(
                (targets[0].provider_lat, targets[0].provider_lng),
                (lat, lng)
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated disposable DATABASE_URL and REDIS_URL"]
    async fn history_weather_database_join_cache_and_error_contracts() {
        let fixture = crate::authorization_test_support::Fixture::new().await;
        let pool = &fixture.state.pool;
        connections::ensure_defaults(pool).await.unwrap();
        let owner = fixture.user("admin").await;
        let vehicle = fixture.vehicle(owner, "history-weather").await;
        let other_vehicle = fixture.vehicle(owner, "history-weather-other").await;
        let id:Uuid=sqlx::query_scalar("INSERT INTO panda.trips (vehicle_id,started_at,ended_at,start_lat,start_lng,distance_miles,energy_wh) VALUES ($1,$2,$2+interval '15 minutes',30.26721,-97.74311,12.5,5000) RETURNING id").bind(vehicle).bind(start()).fetch_one(pool).await.unwrap();
        let other:Uuid=sqlx::query_scalar("INSERT INTO panda.trips (vehicle_id,started_at,ended_at) VALUES ($1,$2,$2) RETURNING id").bind(vehicle).bind(start()).fetch_one(pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.telemetry (vehicle_id,ts,trip_id,latitude,longitude,outside_temp_c) VALUES ($1,$2,$3,10,20,18),($1,$2+interval '15 minutes',$3,11,21,22),($4,$2,$3,40,50,99)").bind(vehicle).bind(start()).bind(other).bind(other_vehicle).execute(pool).await.unwrap();
        let settings = connections::load(pool, connections::OPEN_METEO)
            .await
            .unwrap();
        let outcome = enrich_trip(pool, &reqwest::Client::new(), "", &settings, id)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            EnrichTripOutcome::Complete {
                provider_called: false
            }
        ));
        let result:(f64,String,i64)=sqlx::query_as("SELECT outside_temp_c,outside_temp_source,(SELECT count(*) FROM panda.trip_weather_samples WHERE trip_id=$1) FROM panda.trips WHERE id=$1").bind(id).fetch_one(pool).await.unwrap();
        assert_eq!(result, (20., "vehicle".into(), 0));
        let missing:Uuid=sqlx::query_scalar("INSERT INTO panda.trips (vehicle_id,started_at,ended_at) VALUES ($1,$2,$2) RETURNING id").bind(other_vehicle).bind(start()+chrono::Duration::days(2)).fetch_one(pool).await.unwrap();
        let unavailable = enrich_trip(pool, &reqwest::Client::new(), "", &settings, missing)
            .await
            .unwrap();
        assert!(matches!(unavailable, EnrichTripOutcome::Unavailable));
        sqlx::query("UPDATE panda.trips SET ended_at=started_at+interval '25 hours' WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            enrich_trip(pool, &reqwest::Client::new(), "", &settings, id)
                .await
                .err()
                .unwrap()
                .to_string(),
            "trip exceeds weather sample bounds"
        );
        assert_eq!(
            enrich_trip(pool, &reqwest::Client::new(), "", &settings, Uuid::new_v4())
                .await
                .err()
                .unwrap()
                .to_string(),
            "trip not found"
        );
        enqueue(pool, id).await.unwrap();
        sqlx::query("UPDATE panda.weather_enrichment_jobs SET status='succeeded',attempts=3 WHERE trip_id=$1").bind(id).execute(pool).await.unwrap();
        enqueue(pool, id).await.unwrap();
        let job: (String, i32) = sqlx::query_as(
            "SELECT status,attempts FROM panda.weather_enrichment_jobs WHERE trip_id=$1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(job, ("succeeded".into(), 3));
        assert!(!process_next_job(pool, &reqwest::Client::new(), "")
            .await
            .unwrap());
        let targets = build_targets(&trip(900), &[], false);
        store_estimates(pool, id, vec![(targets[0].clone(), 5.)], 5., "open_meteo")
            .await
            .unwrap();
        let duplicate = vec![(targets[0].clone(), 6.), (targets[0].clone(), 7.)];
        assert!(store_estimates(pool, id, duplicate, 6., "mixed")
            .await
            .is_err());
        let stored:(f64,String,i64)=sqlx::query_as("SELECT outside_temp_c,outside_temp_source,(SELECT count(*) FROM panda.trip_weather_samples WHERE trip_id=$1) FROM panda.trips WHERE id=$1").bind(id).fetch_one(pool).await.unwrap();
        assert_eq!(stored, (5., "open_meteo".into(), 1));
        fixture.cleanup().await;
    }
}

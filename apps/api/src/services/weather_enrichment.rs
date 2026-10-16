use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
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

    let plan = HistoryWeatherPlan::load(pool, &trip).await?;
    if plan.targets.is_empty() {
        return Ok(EnrichTripOutcome::Unavailable);
    }
    let weather_targets = plan.weather_targets();
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

    let estimate = plan.finish(&hourly_by_cell)?;
    store_estimates(
        pool,
        trip.id,
        estimate.samples,
        estimate.summary,
        estimate.source,
    )
    .await?;
    Ok(EnrichTripOutcome::Complete { provider_called })
}

struct HistoryWeatherPlan {
    targets: Vec<TargetSample>,
    vehicle_values: Vec<Option<f64>>,
    has_raw: bool,
}

struct HistoryWeatherEstimate {
    samples: Vec<(TargetSample, f64)>,
    summary: f64,
    source: &'static str,
}

impl HistoryWeatherPlan {
    async fn load(pool: &PgPool, trip: &TripInfo) -> Result<Self> {
        if trip.ended_at < trip.started_at
            || trip.ended_at - trip.started_at > chrono::Duration::days(1)
        {
            anyhow::bail!("trip exceeds weather sample bounds");
        }
        let times = history_weather_times(trip);
        let mut positions = vec![None; times.len()];
        let mut route = sqlx::query_as::<_, RoutePoint>(
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
        .fetch(pool);
        while let Some(point) = route.try_next().await? {
            for (at, position) in times.iter().zip(&mut positions) {
                history_keep_nearest(
                    position,
                    (point.ts - *at).num_seconds().abs(),
                    (point.lat, point.lng),
                );
            }
        }
        drop(route);
        let targets = times
            .into_iter()
            .zip(positions)
            .filter_map(|(at, position)| {
                history_weather_target(trip, at, position.map(|(_, pair)| pair), false)
            })
            .collect::<Vec<_>>();
        let mut temperatures = vec![None; targets.len()];
        let mut has_raw = false;
        if !targets.is_empty() {
            let mut raw = sqlx::query_as::<_, RawOutsideTemp>(
                r#"SELECT ts, outside_temp_c FROM timeseries.telemetry
                   WHERE vehicle_id=$1 AND ts >= $2 AND ts <= $3 AND outside_temp_c IS NOT NULL
                   ORDER BY ts"#,
            )
            .bind(trip.vehicle_id)
            .bind(trip.started_at)
            .bind(trip.ended_at)
            .fetch(pool);
            while let Some(row) = raw.try_next().await? {
                has_raw = true;
                for (target, temperature) in targets.iter().zip(&mut temperatures) {
                    let distance = (row.ts - target.sampled_at).num_seconds().abs();
                    if distance <= SAMPLE_INTERVAL_SECONDS / 2 {
                        history_keep_nearest(temperature, distance, row.outside_temp_c);
                    }
                }
            }
        }
        Ok(Self {
            targets,
            vehicle_values: temperatures
                .into_iter()
                .map(|v| v.map(|(_, temp)| temp))
                .collect(),
            has_raw,
        })
    }

    fn weather_targets(&self) -> Vec<TargetSample> {
        self.targets
            .iter()
            .zip(&self.vehicle_values)
            .filter(|(_, value)| value.is_none())
            .map(|(target, _)| target.clone())
            .collect()
    }

    fn finish(self, hourly: &HashMap<String, Value>) -> Result<HistoryWeatherEstimate> {
        let mut samples = Vec::new();
        let mut summary_values = Vec::with_capacity(self.targets.len());
        let mut vehicle_count = 0;
        for (target, measured) in self.targets.into_iter().zip(self.vehicle_values) {
            if let Some(value) = measured {
                vehicle_count += 1;
                summary_values.push((target.sampled_at, value));
            } else if let Some(value) = hourly
                .get(&target.cell_key)
                .and_then(|body| pick_closest_temp(body, target.sampled_at))
            {
                summary_values.push((target.sampled_at, value));
                samples.push((target, value));
            }
        }
        if samples.is_empty() && !self.has_raw {
            anyhow::bail!("weather provider returned no usable temperatures");
        }
        let source = match (vehicle_count > 0, !samples.is_empty()) {
            (true, false) => "vehicle",
            (false, true) => "open_meteo",
            _ => "mixed",
        };
        let summary =
            time_weighted_average(&summary_values).context("weather summary unavailable")?;
        Ok(HistoryWeatherEstimate {
            samples,
            summary,
            source,
        })
    }
}

fn history_keep_nearest<T>(slot: &mut Option<(i64, T)>, distance: i64, value: T) {
    if slot.as_ref().is_none_or(|(best, _)| distance < *best) {
        *slot = Some((distance, value));
    }
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

#[cfg(test)]
fn nearest_raw_temp(raw: &[RawOutsideTemp], sampled_at: DateTime<Utc>) -> Option<f64> {
    raw.iter()
        .min_by_key(|row| (row.ts - sampled_at).num_seconds().abs())
        .filter(|row| (row.ts - sampled_at).num_seconds().abs() <= SAMPLE_INTERVAL_SECONDS / 2)
        .map(|row| row.outside_temp_c)
}

#[cfg(test)]
fn build_targets(trip: &TripInfo, route: &[RoutePoint], exact: bool) -> Vec<TargetSample> {
    history_weather_times(trip)
        .into_iter()
        .filter_map(|at| {
            let mut position = None;
            for point in route {
                history_keep_nearest(
                    &mut position,
                    (point.ts - at).num_seconds().abs(),
                    (point.lat, point.lng),
                );
            }
            history_weather_target(trip, at, position.map(|(_, pair)| pair), exact)
        })
        .collect()
}

fn history_weather_times(trip: &TripInfo) -> Vec<DateTime<Utc>> {
    let duration = (trip.ended_at - trip.started_at).num_seconds().max(0);
    let mut times = (0..=duration / SAMPLE_INTERVAL_SECONDS)
        .map(|index| trip.started_at + chrono::Duration::seconds(index * SAMPLE_INTERVAL_SECONDS))
        .filter(|at| *at < trip.ended_at)
        .collect::<Vec<_>>();
    times.push(trip.ended_at);
    times
}

fn history_weather_target(
    trip: &TripInfo,
    sampled_at: DateTime<Utc>,
    position: Option<(f64, f64)>,
    exact: bool,
) -> Option<TargetSample> {
    let (lat, lng) = position.or_else(|| trip.start_lat.zip(trip.start_lng))?;
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

#[cfg(test)]
mod history_weather_plan_tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn synthetic_provider_payloads_fill_only_unmeasured_targets() {
        let start = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let trip = TripInfo {
            id: Uuid::new_v4(),
            vehicle_id: Uuid::new_v4(),
            started_at: start,
            ended_at: start + chrono::Duration::minutes(30),
            start_lat: Some(30.26721),
            start_lng: Some(-97.74311),
        };
        let targets = build_targets(&trip, &[], false);
        let payloads = HashMap::from([(
            targets[0].cell_key.clone(),
            json!({"hourly": {
                "time": ["2026-07-14T12:00"], "temperature_2m": [20.0]
            }}),
        )]);
        for (values, source, count) in [
            (vec![Some(10.), None, Some(30.)], "mixed", 1),
            (vec![None, None, None], "open_meteo", 3),
            (vec![Some(10.), Some(20.), Some(30.)], "vehicle", 0),
        ] {
            let plan = HistoryWeatherPlan {
                targets: targets.clone(),
                has_raw: values.iter().any(Option::is_some),
                vehicle_values: values,
            };
            assert_eq!(plan.weather_targets().len(), count);
            let estimate = plan.finish(&payloads).unwrap();
            assert_eq!(
                (estimate.summary, estimate.source, estimate.samples.len()),
                (20., source, count)
            );
            assert!(estimate.samples.iter().all(|(target, temp)| *temp == 20.
                && target.provider_lat == 30.27
                && target.provider_lng == -97.74));
        }
        for (has_raw, message) in [
            (false, "weather provider returned no usable temperatures"),
            (true, "weather summary unavailable"),
        ] {
            let plan = HistoryWeatherPlan {
                targets: targets.clone(),
                vehicle_values: vec![None; 3],
                has_raw,
            };
            assert_eq!(
                plan.finish(&HashMap::new()).err().unwrap().to_string(),
                message
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires isolated disposable DATABASE_URL and REDIS_URL"]
    async fn streamed_sparse_plan_preserves_vehicle_and_trip_joins() {
        let mut fixture = crate::authorization_test_support::Fixture::new().await;
        let options = fixture.state.pool.connect_options().as_ref().clone();
        fixture.state.pool.close().await;
        fixture.state.pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let pool = &fixture.state.pool;
        let owner = fixture.user("admin").await;
        let vehicle = fixture.vehicle(owner, "streamed-history-weather").await;
        let other_vehicle = fixture.vehicle(owner, "streamed-history-other").await;
        let start = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let id: Uuid = sqlx::query_scalar("INSERT INTO panda.trips (vehicle_id,started_at,ended_at,start_lat,start_lng) VALUES ($1,$2,$2+interval '31 minutes',40,-100) RETURNING id")
            .bind(vehicle).bind(start).fetch_one(pool).await.unwrap();
        let other: Uuid = sqlx::query_scalar("INSERT INTO panda.trips (vehicle_id,started_at,ended_at) VALUES ($1,$2,$2) RETURNING id")
            .bind(vehicle).bind(start).fetch_one(pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.telemetry (vehicle_id,ts,trip_id,latitude,longitude,outside_temp_c) VALUES
            ($1,$3,$5,55,66,NULL),
            ($1,$3+interval '10 minutes',$4,30,-97,10),
            ($1,$3+interval '15 minutes',NULL,0,-96,NULL),
            ($1,$3+interval '20 minutes',NULL,31,-98,20),
            ($1,$3+interval '31 minutes',$5,50,-90,30),
            ($2,$3,NULL,60,70,99)")
            .bind(vehicle).bind(other_vehicle).bind(start).bind(id).bind(other).execute(pool).await.unwrap();
        let trip = TripInfo {
            id,
            vehicle_id: vehicle,
            started_at: start,
            ended_at: start + chrono::Duration::minutes(31),
            start_lat: Some(40.),
            start_lng: Some(-100.),
        };
        let plan = HistoryWeatherPlan::load(pool, &trip).await.unwrap();
        assert_eq!(
            plan.targets
                .iter()
                .map(|target| (
                    target.elapsed_seconds,
                    target.provider_lat,
                    target.provider_lng
                ))
                .collect::<Vec<_>>(),
            vec![
                (0, 30., -97.),
                (900, 30., -97.),
                (1800, 31., -98.),
                (1860, 31., -98.)
            ]
        );
        assert_eq!(
            plan.vehicle_values,
            vec![None, Some(10.), Some(30.), Some(30.)]
        );
        let payloads = HashMap::from([(
            plan.targets[0].cell_key.clone(),
            json!({"hourly": {
                "time": ["2026-07-14T12:00"], "temperature_2m": [5.0]
            }}),
        )]);
        let estimate = plan.finish(&payloads).unwrap();
        assert_eq!(
            (estimate.summary, estimate.source, estimate.samples.len()),
            (26550. / 1860., "mixed", 1)
        );
        store_estimates(
            pool,
            id,
            estimate.samples,
            estimate.summary,
            estimate.source,
        )
        .await
        .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM timeseries.telemetry")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(count, 6);
        fixture.cleanup().await;
    }

    #[tokio::test]
    async fn weather_chunk_size_errors_never_need_a_provider_or_database() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://synthetic:synthetic@localhost/unused")
            .unwrap();
        let client = reqwest::Client::new();
        let now = Utc::now();
        let cell = WeatherCell {
            key: "30.00,-97.00".into(),
            lat: 30.,
            lng: -97.,
        };
        for cells in [vec![], vec![cell.clone(), cell]] {
            assert_eq!(
                fetch_weather_chunk(&pool, &client, &cells, now, now)
                    .await
                    .unwrap_err()
                    .to_string(),
                "Weather requests require one location"
            );
        }
        pool.close().await;
    }
}

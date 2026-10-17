mod history;
mod map_projection;
mod power_series;
mod sample_projection;
mod telemetry;
mod temperature_series;
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use crate::{
    db::vehicles::{require_vehicle_membership, require_vehicle_read_access},
    errors::AppError,
    middleware::auth::{require_vehicle_access, AppState, AuthUser},
    routes::trip_tag_filter::{
        parse_tag_filter as parse_shared_tag_filter, require_known_vehicle_tags, sql_predicate,
        TripTagFilter, TripTagMatch,
    },
    services::trip_routes::build_route_preview,
};

pub fn router() -> Router<AppState> {
    super::route_table([
        ("/trips", get(list_trips)),
        (
            "/trips/tire-pressure-timeline",
            get(get_tire_pressure_timeline),
        ),
        ("/trips/{id}", get(get_trip)),
        ("/trips/{id}/detail", get(get_trip_detail)),
        ("/trips/{id}/track", get(get_track)),
        ("/trips/{id}/speed", get(get_speed_profile)),
        ("/trips/{id}/elevation", get(get_elevation_profile)),
        ("/trips/{id}/power", get(get_power_profile)),
        ("/trips/{id}/series", get(get_trip_series)),
        ("/trips/map", get(get_trip_map)),
        (
            "/vehicles/{vehicle_id}/drives/{id}/power",
            get(get_power_profile_path),
        ),
    ])
}

#[derive(Deserialize)]
struct TripListParams {
    vehicle_id: Option<Uuid>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
    page: Option<i64>,
    per_page: Option<i64>,
    search: Option<String>,
    /// Canonical comma-separated UUID list. Repeated query keys are not
    /// portable across all clients/proxies, so the URL form stays stable.
    tag_ids: Option<String>,
    tag_match: Option<TripTagMatch>,
    untagged: Option<bool>,
}

fn parse_tag_filter(params: &TripListParams) -> Result<TripTagFilter, AppError> {
    parse_shared_tag_filter(params.tag_ids.as_deref(), params.tag_match, params.untagged)
}

fn trip_search_predicate(param: usize) -> String {
    let columns = [
        "sg.name",
        "eg.name",
        "sa.display_name",
        "ea.display_name",
        "CONCAT_WS(', ', sa.road, sa.city)",
        "CONCAT_WS(', ', ea.road, ea.city)",
    ];
    let mut alternatives = vec![format!("${param}::text IS NULL")];
    alternatives.extend(
        columns.into_iter().map(|column| {
            format!("COALESCE({column}, '') ILIKE '%' || ${param} || '%' ESCAPE '\\'")
        }),
    );
    alternatives.push(format!(
        "EXISTS (SELECT 1 FROM panda.trip_tag_assignments tta JOIN panda.trip_tags tt ON tt.id=tta.tag_id WHERE tta.trip_id=t.id AND tt.name ILIKE '%' || ${param} || '%' ESCAPE '\\')"));
    format!(" AND ({})", alternatives.join(" OR "))
}

#[cfg(test)]
mod timeframe_tests {
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    #[test]
    fn lifetime_time_bounds_use_epoch_instead_of_default_window() {
        let to = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
        let (from, resolved_to) = super::resolve_time_bounds(None, Some(to), true, 90);

        assert_eq!(resolved_to, to);
        assert_eq!(from, chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap());
    }

    #[test]
    fn tag_filter_defaults_to_all_and_rejects_untagged_combination() {
        let tag = Uuid::new_v4();
        let params = super::TripListParams {
            vehicle_id: None,
            from: None,
            to: None,
            lifetime: None,
            limit: None,
            offset: None,
            page: None,
            per_page: None,
            search: None,
            tag_ids: Some(tag.to_string()),
            tag_match: None,
            untagged: None,
        };
        let filter = super::parse_tag_filter(&params).unwrap();
        assert!(filter.match_all);
        assert_eq!(filter.tag_ids, Some(vec![tag]));

        let invalid = super::TripListParams {
            untagged: Some(true),
            ..params
        };
        assert!(super::parse_tag_filter(&invalid).is_err());
    }

    #[test]
    fn trip_search_predicate_is_balanced() {
        let predicate = super::trip_search_predicate(4);

        assert!(predicate.contains("$4::text IS NULL"));
        assert!(predicate.contains("EXISTS"));
        assert!(predicate.ends_with("))"));

        let mut depth = 0;
        for character in predicate.chars() {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "predicate closed before it opened");
        }
        assert_eq!(depth, 0);
    }
}

fn resolve_time_bounds(
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: bool,
    default_days: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    super::time_bounds(from, to, lifetime, default_days)
}

#[derive(Deserialize)]
struct VehicleParam {
    vehicle_id: Option<Uuid>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct TripRow {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_seconds: Option<i32>,
    distance_miles: Option<f64>,
    efficiency_wh_per_mile: Option<f64>,
    max_speed_mph: Option<f64>,
    drive_mode: Option<String>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
    end_lat: Option<f64>,
    end_lng: Option<f64>,
    start_place: Option<String>,
    end_place: Option<String>,
    start_address: Option<String>,
    end_address: Option<String>,
    outside_temp_c: Option<f64>,
    outside_temp_source: Option<String>,
    tags: serde_json::Value,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct TripMapRow {
    id: Uuid,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    route_preview: Option<serde_json::Value>,
    tags: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize, sqlx::FromRow)]
struct TrackPoint {
    ts: DateTime<Utc>,
    lat: Option<f64>,
    lng: Option<f64>,
    speed_mph: Option<f64>,
    altitude_m: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct TripWindowRow {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_seconds: Option<i32>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct SpeedProfileRow {
    elapsed_s: f64,
    speed_mph: Option<f64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct ElevationPointRow {
    ts: DateTime<Utc>,
    value: Option<f64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct PowerProfileRow {
    ts: DateTime<Utc>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
    speed_mph: Option<f64>,
    battery_level: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct TripPowerTelemetryRow {
    ts: DateTime<Utc>,
    battery_level: Option<f64>,
    battery_capacity_wh: Option<f64>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TripPowerSource {
    Direct,
    EstimatedSoc,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TripPowerMetadata {
    source: TripPowerSource,
    sample_count: usize,
    median_interval_seconds: Option<f64>,
    p90_interval_seconds: Option<f64>,
    coverage_percent: Option<f64>,
}

#[derive(Debug, Clone)]
struct TripPowerInterval {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    net_power_kw: f64,
}

#[derive(Debug, Clone)]
struct TripPowerData {
    metadata: TripPowerMetadata,
    intervals: Vec<TripPowerInterval>,
}

impl TripPowerData {
    fn estimated_at(&self, ts: DateTime<Utc>) -> Option<f64> {
        if !matches!(self.metadata.source, TripPowerSource::EstimatedSoc) {
            return None;
        }
        self.intervals
            .iter()
            .find(|interval| ts >= interval.started_at && ts < interval.ended_at)
            .map(|interval| interval.net_power_kw)
    }

    fn source(&self) -> TripPowerSource {
        self.metadata.source
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct TripSeriesRow {
    ts: DateTime<Utc>,
    speed_mph: Option<f64>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
    battery_level: Option<f64>,
    outside_temp_c: Option<f64>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    hvac_active: Option<bool>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct TripDetailSamples {
    elapsed_s: Vec<i32>,
    lat: Vec<Option<f64>>,
    lng: Vec<Option<f64>>,
    altitude_m: Vec<Option<f64>>,
    speed_mph: Vec<Option<f64>>,
    power_kw: Vec<Option<f64>>,
    regen_power_kw: Vec<Option<f64>>,
    estimated_net_power_kw: Vec<Option<f64>>,
    battery_level: Vec<Option<f64>>,
    outside_temp_c: Vec<Option<f64>>,
    cabin_temp_c: Vec<Option<f64>>,
    driver_temp_c: Vec<Option<f64>>,
    hvac_active: Vec<Option<bool>>,
    tire_fl_psi: Vec<Option<f64>>,
    tire_fr_psi: Vec<Option<f64>>,
    tire_rl_psi: Vec<Option<f64>>,
    tire_rr_psi: Vec<Option<f64>>,
}

#[derive(Debug, Serialize)]
struct TripDetailResponse {
    trip: TripRow,
    sample_interval_seconds: i32,
    samples: TripDetailSamples,
    power: TripPowerMetadata,
    outside_temperature: OutsideTemperatureResponse,
}

#[derive(Debug, Serialize, Deserialize)]
struct TripDetailCachePayload {
    samples: TripDetailSamples,
    power: TripPowerMetadata,
}

#[derive(Debug, Serialize)]
struct OutsideTemperatureResponse {
    source: String,
    attribution: Option<OutsideTemperatureAttribution>,
    samples: Vec<OutsideTemperatureSample>,
}

#[derive(Debug, Serialize)]
struct OutsideTemperatureAttribution {
    name: &'static str,
    url: &'static str,
}

#[derive(Debug, Serialize)]
struct OutsideTemperatureSample {
    elapsed_s: i32,
    ts: DateTime<Utc>,
    temperature_c: f64,
    source: &'static str,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TripWeatherRow {
    sampled_at: DateTime<Utc>,
    elapsed_seconds: i32,
    temperature_c: f64,
}

#[derive(Debug, Serialize)]
struct TripMapRoute {
    trip_id: Uuid,
    coordinates: Vec<[f64; 2]>,
    tags: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct TripMapResponse {
    vehicle_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    total_trips: usize,
    missing_route_count: usize,
    routes: Vec<TripMapRoute>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct TirePressureTimelineSample {
    ts: DateTime<Utc>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct TirePressureTimelineTrip {
    id: Uuid,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_seconds: Option<i32>,
    duration_min: Option<f64>,
    distance_miles: Option<f64>,
    start_place: Option<String>,
    end_place: Option<String>,
    start_address: Option<String>,
    end_address: Option<String>,
    tags: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct TirePressureTimelineResponse {
    vehicle_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    samples: Vec<TirePressureTimelineSample>,
    trips: Vec<TirePressureTimelineTrip>,
}

#[derive(Debug, sqlx::FromRow)]
struct DetailSampleRow {
    bucket_ts: DateTime<Utc>,
    elapsed_s: i32,
    lat: Option<f64>,
    lng: Option<f64>,
    altitude_m: Option<f64>,
    speed_mph: Option<f64>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
    battery_level: Option<f64>,
    outside_temp_c: Option<f64>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    hvac_active: Option<bool>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct RouteTelemetryRow {
    trip_id: Uuid,
    lat: f64,
    lng: f64,
}

fn finite_value(value: Option<f64>) -> Option<f64> {
    value.filter(|candidate| candidate.is_finite())
}

fn normalize_capacity_wh(value: Option<f64>) -> Option<f64> {
    let value = value?;
    [(40.0..=250.0, 1000.0), (40_000.0..=250_000.0, 1.0)]
        .into_iter()
        .find_map(|(bounds, scale)| bounds.contains(&value).then_some(value * scale))
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    let last = values.len().checked_sub(1)?;
    let position = percentile.clamp(0.0, 1.0) * last as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let mut work = values.to_vec();
    let (_, low, greater) = work.select_nth_unstable_by(lower, f64::total_cmp);
    if lower == upper {
        return Some(*low);
    }
    let high = greater.iter().min_by(|a, b| a.total_cmp(b))?;
    Some(*low + (*high - *low) * (position - lower as f64))
}

fn derive_soc_power(rows: &[TripPowerTelemetryRow], trip_duration_seconds: i32) -> TripPowerData {
    power_series::derive(rows, trip_duration_seconds)
}

async fn load_trip_power_data(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    trip_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_seconds: i32,
) -> Result<TripPowerData, AppError> {
    let window = telemetry::Window::new(vehicle_id, trip_id, started_at, ended_at);
    let rows = window
        .fetch(pool, telemetry::Projection::PowerEvidence)
        .await?;

    Ok(derive_soc_power(&rows, duration_seconds))
}

async fn list_trips(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<TripListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    let tag_filter = parse_tag_filter(&p)?;
    require_known_vehicle_tags(&state.pool, vid, &tag_filter).await?;
    let plan = history::HistoryPlan::new(vid, auth.user_id, &p, tag_filter);
    Ok(Json(plan.page(&state.pool, history::Page::new(&p)).await?))
}

async fn get_tire_pressure_timeline(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<TripListParams>,
) -> Result<Json<TirePressureTimelineResponse>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    let tag_filter = parse_tag_filter(&p)?;
    require_known_vehicle_tags(&state.pool, vid, &tag_filter).await?;
    let plan = history::HistoryPlan::new(vid, auth.user_id, &p, tag_filter);
    Ok(Json(plan.timeline(&state.pool).await?))
}

async fn get_trip(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<TripRow>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;

    let row = history::trip(&state.pool, vid, id, auth.user_id).await?;

    Ok(Json(row))
}

async fn get_trip_map(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<TripListParams>,
) -> Result<Json<TripMapResponse>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;
    let tag_filter = parse_tag_filter(&p)?;
    require_known_vehicle_tags(&state.pool, vid, &tag_filter).await?;
    let plan = history::HistoryPlan::new(vid, auth.user_id, &p, tag_filter);
    Ok(Json(plan.map(&state.pool).await?))
}

async fn get_trip_detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<TripDetailResponse>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;

    let trip = history::trip(&state.pool, vid, id, auth.user_id).await?;

    let duration_seconds = trip
        .duration_seconds
        .unwrap_or_else(|| (trip.ended_at - trip.started_at).num_seconds().max(0) as i32);
    let sample_interval_seconds = detail_bucket_seconds(duration_seconds);
    let weather_rows = sqlx::query_as::<_, TripWeatherRow>(
        "SELECT sampled_at, elapsed_seconds, temperature_c FROM panda.trip_weather_samples WHERE trip_id=$1 ORDER BY sampled_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let cache_key = format!("trips:detail:v3:{vid}:{id}:{sample_interval_seconds}");
    let mut redis_conn = state.redis.get_multiplexed_async_connection().await.ok();
    if let Some(conn) = redis_conn.as_mut() {
        let cached: Option<String> = conn.get(&cache_key).await.ok();
        if let Some(payload) = cached {
            if let Ok(cached) = serde_json::from_str::<TripDetailCachePayload>(&payload) {
                let (payload, outside_temperature) =
                    sample_projection::DetailEvidence::cached(cached).project(&trip, &weather_rows);
                debug!(trip_id = %id, vehicle_id = %vid, cache_hit = true, "trips.detail_cache");
                return Ok(Json(TripDetailResponse {
                    trip,
                    sample_interval_seconds,
                    samples: payload.samples,
                    power: payload.power,
                    outside_temperature,
                }));
            }
        }
    }

    let window = telemetry::Window::new(vid, id, trip.started_at, trip.ended_at);
    let rows = window
        .fetch(
            &state.pool,
            telemetry::Projection::Detail(sample_interval_seconds),
        )
        .await?;

    let power_data = load_trip_power_data(
        &state.pool,
        vid,
        id,
        trip.started_at,
        trip.ended_at,
        duration_seconds,
    )
    .await?;
    let (payload, outside_temperature) =
        sample_projection::DetailEvidence::sampled(rows, power_data).project(&trip, &weather_rows);

    if let Some(conn) = redis_conn.as_mut() {
        if let Ok(payload) = serde_json::to_string(&payload) {
            let _: Result<(), redis::RedisError> =
                conn.set_ex(&cache_key, payload, 24 * 60 * 60).await;
        }
    }

    debug!(
        trip_id = %id,
        vehicle_id = %vid,
        cache_hit = false,
        sample_interval_seconds,
        point_count = payload.samples.elapsed_s.len(),
        "trips.detail_data"
    );

    Ok(Json(TripDetailResponse {
        trip,
        sample_interval_seconds,
        samples: payload.samples,
        power: payload.power,
        outside_temperature,
    }))
}

fn raw_outside_elapsed(samples: &TripDetailSamples) -> std::collections::HashSet<i32> {
    samples
        .elapsed_s
        .iter()
        .copied()
        .zip(samples.outside_temp_c.iter())
        .filter_map(|(elapsed, value)| value.map(|_| elapsed))
        .collect()
}

fn merge_weather_samples(samples: &mut TripDetailSamples, weather_rows: &[TripWeatherRow]) {
    let mut timeline = samples
        .elapsed_s
        .iter()
        .copied()
        .enumerate()
        .map(|(index, elapsed)| (elapsed, index))
        .collect::<Vec<_>>();
    timeline.sort_unstable();
    timeline.dedup_by_key(|point| point.0);
    let Some(&(first, _)) = timeline.first() else {
        return;
    };
    let last = timeline.last().unwrap().0;
    for weather in weather_rows {
        let elapsed = weather.elapsed_seconds;
        let safe_distance = |point: i32| point.checked_sub(elapsed).and_then(i32::checked_abs);
        let index = if safe_distance(first).is_some() && safe_distance(last).is_some() {
            let split = timeline.partition_point(|point| point.0 < elapsed);
            [
                split.checked_sub(1),
                (split < timeline.len()).then_some(split),
            ]
            .into_iter()
            .flatten()
            .map(|position| {
                let (point, index) = timeline[position];
                ((point - elapsed).abs(), index)
            })
            .min()
            .map(|(_, index)| index)
        } else {
            // Preserve the legacy overflow behavior for nonphysical elapsed values.
            samples
                .elapsed_s
                .iter()
                .enumerate()
                .min_by_key(|(_, point)| (**point - elapsed).abs())
                .map(|(index, _)| index)
        };
        if let Some(value) = index.and_then(|index| samples.outside_temp_c.get_mut(index)) {
            value.get_or_insert(weather.temperature_c);
        }
    }
}

fn build_outside_temperature(
    trip: &TripRow,
    samples: &TripDetailSamples,
    weather_rows: &[TripWeatherRow],
    raw_elapsed: &std::collections::HashSet<i32>,
) -> OutsideTemperatureResponse {
    temperature_series::build(trip, samples, weather_rows, raw_elapsed)
}

fn detail_bucket_seconds(duration_seconds: i32) -> i32 {
    let target = ((i64::from(duration_seconds).max(1) + 1499) / 1500) as i32;
    let mut magnitude = 1;
    loop {
        for factor in [1, 2, 5] {
            let seconds = magnitude * factor;
            if seconds >= target {
                return seconds;
            }
        }
        magnitude *= 10;
    }
}

async fn get_track(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<Vec<TrackPoint>>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;

    let trip = telemetry::Window::load(&state.pool, vid, id, true).await?;

    let (bucket, projection) = trip.track();

    let cache_key = format!("trips:track:v2:{vid}:{id}:{bucket}");
    let mut redis_conn = state.redis.get_multiplexed_async_connection().await.ok();

    if let Some(conn) = redis_conn.as_mut() {
        let cached: Option<String> = conn.get(&cache_key).await.ok();
        if let Some(payload) = cached {
            if let Ok(points) = serde_json::from_str::<Vec<TrackPoint>>(&payload) {
                debug!(trip_id = %id, vehicle_id = %vid, cache_hit = true, "trips.track_cache");
                return Ok(Json(points));
            }
        }
    }

    let points: Vec<TrackPoint> = trip.fetch(&state.pool, projection).await?;

    if let Some(conn) = redis_conn.as_mut() {
        if let Ok(payload) = serde_json::to_string(&points) {
            let _: Result<(), redis::RedisError> =
                conn.set_ex(&cache_key, payload, 12 * 60 * 60).await;
        }
    }

    debug!(trip_id = %id, vehicle_id = %vid, cache_hit = false, point_count = points.len(), "trips.track_cache");

    Ok(Json(points))
}

async fn get_speed_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;

    let trip = telemetry::Window::load(&state.pool, vid, id, false).await?;

    let points: Vec<SpeedProfileRow> = trip
        .fetch(&state.pool, telemetry::Projection::Speed)
        .await?;

    Ok(Json(serde_json::json!(points)))
}

async fn get_elevation_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;
    require_vehicle_access(&auth, vid)?;
    require_vehicle_read_access(&state.pool, &auth, vid).await?;

    let trip = telemetry::Window::load(&state.pool, vid, id, false).await?;

    let points: Vec<ElevationPointRow> = trip
        .fetch(&state.pool, telemetry::Projection::Elevation)
        .await?;

    Ok(Json(serde_json::json!(points)))
}

async fn get_power_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<serde_json::Value>, AppError> {
    TripRead::Power(id)
        .respond(&state, &auth, p.vehicle_id)
        .await
}

async fn get_power_profile_path(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    TripRead::Power(id)
        .respond(&state, &auth, Some(vehicle_id))
        .await
}

async fn get_trip_series(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<serde_json::Value>, AppError> {
    TripRead::Series(id)
        .respond(&state, &auth, p.vehicle_id)
        .await
}

async fn power_profile_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    trip_id: Uuid,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;

    let trip = telemetry::Window::load(&state.pool, vehicle_id, trip_id, false).await?;

    let points: Vec<PowerProfileRow> = trip
        .fetch(&state.pool, telemetry::Projection::Power)
        .await?;

    let power_data = load_trip_power_data(
        &state.pool,
        vehicle_id,
        trip_id,
        trip.started_at,
        trip.ended_at,
        (trip.ended_at - trip.started_at).num_seconds().max(1) as i32,
    )
    .await?;
    let points = sample_projection::powered(points, |point| point.ts, &power_data);

    Ok(Json(serde_json::json!(points)))
}

async fn trip_series_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    trip_id: Uuid,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;

    let trip = telemetry::Window::load(&state.pool, vehicle_id, trip_id, false).await?;

    let points: Vec<TripSeriesRow> = trip
        .fetch(&state.pool, telemetry::Projection::Series)
        .await?;

    let power_data = load_trip_power_data(
        &state.pool,
        vehicle_id,
        trip_id,
        trip.started_at,
        trip.ended_at,
        (trip.ended_at - trip.started_at).num_seconds().max(1) as i32,
    )
    .await?;
    let points = sample_projection::powered(points, |point| point.ts, &power_data);

    Ok(Json(serde_json::json!(points)))
}

#[cfg(test)]
mod trip_power_tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn row(offset_seconds: i64, soc: f64) -> TripPowerTelemetryRow {
        TripPowerTelemetryRow {
            ts: Utc.timestamp_opt(offset_seconds, 0).single().unwrap(),
            battery_level: Some(soc),
            battery_capacity_wh: Some(111_500.0),
            power_kw: None,
            regen_power_kw: None,
        }
    }

    #[test]
    fn derives_signed_net_power_from_soc_change() {
        let rows = vec![row(0, 80.0), row(30, 79.5), row(60, 79.8)];
        let data = derive_soc_power(&rows, 60);

        assert!(matches!(
            data.metadata.source,
            TripPowerSource::EstimatedSoc
        ));
        assert_eq!(data.metadata.sample_count, 2);
        assert!(data.intervals[0].net_power_kw > 0.0);
        assert!(data.intervals[1].net_power_kw < 0.0);
        assert_eq!(data.metadata.median_interval_seconds, Some(30.0));
        assert_eq!(data.metadata.coverage_percent, Some(100.0));
    }

    #[test]
    fn rejects_sparse_or_large_soc_jumps() {
        let rows = vec![row(0, 80.0), row(301, 79.0), row(302, 78.0)];
        let data = derive_soc_power(&rows, 302);

        assert!(matches!(data.metadata.source, TripPowerSource::Unavailable));
        assert!(data.intervals.is_empty());
    }

    #[test]
    fn direct_power_wins_when_multiple_samples_exist() {
        let mut first = row(0, 80.0);
        first.power_kw = Some(42.0);
        let mut second = row(10, 80.0);
        second.regen_power_kw = Some(8.0);

        let data = derive_soc_power(&[first, second], 10);
        assert!(matches!(data.metadata.source, TripPowerSource::Direct));
        assert_eq!(data.metadata.sample_count, 2);
        assert!(data.intervals.is_empty());
    }

    #[test]
    fn estimated_at_uses_half_open_intervals() {
        let rows = vec![row(0, 80.0), row(30, 79.5)];
        let data = derive_soc_power(&rows, 30);
        let start = Utc.timestamp_opt(0, 0).single().unwrap();
        assert!(data.estimated_at(start).is_some());
        assert!(data.estimated_at(start + Duration::seconds(30)).is_none());
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use http::{Request, StatusCode};
    use tower::ServiceExt;

    // Run with: cargo test -- --ignored

    async fn make_app() -> axum::Router {
        use crate::middleware::auth::{AppState, JwtKeys};
        use std::sync::Arc;

        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());

        let pool = crate::db::pool::create_pool(&database_url)
            .await
            .expect("create_pool");
        let redis = redis::Client::open(redis_url).expect("redis client");

        let keys = crate::keys::generate_keys().expect("generate test keys");
        let jwt_keys =
            Arc::new(JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).expect("jwt keys"));

        let config = crate::config::Config {
            database_url: database_url.clone(),
            redis_url: "redis://127.0.0.1/".into(),
            jwt_secret: None,
            jwt_public_key: None,
            age_encryption_key: None,
            port: 3001,
            allowed_origins: vec!["http://localhost:3000".into()],
            s3_endpoint: None,
            s3_access_key: None,
            s3_secret_key: None,
            backup_artifact_dir: std::env::temp_dir()
                .join("panda-route-test-backups")
                .to_string_lossy()
                .into_owned(),
            vehicle_image_cache_dir: std::env::temp_dir()
                .join("panda-route-test-vehicle-images")
                .to_string_lossy()
                .into_owned(),
            backup_driver: "pg_dump".into(),
            backup_poll_interval_seconds: 60,
            restore_agent_url: "http://127.0.0.1:3002".into(),
            restore_agent_key_file: "/backups/.restore-agent-key".into(),
            recovery: crate::config::RecoveryConfig::default(),
            origin_bind: crate::config::OriginBindConfig::default(),
            security: Default::default(),
            rivian_ws_reconnect_initial_seconds: 10,
            rivian_ws_reconnect_max_seconds: 900,
            rivian_raw_event_retention_days: 7,
            rivian_persist_raw_events: true,
            rivian_suppress_duplicate_telemetry: true,
            panda_env: None,
            cookie_insecure: None,
            allow_insecure_lan_http_auth: false,
            rate_limit: crate::config::RateLimitConfig::default(),
        };

        let state = AppState {
            pool,
            redis,
            jwt_keys,
            age_key: "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
                .to_string(),
            config,
            nominatim_cache: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            resources: Default::default(),
        };

        crate::routes::build_router(state)
    }

    async fn get_status(app: axum::Router, uri: &str) -> http::StatusCode {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn list_trips_requires_auth() {
        let app = make_app().await;
        assert_eq!(get_status(app, "/v1/trips").await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_map_requires_auth() {
        let app = make_app().await;
        assert_eq!(
            get_status(app, "/v1/trips/map").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn tire_pressure_timeline_requires_auth() {
        let app = make_app().await;
        assert_eq!(
            get_status(app, "/v1/trips/tire-pressure-timeline").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_detail_requires_auth() {
        let app = make_app().await;
        let status = get_status(app, &format!("/v1/trips/{}", uuid::Uuid::new_v4())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_detail_data_requires_auth() {
        let app = make_app().await;
        let status = get_status(app, &format!("/v1/trips/{}/detail", uuid::Uuid::new_v4())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_track_requires_auth() {
        let app = make_app().await;
        let status = get_status(app, &format!("/v1/trips/{}/track", uuid::Uuid::new_v4())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_speed_requires_auth() {
        let app = make_app().await;
        let status = get_status(app, &format!("/v1/trips/{}/speed", uuid::Uuid::new_v4())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_elevation_requires_auth() {
        let app = make_app().await;
        let status = get_status(
            app,
            &format!("/v1/trips/{}/elevation", uuid::Uuid::new_v4()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn trip_series_requires_auth() {
        let app = make_app().await;
        let status = get_status(app, &format!("/v1/trips/{}/series", uuid::Uuid::new_v4())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[cfg(test)]
#[path = "__tests__/trip_queries/contracts.rs"]
mod query_contracts;

#[cfg(test)]
#[path = "__tests__/derived_trip_series.rs"]
mod derived_series_tests;

enum TripRead {
    Power(Uuid),
    Series(Uuid),
}
impl TripRead {
    async fn respond(
        self,
        state: &AppState,
        auth: &AuthUser,
        vehicle: Option<Uuid>,
    ) -> Result<Json<serde_json::Value>, AppError> {
        let request = super::VehicleRequest::select(auth, vehicle)?;
        match self {
            Self::Power(id) => power_profile_response(state, auth.user_id, request.id, id).await,
            Self::Series(id) => trip_series_response(state, auth.user_id, request.id, id).await,
        }
    }
}

#[cfg(test)]
mod full_replacement_weather_characterization {
    use super::*;

    #[test]
    fn nearest_weather_preserves_input_order_ties_and_existing_values() {
        let mut samples = TripDetailSamples {
            elapsed_s: vec![20, 0, 20, 40, 10],
            outside_temp_c: vec![None, Some(90.0), None, None],
            ..Default::default()
        };
        let rows = [(30, 7.0), (20, 8.0), (0, 9.0), (10, 10.0), (41, 11.0)].map(
            |(elapsed_seconds, temperature_c)| TripWeatherRow {
                sampled_at: DateTime::UNIX_EPOCH,
                elapsed_seconds,
                temperature_c,
            },
        );
        merge_weather_samples(&mut samples, &rows);
        assert_eq!(
            samples.outside_temp_c,
            vec![Some(7.0), Some(90.0), None, Some(11.0)]
        );
        let mut empty = TripDetailSamples::default();
        merge_weather_samples(&mut empty, &rows);
        assert!(empty.outside_temp_c.is_empty());
    }
}

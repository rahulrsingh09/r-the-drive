#[path = "metrics/catalog.rs"]
mod catalog;
#[path = "metrics/planning.rs"]
mod planning;
#[path = "metrics/streaming.rs"]
mod streaming;
use catalog::METRICS;

use axum::{
    extract::{DefaultBodyLimit, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{require_vehicle_access, AppState, AuthUser},
    routes::efficiency_math::weighted_average_from_totals,
    routes::trip_tag_filter::{
        parse_tag_filter, require_known_vehicle_tags, sql_predicate, TripTagFilter, TripTagMatch,
    },
};

pub fn router() -> Router<AppState> {
    super::route_table([
        ("/metrics/catalog", get(get_catalog)),
        ("/metrics/value", get(get_value)),
        ("/metrics/series", get(get_series)),
        ("/metrics/batch", post(get_batch)),
    ])
    .layer(DefaultBodyLimit::max(64 * 1024))
}

#[derive(Clone, Copy)]
enum MetricSource {
    Summary,
    Telemetry(&'static str),
}

#[derive(Clone, Copy)]
struct MetricDef {
    id: &'static str,
    label: &'static str,
    unit: Option<&'static str>,
    kind: &'static str,
    source_label: &'static str,
    supports_series: bool,
    default_aggregation: &'static str,
    source: MetricSource,
}

#[derive(Serialize)]
struct MetricCatalogEntry {
    id: &'static str,
    label: &'static str,
    unit: Option<&'static str>,
    kind: &'static str,
    source: &'static str,
    supports_series: bool,
    default_aggregation: &'static str,
}

#[derive(Clone, Serialize)]
struct MetricValueResponse {
    metric: String,
    value: Option<f64>,
    unit: Option<&'static str>,
    label: &'static str,
    ts: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, sqlx::FromRow)]
struct MetricSeriesPoint {
    ts: DateTime<Utc>,
    value: Option<f64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct WeightedEfficiencyRow {
    ts: DateTime<Utc>,
    total_distance_miles: Option<f64>,
    weighted_efficiency_wh_mi: Option<f64>,
}

#[derive(Deserialize)]
struct CatalogParams {}

#[derive(Deserialize)]
struct ValueParams {
    vehicle_id: Option<Uuid>,
    metric: String,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: Option<bool>,
}

#[derive(Deserialize)]
struct SeriesParams {
    vehicle_id: Option<Uuid>,
    metric: String,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: Option<bool>,
    bucket: Option<String>,
}

/// A dashboard-oriented metric request. The singular metric routes remain the
/// public compatibility surface; this route avoids one HTTP request per sensor
/// chip when a dashboard needs several values and sparklines.
#[derive(Deserialize)]
struct MetricBatchRequest {
    vehicle_id: Uuid,
    metrics: Vec<MetricBatchMetricRequest>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: Option<bool>,
    bucket: Option<String>,
    /// `full` returns every retained source point in the selected range. The
    /// compact default preserves the legacy bounded-sparkline behavior for
    /// external callers that have not opted into full density.
    density: Option<String>,
    max_points: Option<usize>,
    tag_ids: Option<String>,
    tag_match: Option<TripTagMatch>,
    untagged: Option<bool>,
}

#[derive(Deserialize)]
struct MetricBatchMetricRequest {
    metric: String,
    #[serde(default = "default_true")]
    include_latest: bool,
    #[serde(default = "default_true")]
    include_series: bool,
}

const DASHBOARD_METRIC_MAX_POINTS: usize = 96;

fn default_true() -> bool {
    true
}

fn resolve_time_bounds(
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: bool,
    default_days: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    super::time_bounds(from, to, lifetime, default_days)
}

fn resolve_bucket(
    requested: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<&'static str, AppError> {
    let requested = requested.unwrap_or("auto");
    if requested == "auto" {
        let minutes = (to - from).num_minutes();
        return Ok([
            (60, "minute"),
            (360, "5min"),
            (1440, "15min"),
            (10080, "hour"),
        ]
        .into_iter()
        .find_map(|(limit, name)| (minutes <= limit).then_some(name))
        .unwrap_or("day"));
    }
    [
        ("minute", "minute"),
        ("1min", "minute"),
        ("5min", "5min"),
        ("15min", "15min"),
        ("hour", "hour"),
        ("1h", "hour"),
        ("day", "day"),
        ("1d", "day"),
        ("raw", "raw"),
        ("full", "raw"),
    ]
    .into_iter()
    .find_map(|(alias, name)| (requested == alias).then_some(name))
    .ok_or_else(|| AppError::Validation(format!("unsupported bucket: {requested}")))
}

fn resolve_batch_density(
    density: Option<&str>,
    requested_bucket: Option<&str>,
    requested_max_points: Option<usize>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<(&'static str, &'static str, Option<usize>), AppError> {
    match density.unwrap_or("compact") {
        "full" => Ok(("full", "raw", None)),
        "compact" => Ok((
            "compact",
            resolve_bucket(requested_bucket, from, to)?,
            Some(
                requested_max_points
                    .unwrap_or(DASHBOARD_METRIC_MAX_POINTS)
                    .clamp(2, DASHBOARD_METRIC_MAX_POINTS),
            ),
        )),
        other => Err(AppError::Validation(format!(
            "unsupported density: {other}"
        ))),
    }
}

pub(crate) fn is_series_metric(metric_id: &str) -> bool {
    METRICS
        .iter()
        .any(|metric| metric.id == metric_id && metric.supports_series)
}

async fn get_catalog(
    _auth: AuthUser,
    Query(_p): Query<CatalogParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let metrics = catalog::entries();
    Ok(Json(serde_json::json!({ "metrics": metrics })))
}

async fn get_value(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<ValueParams>,
) -> Result<Json<MetricValueResponse>, AppError> {
    let vehicle = super::VehicleRequest::select(&auth, p.vehicle_id)?
        .read(&state)
        .await?;
    let metric = find_metric(&p.metric)?;
    let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 30);
    let permit = state
        .resources
        .heavy(auth.user_id, &state.config.security)?;
    let seconds = state.config.security.metrics_timeout_seconds;
    let operation = async {
        let mut read =
            crate::services::admitted_read::AdmittedRead::begin(&state.pool, permit, seconds)
                .await?;
        let result = metric_value(
            &mut read,
            vehicle.id,
            metric,
            from,
            to,
            &TripTagFilter::default(),
        )
        .await?;
        read.commit().await?;
        Ok(Json(MetricValueResponse {
            metric: metric.id.to_owned(),
            value: result.0,
            unit: metric.unit,
            label: metric.label,
            ts: result.1,
        }))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(seconds), operation).await {
        Ok(result) => result,
        Err(_) => Err(AppError::DependencyUnavailable(
            "Metric query deadline exceeded".into(),
        )),
    }
}

async fn get_series(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<SeriesParams>,
) -> Result<axum::response::Response, AppError> {
    let vehicle = super::VehicleRequest::select(&auth, p.vehicle_id)?
        .read(&state)
        .await?;
    let metric = find_metric(&p.metric)?;
    let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 30);
    let bucket = resolve_bucket(p.bucket.as_deref(), from, to)?;
    let tag_filter = TripTagFilter::default();
    let query = metric_series(metric, bucket, &tag_filter)?;
    let request = StreamRequest {
        vehicle_id: vehicle.id,
        from,
        to,
        tag_filter,
        metrics: vec![(metric, false, true, Some(query))],
        bucket,
        density: "full",
        max_points: None,
        standalone: true,
    };
    let permit = state
        .resources
        .heavy(auth.user_id, &state.config.security)?;
    Ok(stream_metrics(state, request, permit))
}

async fn get_batch(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(p): Json<MetricBatchRequest>,
) -> Result<axum::response::Response, AppError> {
    let requested = catalog::select(p.metrics)?;

    require_vehicle_access(&auth, p.vehicle_id)?;
    require_vehicle_read_access(&state.pool, &auth, p.vehicle_id).await?;
    let tag_filter = parse_tag_filter(p.tag_ids.as_deref(), p.tag_match, p.untagged)?;
    require_known_vehicle_tags(&state.pool, p.vehicle_id, &tag_filter).await?;
    let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 30);
    let (density, bucket, max_points) = resolve_batch_density(
        p.density.as_deref(),
        p.bucket.as_deref(),
        p.max_points,
        from,
        to,
    )?;

    let mut metrics = Vec::with_capacity(requested.len());
    for catalog::Selection {
        metric,
        latest,
        series,
    } in requested
    {
        let query = if series {
            Some(metric_series(metric, bucket, &tag_filter)?)
        } else {
            None
        };
        metrics.push((metric, latest, series, query));
    }
    let permit = state
        .resources
        .heavy(auth.user_id, &state.config.security)?;
    Ok(stream_metrics(
        state,
        StreamRequest {
            vehicle_id: p.vehicle_id,
            from,
            to,
            tag_filter,
            metrics,
            bucket,
            density,
            max_points,
            standalone: false,
        },
        permit,
    ))
}

async fn metric_value(
    conn: &mut sqlx::PgConnection,
    vehicle_id: Uuid,
    metric: &MetricDef,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tag_filter: &TripTagFilter,
) -> Result<(Option<f64>, Option<DateTime<Utc>>), AppError> {
    let query = planning::QueryPlan::latest(metric, tag_filter)?;
    let row = query
        .bind(vehicle_id, from, to, tag_filter)
        .fetch_optional(&mut *conn)
        .await?;
    match row {
        Some(row) => {
            let point = query.projection.point(&row)?;
            Ok((point.value, Some(point.ts)))
        }
        None => Ok((None, None)),
    }
}

fn metric_series(
    metric: &MetricDef,
    bucket: &str,
    tag_filter: &TripTagFilter,
) -> Result<SeriesQuery, AppError> {
    planning::QueryPlan::series(metric, bucket, tag_filter)
}

type SeriesQuery = planning::QueryPlan;

struct StreamRequest {
    vehicle_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tag_filter: TripTagFilter,
    metrics: Vec<(&'static MetricDef, bool, bool, Option<SeriesQuery>)>,
    bucket: &'static str,
    density: &'static str,
    max_points: Option<usize>,
    standalone: bool,
}

/// Bounded producer/consumer buffering. Dropping the response cancels the
/// producer. Its read guard cancels the backend and retains admission until
/// rollback has drained the server response and closed its cursor.
fn stream_metrics(
    state: AppState,
    request: StreamRequest,
    permit: crate::services::resource_limits::ResourcePermit,
) -> axum::response::Response {
    use axum::{
        body::{Body, Bytes},
        response::IntoResponse,
    };
    use futures::StreamExt;
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(2);
    let task = tokio::spawn(async move {
        let mut output = MetricOutput {
            sender,
            buffer: Vec::with_capacity(64 * 1024),
        };
        output.produce(state, request, permit).await;
    });
    let stream = futures::stream::unfold(
        (receiver, MetricTask(task)),
        |(mut receiver, task)| async move {
            receiver.recv().await.map(|chunk| (chunk, (receiver, task)))
        },
    )
    .fuse();
    (
        [
            ("content-type", "application/json"),
            ("x-accel-buffering", "no"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

struct MetricOutput {
    sender: tokio::sync::mpsc::Sender<Result<axum::body::Bytes, std::io::Error>>,
    buffer: Vec<u8>,
}
impl MetricOutput {
    async fn write(&mut self, bytes: &[u8]) -> Result<(), AppError> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= 64 * 1024 {
            self.flush().await?;
        }
        Ok(())
    }
    async fn json(&mut self, value: &impl Serialize) -> Result<(), AppError> {
        let original_len = self.buffer.len();
        if let Err(error) = serde_json::to_writer(&mut self.buffer, value) {
            self.buffer.truncate(original_len);
            return Err(AppError::Internal(error.into()));
        }
        if self.buffer.len() >= 64 * 1024 {
            self.flush().await?;
        }
        Ok(())
    }
    async fn flush(&mut self) -> Result<(), AppError> {
        if !self.buffer.is_empty() {
            self.sender
                .send(Ok(std::mem::replace(
                    &mut self.buffer,
                    Vec::with_capacity(64 * 1024),
                )
                .into()))
                .await
                .map_err(|_| {
                    AppError::DependencyUnavailable("Metric client disconnected".into())
                })?;
        }
        Ok(())
    }
}

fn bind_series<'a>(
    sql: &'a str,
    request: &StreamRequest,
    filtered: bool,
) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let query = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(request.vehicle_id)
        .bind(request.from)
        .bind(request.to);
    if filtered {
        query
            .bind(request.tag_filter.tag_ids.clone())
            .bind(request.tag_filter.match_all)
            .bind(request.tag_filter.untagged)
    } else {
        query
    }
}

#[cfg(test)]
fn cap_metric_points(points: Vec<MetricSeriesPoint>, max_points: usize) -> Vec<MetricSeriesPoint> {
    if points.len() <= max_points {
        return points;
    }

    let last_index = points.len() - 1;
    (0..max_points)
        .map(|index| {
            let source_index = index * last_index / (max_points - 1);
            points[source_index].clone()
        })
        .collect()
}

fn find_metric(id: &str) -> Result<&'static MetricDef, AppError> {
    catalog::lookup(id)
}

const ALLOWED_TELEMETRY_COLUMNS: &[&str] = &[
    "battery_level",
    "distance_to_empty_mi",
    "odometer_miles",
    "outside_temp_c",
    "speed_mph",
    "power_kw",
    "tire_fl_psi",
    "tire_fr_psi",
    "tire_rl_psi",
    "tire_rr_psi",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `MetricSource::Telemetry(column)` in the METRICS registry must
    /// appear in `ALLOWED_TELEMETRY_COLUMNS` — otherwise the `get_value` and
    /// `get_series` handlers would always return a validation error for that
    /// metric, silently making it unavailable.
    #[test]
    fn all_telemetry_metric_columns_are_in_allowlist() {
        let unguarded: Vec<&str> = METRICS
            .iter()
            .filter_map(|m| {
                if let MetricSource::Telemetry(col) = m.source {
                    if !ALLOWED_TELEMETRY_COLUMNS.contains(&col) {
                        Some(col)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(
            unguarded.is_empty(),
            "MetricDef::Telemetry columns missing from ALLOWED_TELEMETRY_COLUMNS: {unguarded:?}"
        );
    }

    /// `ALLOWED_TELEMETRY_COLUMNS` must not contain duplicate entries — a
    /// duplicate would be a sign of a copy-paste error with no runtime impact,
    /// but catches future refactoring mistakes.
    #[test]
    fn allowed_telemetry_columns_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for col in ALLOWED_TELEMETRY_COLUMNS {
            assert!(
                seen.insert(*col),
                "duplicate column in ALLOWED_TELEMETRY_COLUMNS: {col}"
            );
        }
    }

    /// Metric IDs must be unique — duplicate IDs would cause `find_metric` to
    /// always return the first match, silently shadowing the later definition.
    #[test]
    fn metric_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in METRICS {
            assert!(seen.insert(m.id), "duplicate metric id: {}", m.id);
        }
    }

    #[test]
    fn metric_lookup_rejects_control_and_oversized_identifiers() {
        assert!(find_metric("total_trips\n").is_err());
        assert!(find_metric(&"x".repeat(129)).is_err());
    }

    #[test]
    fn compact_batch_point_cap_keeps_the_first_and_last_samples() {
        let start = Utc::now();
        let points = (0..200)
            .map(|offset| MetricSeriesPoint {
                ts: start + chrono::Duration::seconds(offset),
                value: Some(offset as f64),
            })
            .collect();

        let capped = cap_metric_points(points, DASHBOARD_METRIC_MAX_POINTS);
        assert_eq!(capped.len(), DASHBOARD_METRIC_MAX_POINTS);
        assert_eq!(capped.first().and_then(|point| point.value), Some(0.0));
        assert_eq!(capped.last().and_then(|point| point.value), Some(199.0));
    }

    #[test]
    fn batch_limit_never_exceeds_the_dashboard_budget() {
        assert_eq!(
            128usize.clamp(2, DASHBOARD_METRIC_MAX_POINTS),
            DASHBOARD_METRIC_MAX_POINTS
        );
    }

    #[test]
    fn full_batch_density_selects_raw_rows_without_a_point_cap() {
        let to = Utc::now();
        let from = to - chrono::Duration::days(365);
        let (density, bucket, max_points) =
            resolve_batch_density(Some("full"), Some("day"), Some(2), from, to).unwrap();

        assert_eq!(density, "full");
        assert_eq!(bucket, "raw");
        assert_eq!(max_points, None);
    }

    #[test]
    fn raw_bucket_is_available_to_the_singular_series_endpoint() {
        let to = Utc::now();
        let from = to - chrono::Duration::days(30);
        assert_eq!(resolve_bucket(Some("raw"), from, to).unwrap(), "raw");
    }
}

#[cfg(test)]
#[path = "__tests__/metrics_queries/contracts.rs"]
mod query_contracts;

struct MetricTask(tokio::task::JoinHandle<()>);
impl Drop for MetricTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl MetricOutput {
    async fn produce(
        &mut self,
        state: AppState,
        request: StreamRequest,
        permit: crate::services::resource_limits::ResourcePermit,
    ) {
        let sender = self.sender.clone();
        let deadline =
            std::time::Duration::from_secs(state.config.security.metrics_timeout_seconds);
        let result = tokio::select! {
            biased;
            _ = sender.closed() => return,
            completed = tokio::time::timeout(deadline, streaming::produce(&state, &request, self, permit)) => completed,
        };
        let failure = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => AppError::DependencyUnavailable("Metric request timed out".into()),
        };
        tracing::warn!(error = ?failure, "metric response interrupted");
        let _ = sender
            .send(Err(std::io::Error::other("metric response interrupted")))
            .await;
    }
}

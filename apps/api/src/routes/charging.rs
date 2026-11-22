mod curves;
mod fallback_series;
mod history;
mod mutations;
mod summary;
use super::charging_sql as c;
use crate::{
    db::vehicles::{
        require_vehicle_manager_access, require_vehicle_membership, require_vehicle_read_access,
    },
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::{charging::PLAUSIBLE_MAX_CHARGE_KW, cost::recompute_charge_session_cost},
};
use axum::{
    extract::{Path, Query, State},
    routing::{get, patch},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub fn router() -> Router<AppState> {
    super::route_table([
        ("/charging", get(list_sessions)),
        ("/charging/chart-series", get(get_chart_series)),
        ("/charging/summary", get(get_summary)),
        ("/charging/sessions", get(list_sessions)),
        ("/charging/sessions/{id}/curve", get(get_session_curve)),
        ("/charging/sessions/{id}", get(get_session)),
        ("/charging/{id}/curve", get(get_session_curve)),
        ("/charging/{id}", get(get_session)),
        ("/charging/{id}", patch(update_session_location)),
        ("/charging/curve-analysis", get(get_curve_analysis)),
        (
            "/vehicles/{vehicle_id}/charging-sessions",
            get(list_sessions_path),
        ),
        (
            "/vehicles/{vehicle_id}/charging-sessions/{id}/curve",
            get(get_session_curve_path),
        ),
        (
            "/vehicles/{vehicle_id}/charging-sessions/{id}",
            get(get_session_path),
        ),
        (
            "/vehicles/{vehicle_id}/charging-sessions/{id}",
            patch(update_session_location_path),
        ),
        (
            "/vehicles/{vehicle_id}/charging-networks",
            get(list_network_preferences),
        ),
        (
            "/vehicles/{vehicle_id}/charging-networks/{vendor}",
            patch(update_network_preference),
        ),
        ("/vehicles/{vehicle_id}/costs", get(get_summary_path)),
    ])
}
#[derive(Deserialize)]
struct SessionListParams {
    vehicle_id: Option<Uuid>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    lifetime: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
    page: Option<i64>,
    per_page: Option<i64>,
    session_day_local: Option<String>,
}
#[derive(Deserialize)]
struct VehicleParam {
    vehicle_id: Option<Uuid>,
}
/// Largest manual session cost accepted, in USD. The derived
/// `cost_rate_per_kwh` divides this total by the energy added, so a bound keeps
/// that division finite.
const MAX_MANUAL_COST_USD: f64 = 100_000.0;

/// A manual cost must be a finite number from 0 to `MAX_MANUAL_COST_USD`, and
/// `cost_usd` is only valid with the manual cost mode.
fn validate_manual_cost(
    cost_mode: Option<CostOverrideMode>,
    cost_usd: Option<f64>,
) -> Result<(), AppError> {
    let manual = cost_mode == Some(CostOverrideMode::Manual);
    if manual
        && !cost_usd
            .is_some_and(|value| value.is_finite() && (0.0..=MAX_MANUAL_COST_USD).contains(&value))
    {
        return Err(AppError::Validation(
            "cost_usd must be a number from 0 to 100000 when cost_mode is manual".into(),
        ));
    }
    if !manual && cost_usd.is_some() {
        return Err(AppError::Validation(
            "cost_usd is only valid when cost_mode is manual".into(),
        ));
    }
    Ok(())
}
#[derive(Deserialize)]
struct SessionLocationUpdate {
    /// Legacy field. `null` retains its historical meaning: no location.
    #[serde(default)]
    place_id: PatchField<Uuid>,
    location_mode: Option<LocationOverrideMode>,
    cost_mode: Option<CostOverrideMode>,
    cost_usd: Option<f64>,
}
/// Distinguishes an omitted patch field from JSON `null`; this is required to
/// preserve `{ "place_id": null }` behavior while allowing a cost-only patch.
#[derive(Debug, Clone, Copy, Default)]
enum PatchField<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PatchField<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LocationOverrideMode {
    Automatic,
    SavedPlace,
    None,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CostOverrideMode {
    Automatic,
    Free,
    Manual,
}

impl CostOverrideMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Free => "free",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Deserialize)]
struct NetworkPreferenceUpdate {
    cost_mode: NetworkCostMode,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NetworkCostMode {
    Automatic,
    Free,
}

impl NetworkCostMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Free => "free",
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct ChargingNetworkPreferenceRow {
    network_vendor: String,
    cost_mode: String,
    session_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct PlaceLookupRow {
    address_id: Option<Uuid>,
    is_home: bool,
    latitude: Option<f64>,
    longitude: Option<f64>,
    cost_profile_id: Option<Uuid>,
}

/// Cost fields for session list and detail. `cost_method` follows `cost_usd`
/// and stays `null` while the session is open. `cost_rate_per_kwh` is a
/// derived display value for `user_manual`: the total divided by the energy
/// added that the session shows (`kwh_added`), `null` when that energy is 0 or
/// missing. The web app saves a rate entered on a charge as such a total.
pub(crate) const COST_RESOLUTION_SQL: &str = "\
    CASE WHEN cs.ended_at IS NULL THEN NULL ELSE cs.cost_method END AS cost_method, \
    CASE WHEN cs.ended_at IS NOT NULL AND cs.cost_method = 'user_manual' \
              AND COALESCE(cs.kwh_added, cs.energy_added_wh / 1000.0) > 0 \
         THEN cs.cost_usd / COALESCE(cs.kwh_added, cs.energy_added_wh / 1000.0) \
    END AS cost_rate_per_kwh";

#[derive(Debug, Serialize, sqlx::FromRow)]
struct SessionRow {
    id: Uuid,
    started_at: DateTime<Utc>,
    #[sqlx(default)]
    session_day_local: Option<String>,
    ended_at: Option<DateTime<Utc>>,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
    #[sqlx(default)]
    source_location_lat: Option<f64>,
    #[sqlx(default)]
    source_location_lng: Option<f64>,
    #[sqlx(default)]
    location_override_mode: Option<String>,
    #[sqlx(default)]
    cost_override_mode: Option<String>,
    #[sqlx(default)]
    cost_override_usd: Option<f64>,
    #[sqlx(default)]
    cost_method: Option<String>,
    #[sqlx(default)]
    cost_rate_per_kwh: Option<f64>,
    location_name: Option<String>,
    is_home: Option<bool>,
    charger_type: Option<String>,
    kwh_added: Option<f64>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    max_charge_rate_kw: Option<f64>,
    duration_minutes: Option<i32>,
    cost_usd: Option<f64>,
    // Enrichment fields (migration 0024)
    network_vendor: Option<String>,
    range_added_km: Option<f64>,
    is_free_session: Option<bool>,
    is_rivian_network: Option<bool>,
    rivian_paid_total: Option<f64>,
    source: Option<String>,
    telemetry_sample_count: i64,
    rivian_charger_type: Option<String>,
    currency_code: Option<String>,
    rivian_city: Option<String>,
    is_public: Option<bool>,
    charger_id: Option<String>,
    live_current_price: Option<f64>,
    live_current_currency: Option<String>,
    live_total_charged_kwh: Option<f64>,
    live_range_added_km: Option<f64>,
    live_power_kw: Option<f64>,
    live_charge_rate_kph: Option<f64>,
    live_time_elapsed_seconds: Option<i32>,
    live_time_remaining_min: Option<i32>,
    live_started_at: Option<DateTime<Utc>>,
    live_soc_pct: Option<f64>,
    live_charger_state: Option<String>,
    live_charger_status: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct NetworkBreakdownRow {
    network_vendor: Option<String>,
    session_count: i64,
    energy_kwh: Option<f64>,
    cost_usd: Option<f64>,
    free_sessions: i64,
}

#[allow(dead_code)]
#[derive(Debug, sqlx::FromRow)]
struct SessionBoundsRow {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<String>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
}

#[cfg(test)]
mod timeframe_tests {
    use chrono::{TimeZone, Utc};

    #[test]
    fn lifetime_time_bounds_use_epoch_instead_of_default_window() {
        let to = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
        let (from, resolved_to) = super::resolve_time_bounds(None, Some(to), true, 365);

        assert_eq!(resolved_to, to);
        assert_eq!(from, chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap());
    }

    #[test]
    fn session_patch_distinguishes_omitted_and_null_legacy_place_id() {
        let omitted: super::SessionLocationUpdate = serde_json::from_str("{}").unwrap();
        assert!(matches!(omitted.place_id, super::PatchField::Absent));

        let cleared: super::SessionLocationUpdate =
            serde_json::from_str(r#"{"place_id":null}"#).unwrap();
        assert!(matches!(cleared.place_id, super::PatchField::Null));
    }

    #[test]
    fn location_mode_changes_replace_or_clear_the_explicit_cost_profile() {
        let place_profile = uuid::Uuid::new_v4();

        assert_eq!(
            super::cost_profile_for_location(
                super::LocationOverrideMode::SavedPlace,
                Some(place_profile),
            ),
            Some(place_profile),
        );
        assert_eq!(
            super::cost_profile_for_location(super::LocationOverrideMode::Automatic, None),
            None,
        );
        assert_eq!(
            super::cost_profile_for_location(super::LocationOverrideMode::None, None),
            None,
        );
    }
}

fn cost_profile_for_location(
    mode: LocationOverrideMode,
    saved_place_profile_id: Option<Uuid>,
) -> Option<Uuid> {
    match mode {
        LocationOverrideMode::SavedPlace => saved_place_profile_id,
        LocationOverrideMode::Automatic | LocationOverrideMode::None => None,
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

#[allow(clippy::too_many_arguments)]
fn build_fallback_curve_samples(
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    charger_type: Option<&str>,
    soc_bounds: Option<(f64, f64)>,
    points: &[RawCurvePointRow],
    cap_kw: f64,
) -> Vec<CurveSampleRow> {
    fallback_series::build(
        session_id,
        started_at,
        ended_at,
        charger_type,
        soc_bounds,
        points,
        cap_kw,
    )
}

#[derive(Debug, sqlx::FromRow)]
struct SummaryAggRow {
    total_kwh: Option<f64>,
    session_count: i64,
    home_kwh: Option<f64>,
    away_kwh: Option<f64>,
    unknown_location_kwh: Option<f64>,
    ac_kwh: Option<f64>,
    ac_l2_kwh: Option<f64>,
    dc_kwh: Option<f64>,
    typed_session_count: i64,
    max_charge_limit_pct: Option<f64>,
    max_charge_rate_kw: Option<f64>,
    total_energy_used_kwh: Option<f64>,
    total_cost_usd: Option<f64>,
    known_cost_session_count: i64,
    unknown_cost_session_count: i64,
    // Enrichment fields (migration 0024)
    free_session_count: i64,
    total_range_added_km: Option<f64>,
    rivian_paid_total_usd: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct WeeklySummaryRow {
    week_start: Option<DateTime<Utc>>,
    kwh: Option<f64>,
    cost_usd: Option<f64>,
    sessions: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct CurveRow {
    minutes_elapsed: Option<f64>,
    charge_rate_kw: Option<f64>,
    soc: Option<f64>,
    #[sqlx(default)]
    power_method: Option<String>,
    #[sqlx(default)]
    sample_source: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct CurveAnalysisSessionRow {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<String>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
}

#[derive(Debug, Serialize, Clone)]
struct CurveSampleRow {
    session_id: Uuid,
    minutes_elapsed: f64,
    charge_rate_kw: f64,
    soc: Option<f64>,
    sample_source: String,
    charger_type: Option<String>,
    power_method: &'static str,
}

#[derive(Debug, sqlx::FromRow)]
struct RawCurvePointRow {
    ts: DateTime<Utc>,
    power_kw: Option<f64>,
    #[sqlx(default)]
    soc: Option<f64>,
    sample_source: String,
}

/// Recorded charge power for a session: the Parallax charging graph (which
/// also carries state of charge) and the Rivian live-session points. A
/// Parallax point wins an exact timestamp collision because it carries the
/// richer sample.
async fn load_recorded_curve_points(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<Vec<RawCurvePointRow>, AppError> {
    Ok(sqlx::query_as::<_, RawCurvePointRow>(
        r#"WITH candidates AS (
               SELECT ts,power_kw,NULL::float8 AS soc,'rivian_charge_curve_points'::text AS sample_source,1 AS precedence
               FROM panda.rivian_charge_curve_points
               WHERE vehicle_id=$1 AND charge_session_id=$2 AND power_kw IS NOT NULL AND power_kw > 0
               UNION ALL
               SELECT source_at AS ts,power_kw,soc,'parallax_charge_curve_points'::text AS sample_source,2 AS precedence
               FROM timeseries.parallax_charge_curve_points
               WHERE vehicle_id=$1 AND charge_session_id=$2 AND power_kw IS NOT NULL AND power_kw > 0
           )
           SELECT DISTINCT ON (ts) ts,power_kw,soc,sample_source
           FROM candidates
           WHERE ts >= $3 - interval '2 minutes' AND ts <= $4 + interval '2 minutes'
           ORDER BY ts,precedence DESC"#,
    )
    .bind(vehicle_id)
    .bind(session_id)
    .bind(started_at)
    .bind(ended_at)
    .fetch_all(pool)
    .await?)
}

fn recorded_curve_json(
    points: &[RawCurvePointRow],
    started_at: DateTime<Utc>,
) -> Vec<serde_json::Value> {
    points
        .iter()
        .map(|point| {
            serde_json::json!({
                "minutes_elapsed": (point.ts - started_at).num_milliseconds() as f64 / 60_000.0,
                "charge_rate_kw": point.power_kw.map(|kw| kw.clamp(0.0, PLAUSIBLE_MAX_CHARGE_KW)),
                "soc": point.soc,
                "sample_source": point.sample_source,
                "power_method": "recorded",
            })
        })
        .collect()
}

#[allow(dead_code)]
#[derive(Debug, sqlx::FromRow)]
struct CurveAnalysisValueRow {
    ts: DateTime<Utc>,
    minutes_elapsed: f64,
    charge_rate_kw: Option<f64>,
    soc: Option<f64>,
    #[sqlx(default)]
    power_method: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct CapacitySourcesRow {
    vehicle_capacity_wh: Option<f64>,
    telemetry_latest_capacity_wh: Option<f64>,
    telemetry_max_capacity_wh: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct ChartSeriesSessionSourceRow {
    session_id: Uuid,
    started_at: DateTime<Utc>,
    #[sqlx(default)]
    session_day_local: Option<String>,
    energy_kwh: Option<f64>,
    cost_usd: Option<f64>,
    location_name: Option<String>,
    charger_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ChargeChartSeriesSession {
    session_id: Uuid,
    day_local: String,
    day_start: DateTime<Utc>,
    started_at: DateTime<Utc>,
    energy_added_kwh: Option<f64>,
    cost_usd: Option<f64>,
    charger_type: Option<String>,
    location_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ChargeChartSeriesDay {
    day_local: String,
    day_start: DateTime<Utc>,
    total_energy_kwh: f64,
    ac_energy_kwh: f64,
    dc_energy_kwh: f64,
    session_count: i64,
}

fn normalize_capacity_kwh(raw: f64) -> Option<f64> {
    [(40.0..=300.0, 1.0), (40_000.0..=300_000.0, 1000.0)]
        .into_iter()
        .find_map(|(bounds, divisor)| bounds.contains(&raw).then_some(raw / divisor))
}

async fn list_sessions(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<SessionListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = p.vehicle_id;
    ChargeRead::History(p).respond(&state, &auth, vehicle).await
}

async fn get_session(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = p.vehicle_id;
    ChargeRead::Session(id)
        .respond(&state, &auth, vehicle)
        .await
}

async fn get_session_curve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = p.vehicle_id;
    ChargeRead::Curve(id).respond(&state, &auth, vehicle).await
}

async fn update_session_location(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(p): Query<VehicleParam>,
    Json(payload): Json<SessionLocationUpdate>,
) -> Result<Json<serde_json::Value>, AppError> {
    change_session_location(&state, &auth, p.vehicle_id, id, payload).await
}

async fn update_session_location_path(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<SessionLocationUpdate>,
) -> Result<Json<serde_json::Value>, AppError> {
    change_session_location(&state, &auth, Some(vehicle_id), id, payload).await
}

async fn update_charge_session(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
    session_id: Uuid,
    payload: SessionLocationUpdate,
) -> Result<(), AppError> {
    mutations::update(pool, auth, vehicle_id, session_id, payload).await
}

async fn list_network_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<Vec<ChargingNetworkPreferenceRow>>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let preferences = sqlx::query_as::<_, ChargingNetworkPreferenceRow>(
        "WITH vendors AS (
           SELECT lower(trim(network_vendor)) AS normalized,MIN(trim(network_vendor)) AS network_vendor,COUNT(*)::int8 AS session_count
           FROM panda.charge_sessions WHERE vehicle_id=$1 AND network_vendor IS NOT NULL AND trim(network_vendor)<>''
           GROUP BY lower(trim(network_vendor))
         ) SELECT v.network_vendor,COALESCE(p.cost_mode,'automatic') AS cost_mode,v.session_count
         FROM vendors v LEFT JOIN panda.vehicle_charging_network_preferences p
         ON p.vehicle_id=$1 AND p.vendor_normalized=v.normalized ORDER BY lower(v.network_vendor)")
        .bind(vehicle_id).fetch_all(&state.pool).await?;
    Ok(Json(preferences))
}

async fn update_network_preference(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, vendor)): Path<(Uuid, String)>,
    Json(payload): Json<NetworkPreferenceUpdate>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(
        mutations::network(&state.pool, &auth, vehicle_id, vendor, payload).await?,
    ))
}

async fn list_sessions_path(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
    Query(p): Query<SessionListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = Some(vehicle_id);
    ChargeRead::History(p).respond(&state, &auth, vehicle).await
}

async fn get_session_path(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = Some(vehicle_id);
    ChargeRead::Session(id)
        .respond(&state, &auth, vehicle)
        .await
}

async fn get_session_curve_path(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = Some(vehicle_id);
    ChargeRead::Curve(id).respond(&state, &auth, vehicle).await
}

async fn get_summary_path(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
    Query(p): Query<SessionListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = Some(vehicle_id);
    ChargeRead::Summary(p).respond(&state, &auth, vehicle).await
}

async fn get_summary(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<SessionListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = p.vehicle_id;
    ChargeRead::Summary(p).respond(&state, &auth, vehicle).await
}

async fn get_chart_series(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<SessionListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle = p.vehicle_id;
    ChargeRead::Chart(p).respond(&state, &auth, vehicle).await
}

async fn get_curve_analysis(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(p): Query<SessionListParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    use futures::{StreamExt, TryStreamExt};
    let vehicle_id = super::VehicleRequest::select(&auth, p.vehicle_id)?
        .read(&state)
        .await?
        .id;
    let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 365);

    let sessions = CurveSelection::Completed { from, to }
        .load::<CurveAnalysisSessionRow>(&state.pool, vehicle_id)
        .await?;

    let curve_rows = futures::stream::iter(
        sessions
            .into_iter()
            .filter(|session| session.charger_type.as_deref() == Some("dc"))
            .map(|session| {
                let charger_type = session.charger_type.clone();
                load_curve_analysis_samples(
                    &state.pool,
                    vehicle_id,
                    session.id,
                    session.started_at,
                    session.ended_at,
                    charger_type,
                    session.soc_start,
                    session.soc_end,
                )
            }),
    )
    .buffered(4)
    .try_collect::<Vec<_>>()
    .await?
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    Ok(Json(serde_json::json!(curve_rows
        .iter()
        .map(|row| serde_json::json!({
            "session_id": row.session_id,
            "minutes_elapsed": row.minutes_elapsed,
            "soc_pct": row.soc,
            "charge_rate_kw": row.charge_rate_kw,
            "sample_source": row.sample_source,
            "charger_type": row.charger_type,
            "power_method": row.power_method,
        }))
        .collect::<Vec<_>>())))
}

async fn list_sessions_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    p: SessionListParams,
) -> Result<Json<serde_json::Value>, AppError> {
    history::history(state, user_id, vehicle_id, p).await
}

async fn get_chart_series_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    p: SessionListParams,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;
    let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 90);

    let rows = sqlx::query_as::<_, ChartSeriesSessionSourceRow>(concat!(
        "SELECT
            cs.id AS session_id,
            cs.started_at,
            TO_CHAR((cs.started_at AT TIME ZONE COALESCE((SELECT value FROM panda.system_config WHERE key = 'app_timezone'), 'UTC'))::date, 'YYYY-MM-DD') AS session_day_local,
            COALESCE(cs.kwh_added, cs.energy_added_wh / 1000.0) AS energy_kwh,
            cs.cost_usd,
            COALESCE(g.name, a.display_name, CASE WHEN cs.is_home THEN 'Home' END) AS location_name,
            ",
        crate::charger_type_sql!("cs"),
        " AS charger_type
         FROM panda.charge_sessions cs
         LEFT JOIN panda.geofences g ON g.id = cs.geofence_id
         LEFT JOIN panda.addresses a ON a.id = cs.address_id
         WHERE cs.vehicle_id=$1 AND cs.started_at>=$2 AND cs.started_at<=$3
         ORDER BY cs.started_at ASC, cs.id ASC",
    ))
    .bind(vehicle_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await?;

    let (daily, daily_sessions) = build_charge_chart_series(rows);

    Ok(Json(
        serde_json::json!({"daily":daily,"daily_sessions":daily_sessions}),
    ))
}

fn build_charge_chart_series(
    rows: Vec<ChartSeriesSessionSourceRow>,
) -> (Vec<ChargeChartSeriesDay>, Vec<ChargeChartSeriesSession>) {
    let mut groups = std::collections::BTreeMap::<String, Vec<ChargeChartSeriesSession>>::new();
    for row in rows {
        let day = row
            .session_day_local
            .unwrap_or_else(|| row.started_at.format("%Y-%m-%d").to_string());
        groups
            .entry(day.clone())
            .or_default()
            .push(ChargeChartSeriesSession {
                session_id: row.session_id,
                day_start: day_start_from_label(&day),
                day_local: day,
                started_at: row.started_at,
                energy_added_kwh: row.energy_kwh,
                cost_usd: row.cost_usd,
                charger_type: normalize_chart_charger_type(row.charger_type.as_deref()),
                location_name: row.location_name,
            });
    }
    let mut daily = Vec::with_capacity(groups.len());
    let mut sessions = Vec::new();
    for (day_local, mut group) in groups {
        group.sort_by_key(|session| (session.started_at, session.session_id));
        let first = &group[0];
        let mut total_energy_kwh = 0.0;
        let mut ac_energy_kwh = 0.0;
        let mut dc_energy_kwh = 0.0;
        for session in &group {
            let energy = session.energy_added_kwh.unwrap_or(0.0).max(0.0);
            total_energy_kwh += energy;
            match session.charger_type.as_deref() {
                Some("AC") => ac_energy_kwh += energy,
                Some("DC") => dc_energy_kwh += energy,
                _ => {}
            }
        }
        daily.push(ChargeChartSeriesDay {
            day_local,
            day_start: first.day_start,
            total_energy_kwh,
            ac_energy_kwh,
            dc_energy_kwh,
            session_count: group.len() as i64,
        });
        sessions.extend(group);
    }
    (daily, sessions)
}

fn day_start_from_label(day_local: &str) -> DateTime<Utc> {
    NaiveDate::parse_from_str(day_local, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date_time| DateTime::<Utc>::from_naive_utc_and_offset(date_time, Utc))
        .unwrap_or_else(Utc::now)
}

/// Map the shared SQL rule's result ('dc', 'ac' or NULL) to the chart labels.
fn normalize_chart_charger_type(charger_type: Option<&str>) -> Option<String> {
    match charger_type
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("dc") | Some("dcfc") => Some("DC".to_string()),
        Some("ac") | Some("ac_l2") => Some("AC".to_string()),
        _ => None,
    }
}

async fn get_session_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    id: Uuid,
) -> Result<Json<serde_json::Value>, AppError> {
    history::detail(state, user_id, vehicle_id, id).await
}

async fn get_session_curve_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    id: Uuid,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;
    let session = CurveSelection::Id(id)
        .load::<SessionBoundsRow>(&state.pool, vehicle_id)
        .await?
        .pop()
        .ok_or(AppError::NotFound)?;
    let samples = load_curve(
        &state.pool,
        vehicle_id,
        session.id,
        session.started_at,
        session.ended_at,
        session.charger_type.as_deref(),
    )
    .await?;
    Ok(Json(serde_json::json!(samples)))
}

async fn get_summary_response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    p: SessionListParams,
) -> Result<Json<serde_json::Value>, AppError> {
    summary::response(state, user_id, vehicle_id, p).await
}

#[cfg(test)]
mod tests {
    use super::{build_charge_chart_series, normalize_capacity_kwh, ChartSeriesSessionSourceRow};
    use axum::body::Body;
    use chrono::{TimeZone, Utc};
    use http::{Request, StatusCode};
    use tower::ServiceExt;
    use uuid::Uuid;

    #[test]
    fn normalizes_capacity_values_in_kwh_and_wh() {
        assert_eq!(normalize_capacity_kwh(135.0), Some(135.0));
        assert_eq!(normalize_capacity_kwh(135_000.0), Some(135.0));
    }

    #[test]
    fn rejects_unrealistic_capacity_values() {
        assert_eq!(normalize_capacity_kwh(0.13), None);
        assert_eq!(normalize_capacity_kwh(600.0), None);
        assert_eq!(normalize_capacity_kwh(-50.0), None);
    }

    // ── Integration tests (require DATABASE_URL) ──────────────────────────────
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
                .join("panda-route-test-image-cache")
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

    async fn get_unauthenticated(app: axum::Router, uri: &str) -> http::StatusCode {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn list_sessions_requires_auth() {
        let app = make_app().await;
        let status = get_unauthenticated(app, "/v1/charging/sessions").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn session_detail_requires_auth() {
        let app = make_app().await;
        let status = get_unauthenticated(
            app,
            &format!("/v1/charging/sessions/{}", uuid::Uuid::new_v4()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn charging_summary_requires_auth() {
        let app = make_app().await;
        let status = get_unauthenticated(app, "/v1/charging/summary").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn charging_chart_series_requires_auth() {
        let app = make_app().await;
        let status = get_unauthenticated(app, "/v1/charging/chart-series").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn session_detail_invalid_uuid() {
        // Without auth header, 401 takes precedence over UUID parse error.
        // This documents the auth-first ordering in the middleware stack.
        let app = make_app().await;
        let status = get_unauthenticated(app, "/v1/charging/sessions/not-a-uuid").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn chart_series_rolls_up_multiple_sessions_under_one_local_day() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let (daily, sessions) = build_charge_chart_series(vec![
            ChartSeriesSessionSourceRow {
                session_id: first,
                started_at: Utc.with_ymd_and_hms(2026, 6, 20, 18, 0, 0).unwrap(),
                session_day_local: Some("2026-06-20".into()),
                energy_kwh: Some(18.5),
                cost_usd: Some(2.25),
                location_name: Some("Home".into()),
                charger_type: Some("ac".into()),
            },
            ChartSeriesSessionSourceRow {
                session_id: second,
                started_at: Utc.with_ymd_and_hms(2026, 6, 20, 22, 30, 0).unwrap(),
                session_day_local: Some("2026-06-20".into()),
                energy_kwh: Some(24.0),
                cost_usd: Some(6.50),
                location_name: Some("Office".into()),
                charger_type: Some("dc".into()),
            },
        ]);

        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].day_local, "2026-06-20");
        assert!((daily[0].total_energy_kwh - 42.5).abs() < f64::EPSILON);
        assert!((daily[0].ac_energy_kwh - 18.5).abs() < f64::EPSILON);
        assert!((daily[0].dc_energy_kwh - 24.0).abs() < f64::EPSILON);
        assert_eq!(daily[0].session_count, 2);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, first);
        assert_eq!(sessions[1].session_id, second);
        assert_eq!(sessions[0].cost_usd, Some(2.25));
        assert_eq!(sessions[1].cost_usd, Some(6.50));
    }

    #[test]
    fn chart_series_uses_local_charge_day_for_day_boundaries() {
        let session = Uuid::new_v4();
        let (daily, sessions) = build_charge_chart_series(vec![ChartSeriesSessionSourceRow {
            session_id: session,
            started_at: Utc.with_ymd_and_hms(2026, 6, 21, 6, 0, 0).unwrap(),
            session_day_local: Some("2026-06-20".into()),
            energy_kwh: Some(12.0),
            cost_usd: None,
            location_name: Some("Road Trip".into()),
            charger_type: Some("dc".into()),
        }]);

        assert_eq!(daily[0].day_local, "2026-06-20");
        assert_eq!(sessions[0].day_local, "2026-06-20");
        assert_eq!(sessions[0].charger_type.as_deref(), Some("DC"));
    }

    #[test]
    fn chart_series_includes_more_than_two_hundred_sessions_without_pagination() {
        let rows = (0..205)
            .map(|index| ChartSeriesSessionSourceRow {
                session_id: Uuid::new_v4(),
                started_at: Utc
                    .with_ymd_and_hms(2026, 6, 1 + (index / 10) as u32, (index % 24) as u32, 0, 0)
                    .unwrap(),
                session_day_local: Some(format!("2026-06-{:02}", 1 + (index / 10))),
                energy_kwh: Some(1.0),
                cost_usd: None,
                location_name: None,
                charger_type: Some("ac".into()),
            })
            .collect::<Vec<_>>();

        let (daily, sessions) = build_charge_chart_series(rows);

        assert_eq!(sessions.len(), 205);
        assert_eq!(daily.iter().map(|day| day.session_count).sum::<i64>(), 205);
    }

    #[test]
    fn fallback_curve_samples_fill_soc_using_session_bounds() {
        let session_id = Uuid::new_v4();
        let started_at = Utc.with_ymd_and_hms(2026, 6, 1, 8, 0, 0).unwrap();
        let ended_at = Utc.with_ymd_and_hms(2026, 6, 1, 8, 20, 0).unwrap();
        let points = vec![
            super::RawCurvePointRow {
                ts: Utc.with_ymd_and_hms(2026, 6, 1, 8, 2, 0).unwrap(),
                power_kw: Some(160.0),
                soc: None,
                sample_source: "rivian_charge_curve_points".to_string(),
            },
            super::RawCurvePointRow {
                ts: Utc.with_ymd_and_hms(2026, 6, 1, 8, 10, 0).unwrap(),
                power_kw: Some(120.0),
                soc: None,
                sample_source: "parallax_charge_curve_points".to_string(),
            },
        ];

        let rows = super::build_fallback_curve_samples(
            session_id,
            started_at,
            ended_at,
            Some("dc"),
            Some((18.0, 76.0)),
            &points,
            300.0,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].session_id, session_id);
        assert_eq!(rows[0].sample_source, "rivian_charge_curve_points");
        assert!((rows[0].soc.unwrap_or_default() - 18.0).abs() < 0.01);
        assert!((rows[1].soc.unwrap_or_default() - 76.0).abs() < 0.01);
        assert!(rows.iter().all(|row| row.charge_rate_kw > 0.0));
    }

    #[test]
    fn recorded_points_keep_their_measured_soc_and_power() {
        let session_id = Uuid::new_v4();
        let started_at = Utc.with_ymd_and_hms(2026, 11, 6, 19, 47, 20).unwrap();
        let ended_at = Utc.with_ymd_and_hms(2026, 11, 6, 20, 3, 4).unwrap();
        let points = vec![
            super::RawCurvePointRow {
                ts: Utc.with_ymd_and_hms(2026, 11, 6, 19, 51, 25).unwrap(),
                power_kw: Some(189.5),
                soc: Some(41.0),
                sample_source: "parallax_charge_curve_points".to_string(),
            },
            super::RawCurvePointRow {
                ts: Utc.with_ymd_and_hms(2026, 11, 6, 20, 2, 30).unwrap(),
                power_kw: Some(116.5),
                soc: Some(71.0),
                sample_source: "parallax_charge_curve_points".to_string(),
            },
        ];

        let rows = super::build_fallback_curve_samples(
            session_id,
            started_at,
            ended_at,
            Some("dc"),
            None,
            &points,
            300.0,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].soc, Some(41.0));
        assert!((rows[0].charge_rate_kw - 189.5).abs() < 0.01);
        assert_eq!(rows[1].soc, Some(71.0));

        let json = super::recorded_curve_json(&points, started_at);
        assert_eq!(json[0]["charge_rate_kw"], 189.5);
        assert_eq!(json[0]["power_method"], "recorded");
        assert!((json[1]["minutes_elapsed"].as_f64().unwrap() - 15.1667).abs() < 0.01);
    }

    #[test]
    fn chart_type_labels_follow_the_shared_rule_output() {
        assert_eq!(
            super::normalize_chart_charger_type(Some("dc")).as_deref(),
            Some("DC")
        );
        assert_eq!(
            super::normalize_chart_charger_type(Some("ac")).as_deref(),
            Some("AC")
        );
        assert_eq!(
            super::normalize_chart_charger_type(Some("ac_l2")).as_deref(),
            Some("AC")
        );
        assert_eq!(super::normalize_chart_charger_type(None), None);
    }

    #[test]
    fn manual_cost_must_be_finite_and_at_most_the_cap() {
        let manual = Some(super::CostOverrideMode::Manual);
        for ok in [0.0, 12.5, super::MAX_MANUAL_COST_USD] {
            assert!(super::validate_manual_cost(manual, Some(ok)).is_ok(), "{ok}");
        }
        for bad in [
            -0.01,
            super::MAX_MANUAL_COST_USD + 0.01,
            1e308,
            f64::INFINITY,
            f64::NAN,
        ] {
            assert!(
                matches!(
                    super::validate_manual_cost(manual, Some(bad)),
                    Err(crate::errors::AppError::Validation(_))
                ),
                "{bad}"
            );
        }
        assert!(matches!(
            super::validate_manual_cost(manual, None),
            Err(crate::errors::AppError::Validation(_))
        ));
        assert!(matches!(
            super::validate_manual_cost(Some(super::CostOverrideMode::Free), Some(1.0)),
            Err(crate::errors::AppError::Validation(_))
        ));
        assert!(super::validate_manual_cost(Some(super::CostOverrideMode::Automatic), None).is_ok());
        assert!(super::validate_manual_cost(None, None).is_ok());
    }
}

async fn load_curve(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<&str>,
) -> Result<Vec<serde_json::Value>, AppError> {
    curves::detail(
        pool,
        vehicle_id,
        session_id,
        started_at,
        ended_at,
        charger_type,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn load_curve_analysis_samples(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<String>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
) -> Result<Vec<CurveSampleRow>, AppError> {
    curves::analysis(
        pool,
        vehicle_id,
        session_id,
        started_at,
        ended_at,
        charger_type,
        soc_start,
        soc_end,
    )
    .await
}

#[cfg(test)]
#[path = "__tests__/charging_workflow.rs"]
mod workflow_contract_tests;

#[cfg(test)]
#[path = "__tests__/derived_charge_series.rs"]
mod derived_series_tests;

enum ChargeRead {
    History(SessionListParams),
    Session(Uuid),
    Curve(Uuid),
    Summary(SessionListParams),
    Chart(SessionListParams),
}

impl ChargeRead {
    async fn respond(
        self,
        state: &AppState,
        auth: &AuthUser,
        vehicle: Option<Uuid>,
    ) -> Result<Json<serde_json::Value>, AppError> {
        let request = super::VehicleRequest::select(auth, vehicle)?;
        let (user_id, vehicle_id) = (request.auth.user_id, request.id);
        match self {
            Self::History(params) => {
                list_sessions_response(state, user_id, vehicle_id, params).await
            }
            Self::Session(id) => get_session_response(state, user_id, vehicle_id, id).await,
            Self::Curve(id) => get_session_curve_response(state, user_id, vehicle_id, id).await,
            Self::Summary(params) => get_summary_response(state, user_id, vehicle_id, params).await,
            Self::Chart(params) => {
                get_chart_series_response(state, user_id, vehicle_id, params).await
            }
        }
    }
}

async fn change_session_location(
    state: &AppState,
    auth: &AuthUser,
    vehicle: Option<Uuid>,
    id: Uuid,
    payload: SessionLocationUpdate,
) -> Result<Json<serde_json::Value>, AppError> {
    let vehicle_id = vehicle.ok_or_else(|| AppError::Validation("vehicle_id required".into()))?;
    update_charge_session(&state.pool, auth, vehicle_id, id, payload).await?;
    get_session_response(state, auth.user_id, vehicle_id, id).await
}

enum CurveSelection {
    Id(Uuid),
    Completed {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
}
impl CurveSelection {
    async fn load<T>(self, pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Vec<T>, AppError>
    where
        T: for<'row> sqlx::FromRow<'row, sqlx::postgres::PgRow> + Send + Unpin,
    {
        let kind = match self {
            Self::Id(_) => concat!(crate::charger_type_sql!("cs"), " AS charger_type"),
            Self::Completed { .. } => "'dc'::text AS charger_type",
        };
        let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
            "SELECT cs.id,cs.started_at,cs.ended_at,cs.soc_start,cs.soc_end,",
        );
        query
            .push(kind)
            .push(" FROM panda.charge_sessions cs WHERE cs.vehicle_id=")
            .push_bind(vehicle);
        match self {
            Self::Id(id) => {
                query.push(" AND cs.id=").push_bind(id);
            }
            Self::Completed { from, to } => {
                query
                    .push(" AND cs.started_at>=")
                    .push_bind(from)
                    .push(" AND cs.started_at<=")
                    .push_bind(to)
                    .push(concat!(
                        " AND cs.ended_at IS NOT NULL AND cs.is_home IS DISTINCT FROM true AND ",
                        crate::charger_type_sql!("cs"),
                        " = 'dc' ORDER BY cs.started_at,cs.id",
                    ));
            }
        }
        query
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(AppError::from)
    }
}

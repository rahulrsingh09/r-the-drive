//! Vehicle health endpoint — returns tire pressures, software version history,
//! closure states, and thermal event counts.

mod history;
mod observations;
use history::{fetch_sw_history, fetch_thermal_count};

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    db::{users::require_admin_or_super_user, vehicles::require_vehicle_read_access},
    errors::AppError,
    middleware::auth::{require_vehicle_access, AppState, AuthUser},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vehicles/{vehicle_id}/health", get(health))
        .route(
            "/admin/charge-session-repairs/{journal_id}/rollback",
            post(rollback_repair),
        )
}

async fn rollback_repair(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(journal_id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    crate::services::charge_session_repair::rollback_repair(&state.pool, journal_id)
        .await
        .map_err(|error| AppError::Conflict(error.to_string()))?;
    Ok(Json(
        serde_json::json!({ "journal_id": journal_id, "status": "reverted" }),
    ))
}

#[derive(Serialize)]
struct HealthResponse {
    vehicle_id: Uuid,
    vehicle: HealthVehicle,
    generated_at: DateTime<Utc>,
    runtime: Option<RuntimeHealth>,
    trip_capture: TripCaptureHealth,
    latest: Option<LatestHealthTelemetry>,
    tires: Option<TirePressures>,
    closures: Option<Closures>,
    current_software_version: Option<String>,
    ota_release_notes_url: Option<String>,
    software_history: Vec<SoftwareEntry>,
    thermal_events_30d: i64,
    extended_telemetry: ExtendedTelemetry,
}

#[derive(Serialize)]
struct ExtendedTelemetry {
    collector: Option<CollectorHealth>,
    parallax: Option<ParallaxHealth>,
    legacy_charging_session: LegacyChargingHealth,
    session_repair: Option<RepairHealth>,
    network: Option<NetworkHealth>,
    efficiency: Option<EfficiencyHealth>,
    mass: Option<MassHealth>,
    cold_weather: Option<ColdWeatherHealth>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ParallaxHealth {
    status: String,
    last_frame_at: Option<DateTime<Utc>>,
    last_meaningful_frame_at: Option<DateTime<Utc>>,
    reconnect_count: i64,
    decode_error_count: i64,
    empty_frame_count: i64,
    ambiguity_count: i64,
    last_error: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct LegacyChargingHealth {
    classification: String,
    last_frame_at: Option<DateTime<Utc>>,
    last_meaningful_frame_at: Option<DateTime<Utc>>,
    null_count: i64,
    missing_count: i64,
    malformed_count: i64,
    all_null_count: i64,
    meaningful_count: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct RepairHealth {
    repair_key: String,
    reason: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
struct CollectorHealth {
    status: String,
    running: bool,
    connected_at: Option<DateTime<Utc>>,
    last_event_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
struct NetworkHealth {
    source_at: DateTime<Utc>,
    wifi_connected: Option<bool>,
    wifi_rssi_dbm: Option<i32>,
    wifi_link_speed_mbps: Option<i32>,
    wifi_frequency_mhz: Option<i32>,
    wifi_channel_width_mhz: Option<i32>,
    cellular_access_technology: Option<String>,
    cellular_signal_dbm: Option<i32>,
}

#[derive(Serialize, sqlx::FromRow)]
struct EfficiencyHealth {
    source_at: DateTime<Utc>,
    reference_wh_per_km: Option<i32>,
    learned_wh_per_km: Option<i32>,
    mode_ranges_km: serde_json::Value,
}

#[derive(Serialize, sqlx::FromRow)]
struct MassHealth {
    source_at: DateTime<Utc>,
    estimated_mass_kg: i32,
}

#[derive(Serialize, sqlx::FromRow)]
struct ColdWeatherHealth {
    source_at: DateTime<Utc>,
    available_soc_pct: Option<i32>,
    cold_limited_soc_pct: Option<i32>,
    cold_range_impact_km: Option<f64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct HealthVehicle {
    name: Option<String>,
    model: String,
    trim: Option<String>,
    vin: Option<String>,
    ota_release_notes_url: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct RuntimeHealth {
    is_online: Option<bool>,
    last_event_at: Option<DateTime<Utc>>,
    worker_health: Option<String>,
    worker_health_msg: Option<String>,
    auth_state: Option<String>,
    auth_reason_code: Option<String>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct TripCaptureHealth {
    ready: bool,
    collector_heartbeat_at: Option<DateTime<Utc>>,
    checkpoint_at: Option<DateTime<Utc>>,
    pending_completions: i64,
    persistence_error: Option<String>,
}

pub(crate) async fn fetch_trip_capture_health(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
) -> Result<TripCaptureHealth, AppError> {
    let row = sqlx::query_as::<_, TripCaptureSnapshot>(
        r#"SELECT now() AS observed_at, r.collector_heartbeat_at, r.auth_state, r.worker_health,
                  c.vehicle_id IS NOT NULL AS checkpoint_present, c.updated_at AS checkpoint_at,
                  (SELECT count(*) FROM panda.pending_trip_completions p WHERE p.vehicle_id=v.id) AS pending_completions,
                  r.trip_persistence_error AS persistence_error
           FROM panda.vehicles v
           LEFT JOIN panda.vehicle_runtime_state r ON r.vehicle_id=v.id
           LEFT JOIN panda.active_trip_checkpoints c ON c.vehicle_id=v.id
           WHERE v.id=$1"#,
    ).bind(vehicle_id).fetch_one(pool).await?;
    Ok(row.project())
}

#[derive(Serialize, sqlx::FromRow)]
struct LatestHealthTelemetry {
    ts: DateTime<Utc>,
    twelve_volt_health: Option<String>,
    hv_thermal_event: Option<String>,
    ota_current_version: Option<String>,
    ota_available_version: Option<String>,
    ota_status: Option<String>,
    ota_current_status: Option<String>,
    is_online: Option<bool>,
}

#[derive(Serialize, sqlx::FromRow)]
struct TirePressures {
    ts: DateTime<Utc>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
    tire_fl_status: Option<String>,
    tire_fr_status: Option<String>,
    tire_rl_status: Option<String>,
    tire_rr_status: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct Closures {
    ts: DateTime<Utc>,
    closure_frunk_closed: Option<bool>,
    closure_liftgate_closed: Option<bool>,
    closure_tailgate_closed: Option<bool>,
    door_front_left_closed: Option<bool>,
    door_front_right_closed: Option<bool>,
    door_rear_left_closed: Option<bool>,
    door_rear_right_closed: Option<bool>,
}

#[derive(Serialize, sqlx::FromRow)]
struct SoftwareEntry {
    version: Option<String>,
    installed_at: DateTime<Utc>,
    observed_until: Option<DateTime<Utc>>,
}

async fn health(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<HealthResponse>, AppError> {
    require_vehicle_access(&auth, vehicle_id)?;
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;

    let (vehicle, runtime, observations, sw_history, thermal_count, extended) = tokio::try_join!(
        fetch_vehicle(&state.pool, vehicle_id),
        fetch_runtime(&state.pool, vehicle_id),
        observations::fetch(&state.pool, vehicle_id),
        fetch_sw_history(&state.pool, vehicle_id),
        fetch_thermal_count(&state.pool, vehicle_id),
        fetch_extended_telemetry(&state.pool, vehicle_id),
    )?;

    let observations::Observations {
        latest,
        tires,
        closures,
    } = observations;
    let ota_release_notes_url = vehicle.ota_release_notes_url.clone();
    let trip_capture = fetch_trip_capture_health(&state.pool, vehicle_id).await?;

    let current_version = sw_history
        .iter()
        .find(|e| e.observed_until.is_none() && e.version.is_some())
        .and_then(|e| e.version.clone())
        .or_else(|| latest.as_ref().and_then(|e| e.ota_current_version.clone()));

    Ok(Json(HealthResponse {
        vehicle_id,
        vehicle,
        generated_at: Utc::now(),
        runtime,
        trip_capture,
        latest,
        tires,
        closures,
        current_software_version: current_version,
        ota_release_notes_url,
        software_history: sw_history,
        thermal_events_30d: thermal_count,
        extended_telemetry: extended,
    }))
}

async fn fetch_extended_telemetry(
    pool: &sqlx::PgPool,
    vid: Uuid,
) -> Result<ExtendedTelemetry, AppError> {
    let (collector, session_repair, network, efficiency, mass, cold_weather) = tokio::try_join!(
        CollectorSnapshot::read(pool, vid),
        sqlx::query_as::<_, RepairHealth>(r#"SELECT repair_key,reason,created_at FROM panda.charge_session_repair_journal WHERE vehicle_id=$1 ORDER BY created_at DESC LIMIT 1"#).bind(vid).fetch_optional(pool),
        sqlx::query_as::<_, NetworkHealth>(r#"SELECT source_at, wifi_connected, wifi_rssi_dbm, wifi_link_speed_mbps,
                      wifi_frequency_mhz, wifi_channel_width_mhz,
                      cellular_access_technology, cellular_signal_dbm
               FROM timeseries.parallax_network_samples
               WHERE vehicle_id = $1 ORDER BY source_at DESC LIMIT 1"#).bind(vid).fetch_optional(pool),
        sqlx::query_as::<_, EfficiencyHealth>(r#"SELECT source_at, reference_wh_per_km, learned_wh_per_km, mode_ranges_km
               FROM timeseries.parallax_efficiency_samples
               WHERE vehicle_id = $1 ORDER BY source_at DESC LIMIT 1"#).bind(vid).fetch_optional(pool),
        sqlx::query_as::<_, MassHealth>(r#"SELECT source_at, estimated_mass_kg
               FROM timeseries.parallax_mass_samples
               WHERE vehicle_id = $1 ORDER BY source_at DESC LIMIT 1"#).bind(vid).fetch_optional(pool),
        sqlx::query_as::<_, ColdWeatherHealth>(r#"SELECT source_at, available_soc_pct, cold_limited_soc_pct, cold_range_impact_km
               FROM timeseries.parallax_cold_weather_samples
               WHERE vehicle_id = $1
                 AND (cold_limited_soc_pct IS NOT NULL OR cold_range_impact_km IS NOT NULL)
               ORDER BY source_at DESC LIMIT 1"#).bind(vid).fetch_optional(pool),
    )?;
    let (collector, parallax, legacy_charging_session) = match collector {
        Some(snapshot) => snapshot.project(),
        None => (None, None, LegacyChargingHealth::unobserved()),
    };
    Ok(ExtendedTelemetry {
        collector,
        parallax,
        legacy_charging_session,
        session_repair,
        network,
        efficiency,
        mass,
        cold_weather,
    })
}

async fn fetch_vehicle(pool: &sqlx::PgPool, vid: Uuid) -> Result<HealthVehicle, AppError> {
    let row = sqlx::query_as::<_, HealthVehicle>(
        r#"SELECT name, model, trim, vin
                  , ota_release_notes_url
           FROM panda.vehicles
           WHERE id = $1"#,
    )
    .bind(vid)
    .fetch_one(pool)
    .await
    .map_err(AppError::from)?;
    Ok(row)
}

async fn fetch_runtime(pool: &sqlx::PgPool, vid: Uuid) -> Result<Option<RuntimeHealth>, AppError> {
    let row = sqlx::query_as::<_, RuntimeHealth>(
        r#"SELECT is_online, last_event_at, worker_health, worker_health_msg, auth_state, auth_reason_code, updated_at
           FROM panda.vehicle_runtime_state
           WHERE vehicle_id = $1"#,
    )
    .bind(vid)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;
    Ok(row)
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    use crate::authorization_test_support::Fixture;
    use serde_json::json;

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn health_complete_projection_characterization() {
        let f = Fixture::new().await;
        let owner = f.user("user").await;
        let vehicle = f.vehicle(owner, "demo-health-projection").await;
        let mut observations = vec![serde_json::to_value(fetch_extended_telemetry(&f.state.pool, vehicle).await.unwrap()).unwrap()];
        assert_eq!(observations[0]["legacy_charging_session"]["classification"], "not_observed");
        sqlx::query("INSERT INTO panda.parallax_collector_state(vehicle_id,status,owner_kind,connected_at,last_event_at,last_error,updated_at,last_frame_at,last_meaningful_frame_at,reconnect_count,decode_error_count,empty_frame_count,ambiguity_count,legacy_last_classification,legacy_null_count,legacy_missing_count,legacy_malformed_count,legacy_all_null_count,legacy_meaningful_count) VALUES($1,'connected','in_process','2026-01-01Z','2026-01-02Z','synthetic error','2026-01-03Z','2026-01-04Z','2026-01-05Z',1,2,3,4,'meaningful',5,6,7,8,9)")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.parallax_network_samples(vehicle_id,source_at,received_at,payload_hash,wifi_connected,wifi_rssi_dbm,wifi_link_speed_mbps,wifi_frequency_mhz,wifi_channel_width_mhz,cellular_access_technology,cellular_signal_dbm) VALUES($1,'2026-01-01Z','2026-01-01Z',decode('aa','hex'),true,-50,42,2400,20,'fixture',-80)")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.parallax_efficiency_samples(vehicle_id,source_at,received_at,payload_hash,reference_wh_per_km,learned_wh_per_km,mode_ranges_km) VALUES($1,'2026-01-01Z','2026-01-01Z',decode('ab','hex'),200,210,'{\"all_purpose\":300}')")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.parallax_mass_samples(vehicle_id,source_at,received_at,payload_hash,estimated_mass_kg) VALUES($1,'2026-01-01Z','2026-01-01Z',decode('ac','hex'),3000)")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        sqlx::query("INSERT INTO timeseries.parallax_cold_weather_samples(vehicle_id,source_at,received_at,payload_hash,available_soc_pct,cold_limited_soc_pct,cold_range_impact_km) VALUES($1,'2026-01-01Z','2026-01-01Z',decode('ad','hex'),80,10,20.5)")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        let stale = serde_json::to_value(fetch_extended_telemetry(&f.state.pool, vehicle).await.unwrap()).unwrap();
        assert_eq!(stale["collector"]["status"], "stale");
        assert_eq!(stale["collector"]["running"], false);
        assert_eq!(stale["parallax"]["reconnect_count"], 1);
        assert_eq!(stale["legacy_charging_session"]["meaningful_count"], 9);
        observations.push(stale);
        sqlx::query("UPDATE panda.parallax_collector_state SET updated_at='2099-01-01Z' WHERE vehicle_id=$1")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        let active = serde_json::to_value(fetch_extended_telemetry(&f.state.pool, vehicle).await.unwrap()).unwrap();
        assert_eq!(active["collector"]["status"], "connected");
        assert_eq!(active["collector"]["running"], true);
        observations.push(active);
        sqlx::query("UPDATE panda.parallax_collector_state SET owner_kind=NULL,status='disconnected' WHERE vehicle_id=$1")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        let duplicate = serde_json::to_value(fetch_extended_telemetry(&f.state.pool, vehicle).await.unwrap()).unwrap();
        assert_eq!(duplicate["collector"]["status"], "duplicate_owner");
        assert_eq!(duplicate["parallax"]["status"], "disconnected");
        observations.push(duplicate);
        sqlx::query("ALTER TABLE timeseries.parallax_network_samples RENAME TO parallax_network_samples_fault").execute(&f.state.pool).await.unwrap();
        assert!(matches!(fetch_extended_telemetry(&f.state.pool, vehicle).await, Err(AppError::Database(_))));
        sqlx::query("ALTER TABLE timeseries.parallax_network_samples_fault RENAME TO parallax_network_samples").execute(&f.state.pool).await.unwrap();
        assert_eq!(serde_json::to_value(fetch_extended_telemetry(&f.state.pool, vehicle).await.unwrap()).unwrap(), observations[3]);
        let evidence = json!(observations);
        if let Ok(path) = std::env::var("HEALTH_CHARACTERIZATION_CAPTURE") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
        if let Ok(path) = std::env::var("HEALTH_CHARACTERIZATION_COMPARE") {
            let original: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(evidence, original);
        }
        f.cleanup().await;
    }
}

#[derive(sqlx::FromRow)]
struct CollectorSnapshot {
    collector_status: String,
    running: bool,
    parallax_status: String,
    connected_at: Option<DateTime<Utc>>,
    last_event_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    updated_at: DateTime<Utc>,
    last_frame_at: Option<DateTime<Utc>>,
    last_meaningful_frame_at: Option<DateTime<Utc>>,
    reconnect_count: i64,
    decode_error_count: i64,
    empty_frame_count: i64,
    ambiguity_count: i64,
    classification: String,
    legacy_last_frame_at: Option<DateTime<Utc>>,
    legacy_last_meaningful_frame_at: Option<DateTime<Utc>>,
    null_count: i64,
    missing_count: i64,
    malformed_count: i64,
    all_null_count: i64,
    meaningful_count: i64,
}
impl CollectorSnapshot {
    async fn read(pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as(r#"SELECT
            CASE WHEN owner_kind IS NULL AND updated_at >= now()-interval '2 minutes' THEN 'duplicate_owner'
                 WHEN status='connected' AND updated_at < now()-interval '2 minutes' THEN 'stale'
                 ELSE status END AS collector_status,
            status='connected' AND owner_kind='in_process' AND updated_at >= now()-interval '2 minutes' AS running,
            CASE WHEN status='connected' AND updated_at < now()-interval '2 minutes' THEN 'stale' ELSE status END AS parallax_status,
            connected_at,last_event_at,last_error,updated_at,last_frame_at,last_meaningful_frame_at,
            reconnect_count,decode_error_count,empty_frame_count,ambiguity_count,
            COALESCE(legacy_last_classification,'not_observed') AS classification,
            legacy_last_frame_at,legacy_last_meaningful_frame_at,
            legacy_null_count AS null_count,legacy_missing_count AS missing_count,
            legacy_malformed_count AS malformed_count,legacy_all_null_count AS all_null_count,legacy_meaningful_count AS meaningful_count
            FROM panda.parallax_collector_state WHERE vehicle_id=$1"#).bind(vehicle).fetch_optional(pool).await
    }
    fn project(
        self,
    ) -> (
        Option<CollectorHealth>,
        Option<ParallaxHealth>,
        LegacyChargingHealth,
    ) {
        let Self {
            collector_status,
            running,
            parallax_status,
            connected_at,
            last_event_at,
            last_error,
            updated_at,
            last_frame_at,
            last_meaningful_frame_at,
            reconnect_count,
            decode_error_count,
            empty_frame_count,
            ambiguity_count,
            classification,
            legacy_last_frame_at,
            legacy_last_meaningful_frame_at,
            null_count,
            missing_count,
            malformed_count,
            all_null_count,
            meaningful_count,
        } = self;
        let parallax = ParallaxHealth {
            status: parallax_status,
            last_frame_at,
            last_meaningful_frame_at,
            reconnect_count,
            decode_error_count,
            empty_frame_count,
            ambiguity_count,
            last_error: last_error.clone(),
        };
        let collector = CollectorHealth {
            status: collector_status,
            running,
            connected_at,
            last_event_at,
            last_error,
            updated_at,
        };
        let legacy = LegacyChargingHealth {
            classification,
            last_frame_at: legacy_last_frame_at,
            last_meaningful_frame_at: legacy_last_meaningful_frame_at,
            null_count,
            missing_count,
            malformed_count,
            all_null_count,
            meaningful_count,
        };
        (Some(collector), Some(parallax), legacy)
    }
}
impl LegacyChargingHealth {
    fn unobserved() -> Self {
        Self {
            classification: "not_observed".into(),
            last_frame_at: None,
            last_meaningful_frame_at: None,
            null_count: 0,
            missing_count: 0,
            malformed_count: 0,
            all_null_count: 0,
            meaningful_count: 0,
        }
    }
}

#[cfg(test)]
mod readiness_characterization {
    use super::*;
    use crate::authorization_test_support::Fixture;
    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn trip_readiness_predicate_characterization() {
        let f=Fixture::new().await;
        let owner=f.user("user").await;
        let vehicle=f.vehicle(owner,"demo-readiness-characterization").await;
        let read=|pool: sqlx::PgPool| async move {serde_json::to_value(fetch_trip_capture_health(&pool,vehicle).await.unwrap()).unwrap()};
        let empty=read(f.state.pool.clone()).await;
        assert_eq!(empty,serde_json::json!({"ready":false,"collector_heartbeat_at":null,"checkpoint_at":null,"pending_completions":0,"persistence_error":null}));
        sqlx::query("INSERT INTO panda.active_trip_checkpoints(vehicle_id,snapshot,updated_at) VALUES($1,'{}','2026-01-01Z')").bind(vehicle).execute(&f.state.pool).await.unwrap();
        sqlx::query("INSERT INTO panda.vehicle_runtime_state(vehicle_id,collector_heartbeat_at,auth_state,worker_health,updated_at) VALUES($1,'2099-01-01Z','authorized','connected','2026-01-01Z') ON CONFLICT (vehicle_id) DO UPDATE SET collector_heartbeat_at=EXCLUDED.collector_heartbeat_at,auth_state=EXCLUDED.auth_state,worker_health=EXCLUDED.worker_health,updated_at=EXCLUDED.updated_at").bind(vehicle).execute(&f.state.pool).await.unwrap();
        let healthy=read(f.state.pool.clone()).await;
        assert_eq!(healthy,serde_json::json!({"ready":true,"collector_heartbeat_at":"2099-01-01T00:00:00Z","checkpoint_at":"2026-01-01T00:00:00Z","pending_completions":0,"persistence_error":null}));
        for assignments in ["collector_heartbeat_at='2026-01-01Z'", "collector_heartbeat_at=NULL", "auth_state='expired'", "auth_state=NULL", "worker_health='starting'", "trip_persistence_error='synthetic persistence failure'"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE panda.vehicle_runtime_state SET {assignments} WHERE vehicle_id=$1"))).bind(vehicle).execute(&f.state.pool).await.unwrap();
            assert_eq!(read(f.state.pool.clone()).await["ready"],false,"{assignments}");
            sqlx::query("UPDATE panda.vehicle_runtime_state SET collector_heartbeat_at='2099-01-01Z',auth_state='authorized',worker_health='connected',trip_persistence_error=NULL WHERE vehicle_id=$1").bind(vehicle).execute(&f.state.pool).await.unwrap();
        }
        sqlx::query("INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip) VALUES($1,$2,'{}')").bind(Uuid::new_v4()).bind(vehicle).execute(&f.state.pool).await.unwrap();
        let pending=read(f.state.pool.clone()).await;
        assert_eq!(pending["ready"],false);assert_eq!(pending["pending_completions"],1);
        assert!(matches!(fetch_trip_capture_health(&f.state.pool,Uuid::new_v4()).await,Err(AppError::Database(sqlx::Error::RowNotFound))));
        let mut cleanup_url=url::Url::parse(&f.state.config.database_url).unwrap();
        let database=cleanup_url.path().trim_start_matches('/').to_owned();
        assert!(database.starts_with("authorization_test_"));
        cleanup_url.set_path("/postgres");
        f.state.pool.close().await;
        let probe=sqlx::PgPool::connect(cleanup_url.as_str()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5),async {
            loop {
                let connections:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=$1 AND backend_type='client backend'").bind(&database).fetch_one(&probe).await.unwrap();
                if connections==0 {break;}
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }).await.expect("fixture clients closed");
        probe.close().await;
        f.cleanup().await;
    }
}

#[derive(sqlx::FromRow)]
struct TripCaptureSnapshot {
    observed_at: DateTime<Utc>,
    collector_heartbeat_at: Option<DateTime<Utc>>,
    auth_state: Option<String>,
    worker_health: Option<String>,
    checkpoint_present: bool,
    checkpoint_at: Option<DateTime<Utc>>,
    pending_completions: i64,
    persistence_error: Option<String>,
}
impl TripCaptureSnapshot {
    fn project(self) -> TripCaptureHealth {
        let current = self
            .collector_heartbeat_at
            .is_some_and(|at| at >= self.observed_at - chrono::Duration::seconds(60));
        TripCaptureHealth {
            ready: current
                && self.auth_state.as_deref() == Some("authorized")
                && self.worker_health.as_deref() == Some("connected")
                && self.persistence_error.is_none()
                && self.checkpoint_present
                && self.pending_completions == 0,
            collector_heartbeat_at: self.collector_heartbeat_at,
            checkpoint_at: self.checkpoint_at,
            pending_completions: self.pending_completions,
            persistence_error: self.persistence_error,
        }
    }
}
#[cfg(test)]
mod readiness_boundary_tests {
    use super::*;
    #[test]
    fn trip_readiness_inclusive_sixty_second_boundary() {
        let observed_at: DateTime<Utc> = "2026-01-01T00:01:00Z".parse().unwrap();
        for (age, ready) in [
            (chrono::Duration::seconds(60), true),
            (
                chrono::Duration::seconds(60) + chrono::Duration::microseconds(1),
                false,
            ),
        ] {
            let row = TripCaptureSnapshot {
                observed_at,
                collector_heartbeat_at: Some(observed_at - age),
                auth_state: Some("authorized".into()),
                worker_health: Some("connected".into()),
                checkpoint_present: true,
                checkpoint_at: Some(observed_at),
                pending_completions: 0,
                persistence_error: None,
            };
            assert_eq!(row.project().ready, ready);
        }
    }
}

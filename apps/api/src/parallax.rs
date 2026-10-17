//! Default, isolated in-process Parallax telemetry companion.
//!
//! This module deliberately does not call or modify the canonical
//! `ingestion::ws_client` flow. It opens its own allowlisted subscription and
//! persists only typed, privacy-filtered values.

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use age::x25519::Identity;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, TimeZone, Utc};
use futures::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use uuid::Uuid;

use crate::ingestion::session_store::{decrypt_tokens, RivianTokenBundle};
use crate::models::telemetry::{ClosureTransition, PowerState, TelemetryEvent};
use crate::services::ingestion_capture::{self, Kind as CaptureKind};

#[path = "parallax/forwarding.rs"]
mod forwarding;
#[path = "parallax/transport.rs"]
mod transport;
use transport::{RetrySchedule, SocketAction};
#[path = "parallax/projection.rs"]
mod projection;
#[path = "parallax/snapshot.rs"]
mod snapshot;
use snapshot::Snapshot;

const WS_URL: &str = "wss://api.rivian.com/gql-consumer-subscriptions/graphql";
const SUBSCRIPTION_ID: &str = "panda-parallax-collector";
const SCHEMA_VERSION: i32 = 1;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
static TELEMETRY_FORWARD_DROP_LOG_COUNT: AtomicU64 = AtomicU64::new(0);
const VEHICLE_STATE_TOPICS: &[&str] = &[
    "vehicle.network.state",
    "dynamics.vehicle.efficiency",
    "dynamics.vehicle.mass_estimate",
    "dynamics.vehicle.drive_mode",
    "energy_edge_compute.graphs.parked_energy_distributions",
    "energy_edge_compute.graphs.charge_session_breakdown",
    "energy.high_voltage.battery_state",
    "energy_edge_compute.graphs.charging_graph_global",
    "charging.session.time_estimation",
    "charging.session.status",
    "energy_edge_compute.graphs.cold_weather_soc",
    "vehicle.power.state",
    "dynamics.vehicle.gnss",
    "dynamics.vehicle.odometer",
    "dynamics.vehicle.gear",
    "body.closures.states",
    "body.locks.states",
    "dynamics.tires.state",
    "comfort.cabin.cabin_temperatures",
    "comfort.cabin.cabin_preconditioning_status",
    "comfort.cabin.defrost_defog_status",
];

/// The subscription allowlist is also the decoder/persistence boundary.
/// Keeping the list in one place prevents an unsolicited topic from becoming
/// a raw-payload write path.
fn is_allowlisted_topic(topic: &str) -> bool {
    VEHICLE_STATE_TOPICS.contains(&topic)
}

/// Start the isolated Parallax companion for one vehicle.  The returned task
/// owns its socket and reconnect loop; it never receives the canonical
/// telemetry channel, so backpressure or schema failures cannot delay it.
/// `active_sessions` is deliberately a watch channel: only the latest
/// canonical lifecycle context is relevant to enrichment consumers.
#[allow(clippy::too_many_arguments)] // The owner, credentials, and lifecycle channels are independent inputs.
pub fn spawn_in_process(
    pool: PgPool,
    vehicle_id: Uuid,
    rivian_vehicle_id: String,
    age_key: String,
    telemetry_tx: mpsc::Sender<(String, TelemetryEvent)>,
    mut active_sessions: watch::Receiver<crate::ingestion::worker::ActiveSessionContext>,
    mut shutdown: broadcast::Receiver<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        enum Step {
            Lease,
            Credentials,
            Connected(CollectorSession),
            Wait(Duration, bool),
        }
        let owner = Uuid::new_v4();
        let mut owned = false;
        let mut retry = RetrySchedule::default();
        let mut step = Step::Lease;
        loop {
            step = match step {
                Step::Lease => match acquire_in_process_lease(&pool, vehicle_id, owner).await {
                    Ok(true) => {
                        owned = true;
                        Step::Credentials
                    }
                    result => {
                        match result {
                            Ok(false) => {
                                tracing::warn!(%vehicle_id, "fresh standalone Parallax owner detected; waiting for upgrade overlap to clear")
                            }
                            Err(error) => {
                                tracing::warn!(%vehicle_id, err=%error, "Parallax lease acquisition failed")
                            }
                            _ => unreachable!(),
                        }
                        Step::Wait(Duration::from_secs(30), false)
                    }
                },
                Step::Credentials => match crate::ingestion::rivian_poll::load_vehicle_tokens(
                    vehicle_id, &pool, &age_key,
                )
                .await
                {
                    Ok((_, tokens)) => Step::Connected(CollectorSession {
                        vehicle_id,
                        rivian_vehicle_id: rivian_vehicle_id.clone(),
                        tokens,
                    }),
                    Err(error) => {
                        let _ = set_collector_state(
                            &pool,
                            vehicle_id,
                            "error",
                            Some(&error.to_string()),
                        )
                        .await;
                        Step::Wait(retry.delay(), true)
                    }
                },
                Step::Connected(session) => {
                    let result = tokio::select! {
                        _ = shutdown.recv() => {
                            let _ = set_collector_state(&pool, vehicle_id, "disconnected", Some("shutdown")).await;
                            break;
                        }
                        result = collect_connection_with_context(&pool, &session, &mut active_sessions, Some(&telemetry_tx)) => result,
                    };
                    match result {
                        Ok(()) => {
                            retry.reset();
                            Step::Credentials
                        }
                        Err(_) if shutdown.try_recv().is_ok() => break,
                        Err(error) => {
                            let _ = AcquisitionWrite::new("panda.parallax_collector_state")
                                .field(
                                    "reconnect_count",
                                    Stored::Sql("reconnect_count + 1"),
                                    MergeWrite::Replace,
                                )
                                .update(&pool, &[("vehicle_id", vehicle_id)], "")
                                .await;
                            let _ = set_collector_state(
                                &pool,
                                vehicle_id,
                                "error",
                                Some(&error.to_string()),
                            )
                            .await;
                            Step::Wait(retry.delay(), true)
                        }
                    }
                }
                Step::Wait(delay, advance) => {
                    if !transport::wait_to_retry(&mut shutdown, delay).await {
                        break;
                    }
                    if advance {
                        retry.advance();
                    }
                    if owned {
                        Step::Credentials
                    } else {
                        Step::Lease
                    }
                }
            };
        }
        if owned {
            let _ = release_in_process_lease(&pool, vehicle_id, owner).await;
        }
    })
}

async fn acquire_in_process_lease(pool: &PgPool, vehicle_id: Uuid, owner_id: Uuid) -> Result<bool> {
    let changed = AcquisitionWrite::new("panda.parallax_collector_state")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field("status", Stored::Text(Some("starting")), MergeWrite::Replace)
        .field("schema_version", Stored::Int(Some(SCHEMA_VERSION)), MergeWrite::Keep)
        .field("owner_kind", Stored::Text(Some("in_process")), MergeWrite::Replace)
        .field("owner_instance_id", Stored::Id(owner_id), MergeWrite::Replace)
        .field("last_error", Stored::Text(None), MergeWrite::Replace)
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .upsert(pool, "(vehicle_id)", "panda.parallax_collector_state.owner_instance_id = EXCLUDED.owner_instance_id OR panda.parallax_collector_state.updated_at < now() - interval '2 minutes' OR panda.parallax_collector_state.status = 'disconnected'").await?;
    Ok(changed != 0)
}

pub(crate) async fn release_in_process_lease(
    pool: &PgPool,
    vehicle_id: Uuid,
    owner_id: Uuid,
) -> Result<()> {
    AcquisitionWrite::new("panda.parallax_collector_state")
        .field(
            "status",
            Stored::Text(Some("disconnected")),
            MergeWrite::Replace,
        )
        .field(
            "owner_instance_id",
            Stored::Sql("NULL"),
            MergeWrite::Replace,
        )
        .field(
            "last_error",
            Stored::Text(Some("shutdown")),
            MergeWrite::Replace,
        )
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .update(
            pool,
            &[("vehicle_id", vehicle_id), ("owner_instance_id", owner_id)],
            "owner_kind = 'in_process'",
        )
        .await?;
    Ok(())
}

pub(crate) async fn update_parallax_power(
    pool: &PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    power_kw: f64,
    observed_at: DateTime<Utc>,
) -> Result<u64> {
    Ok(sqlx::query("UPDATE panda.charge_sessions SET parallax_live_power_kw=$1,parallax_power_observed_at=$4 WHERE id=$2 AND vehicle_id=$3 AND ended_at IS NULL AND (parallax_power_observed_at IS NULL OR $4>=parallax_power_observed_at)")
        .bind(power_kw).bind(session_id).bind(vehicle_id).bind(observed_at).execute(pool).await?.rows_affected())
}

#[derive(Debug)]
struct CollectorSession {
    vehicle_id: Uuid,
    rivian_vehicle_id: String,
    tokens: RivianTokenBundle,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct NetworkState {
    #[prost(int32, optional, tag = "1")]
    overall_state: Option<i32>,
    #[prost(int32, optional, tag = "3")]
    active_transport: Option<i32>,
    #[prost(message, optional, tag = "4")]
    wifi: Option<WifiState>,
    #[prost(message, optional, tag = "5")]
    cellular: Option<CellularState>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct WifiState {
    #[prost(int32, optional, tag = "1")]
    status: Option<i32>,
    // Tags 2 and 3 intentionally omitted: they can contain network identity.
    #[prost(int32, optional, tag = "7")]
    connection_state: Option<i32>,
    #[prost(int32, optional, tag = "8")]
    rssi_dbm: Option<i32>,
    #[prost(int32, optional, tag = "9")]
    link_speed_mbps: Option<i32>,
    #[prost(int32, optional, tag = "10")]
    frequency_mhz: Option<i32>,
    #[prost(int32, optional, tag = "11")]
    channel_width_mhz: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct CellularState {
    // Tag 1 (carrier name) is intentionally omitted.
    #[prost(string, optional, tag = "2")]
    access_technology: Option<String>,
    #[prost(int32, optional, tag = "4")]
    signal_dbm: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct EfficiencyState {
    #[prost(int32, optional, tag = "1")]
    reference_wh_per_km: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    learned_wh_per_km: Option<i32>,
    #[prost(message, repeated, tag = "3")]
    mode_ranges: Vec<ModeRange>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ModeRange {
    #[prost(int32, optional, tag = "1")]
    mode: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    full_charge_range_km: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct MassEstimate {
    #[prost(int32, optional, tag = "1")]
    estimated_mass_kg: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ParkedEnergyDistributions {
    #[prost(message, optional, tag = "1")]
    hours_24: Option<ParkedEnergyWindow>,
    #[prost(message, optional, tag = "2")]
    hours_8: Option<ParkedEnergyWindow>,
    #[prost(message, optional, tag = "3")]
    since_parked: Option<ParkedEnergyWindow>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ParkedEnergyWindow {
    #[prost(float, optional, tag = "1")]
    total_kwh: Option<f32>,
    #[prost(float, optional, tag = "2")]
    vehicle_systems_kwh: Option<f32>,
    #[prost(float, optional, tag = "3")]
    outlets_kwh: Option<f32>,
    #[prost(float, optional, tag = "4")]
    climate_kwh: Option<f32>,
    #[prost(float, optional, tag = "5")]
    gear_guard_kwh: Option<f32>,
    #[prost(float, optional, tag = "6")]
    total_range_impact_km: Option<f32>,
    #[prost(float, optional, tag = "7")]
    vehicle_systems_range_impact_km: Option<f32>,
    #[prost(float, optional, tag = "8")]
    outlets_range_impact_km: Option<f32>,
    #[prost(float, optional, tag = "9")]
    climate_range_impact_km: Option<f32>,
    #[prost(float, optional, tag = "10")]
    gear_guard_range_impact_km: Option<f32>,
    #[prost(int32, optional, tag = "11")]
    duration_minutes: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ChargeBreakdown {
    #[prost(float, optional, tag = "1")]
    total_kwh: Option<f32>,
    // Tag 11 (cost display text) is intentionally omitted.
    #[prost(float, optional, tag = "9")]
    current_power_kw: Option<f32>,
    #[prost(int32, optional, tag = "10")]
    fallback_power_kw: Option<i32>,
    #[prost(int32, optional, tag = "13")]
    charging_state: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct HvBatteryState {
    #[prost(message, optional, tag = "1")]
    charge_state: Option<HvChargeState>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct HvChargeState {
    #[prost(double, optional, tag = "1")]
    soc: Option<f64>,
    #[prost(double, optional, tag = "2")]
    pack_energy_kwh: Option<f64>,
    #[prost(float, optional, tag = "3")]
    range_km: Option<f32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct PowerStateMessage {
    #[prost(int32, optional, tag = "1")]
    state: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct GnssState {
    #[prost(double, optional, tag = "1")]
    latitude: Option<f64>,
    #[prost(double, optional, tag = "2")]
    longitude: Option<f64>,
    #[prost(double, optional, tag = "3")]
    altitude_m: Option<f64>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct GearState {
    #[prost(int32, optional, tag = "1")]
    gear: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct OdometerState {
    #[prost(uint64, optional, tag = "1")]
    kilometers: Option<u64>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ClosureStates {
    #[prost(message, repeated, tag = "1")]
    states: Vec<ClosureState>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ClosureState {
    #[prost(int32, optional, tag = "1")]
    position: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    state: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct LockStates {
    #[prost(message, repeated, tag = "1")]
    states: Vec<LockState>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct LockState {
    #[prost(int32, optional, tag = "1")]
    position: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    state: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct TireStates {
    #[prost(message, repeated, tag = "2")]
    states: Vec<TireState>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct TireState {
    #[prost(int32, optional, tag = "1")]
    position: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    status: Option<i32>,
    #[prost(double, optional, tag = "3")]
    pressure_bar: Option<f64>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct CabinTemperatures {
    #[prost(float, optional, tag = "3")]
    cabin_c: Option<f32>,
    #[prost(float, optional, tag = "4")]
    driver_c: Option<f32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct PreconditioningState {
    #[prost(int32, optional, tag = "1")]
    status: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct DefrostState {
    #[prost(int32, optional, tag = "1")]
    status: Option<i32>,
}

/// R2 closure position with no canonical field (the rear drop glass).
const CLOSURE_REAR_GLASS_POSITION: i32 = 16;
/// The charge port door. Matched against legacy chargePortState on an R1S:
/// opening, open, closing, and closed changed at the same moments.
const CLOSURE_CHARGE_PORT_POSITION: i32 = 10;
/// Stateless entry that ends every R2 closure frame.
const CLOSURE_SENTINEL_POSITION: i32 = 10000;

#[cfg(test)]
pub(crate) fn decode_vehicle_telemetry(
    topic: &str,
    payload: &[u8],
    source_at: DateTime<Utc>,
    vehicle_id: Uuid,
) -> Result<Option<TelemetryEvent>> {
    decode_vehicle_telemetry_with_notes(topic, payload, source_at, vehicle_id, &mut Vec::new())
}

/// Decode one allowlisted Parallax RVM into the canonical partial event.
///
/// This is deliberately pure: callers decide how to merge and persist the
/// partial event. Unknown RVMs return `Ok(None)`, while malformed payloads or
/// values outside the documented physical/enum ranges are rejected. Entries
/// skipped for an unexpected reason while the rest of the frame still decodes
/// are described in `notes` so callers can surface them through ingestion
/// diagnostics.
pub(crate) fn decode_vehicle_telemetry_with_notes(
    topic: &str,
    payload: &[u8],
    source_at: DateTime<Utc>,
    vehicle_id: Uuid,
    notes: &mut Vec<String>,
) -> Result<Option<TelemetryEvent>> {
    let Some(reading) = projection::CanonicalReading::decode(topic, payload)? else {
        return Ok(None);
    };
    reading.project(payload.is_empty(), source_at, vehicle_id, notes)
}

fn valid_temperature(value: f64) -> Result<f64> {
    if value.is_finite() && (-80.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        anyhow::bail!("invalid cabin temperature")
    }
}

/// Canonical field for a closure position, or `None` when the position has no
/// canonical field (including 16, the R2 rear drop glass).
fn closure_field(event: &mut TelemetryEvent, position: i32) -> Option<&mut Option<bool>> {
    CLOSURE_FIELDS
        .iter()
        .find(|field| field.position == position)
        .map(|field| (field.access)(event))
}

/// Name of the canonical field a closure position maps to, for captures.
/// Must agree with `closure_field`.
fn closure_field_name(position: i32) -> Option<&'static str> {
    if position == CLOSURE_CHARGE_PORT_POSITION {
        return Some("charge_port_open");
    }
    CLOSURE_FIELDS
        .iter()
        .find(|field| field.position == position)
        .map(|field| field.name)
}

/// Name of the canonical field a lock position maps to, for captures.
/// Must agree with `set_lock`.
fn lock_field_name(position: i32) -> Option<&'static str> {
    LOCK_FIELDS
        .iter()
        .find(|field| field.position == position)
        .map(|field| field.name)
}

fn set_lock(event: &mut TelemetryEvent, position: i32, locked: bool) -> Result<bool> {
    let field = LOCK_FIELDS
        .iter()
        .find(|field| field.position == position)
        .with_context(|| format!("unknown lock position {position}"))?;
    *(field.access)(event) = Some(locked);
    Ok(true)
}

fn set_tire(
    event: &mut TelemetryEvent,
    position: i32,
    pressure_bar: f64,
    ok: bool,
) -> Result<bool> {
    let (pressure, status, valid) = match position {
        1 => (
            &mut event.tire_fl_psi,
            &mut event.tire_fl_status,
            &mut event.tire_fl_valid,
        ),
        2 => (
            &mut event.tire_fr_psi,
            &mut event.tire_fr_status,
            &mut event.tire_fr_valid,
        ),
        3 => (
            &mut event.tire_rl_psi,
            &mut event.tire_rl_status,
            &mut event.tire_rl_valid,
        ),
        4 => (
            &mut event.tire_rr_psi,
            &mut event.tire_rr_status,
            &mut event.tire_rr_valid,
        ),
        other => anyhow::bail!("unknown tire position {other}"),
    };
    *pressure = Some(pressure_bar * 14.503_773_8);
    *status = Some(if ok { "OK" } else { "Warning" }.to_owned());
    *valid = Some(true);
    Ok(true)
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ChargingGraphGlobal {
    #[prost(message, repeated, tag = "1")]
    segments: Vec<ChargingGraphSegment>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ChargingGraphSegment {
    #[prost(int32, optional, tag = "1")]
    soc: Option<i32>,
    #[prost(float, optional, tag = "2")]
    power_kw: Option<f32>,
    #[prost(int64, optional, tag = "3")]
    start_unix_ms: Option<i64>,
    #[prost(int64, optional, tag = "4")]
    end_unix_ms: Option<i64>,
    #[prost(int32, optional, tag = "6")]
    state: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ChargingTimeEstimation {
    #[prost(int32, optional, tag = "1")]
    remaining_seconds: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ChargingStatus {
    #[prost(int32, optional, tag = "1")]
    plug_connection_status: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    display_status: Option<i32>,
    #[prost(int32, optional, tag = "3")]
    evse_type: Option<i32>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct ColdWeatherSoc {
    #[prost(int32, optional, tag = "1")]
    available_soc_pct: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    cold_limited_soc_pct: Option<i32>,
    #[prost(float, optional, tag = "3")]
    cold_range_impact_km: Option<f32>,
}

pub async fn run(database_url: &str) -> Result<()> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(database_url)
        .await
        .context("connect Parallax collector to Panda database")?;
    crate::db::migrations::run_current_migrations(&pool)
        .await
        .context("apply Parallax telemetry migrations")?;
    let sessions = load_sessions(&pool).await?;
    anyhow::ensure!(
        !sessions.is_empty(),
        "no enrolled vehicle credentials were found"
    );
    let mut tasks = tokio::task::JoinSet::new();
    sessions.into_iter().for_each(|session| {
        tasks.spawn(run_vehicle(pool.clone(), session));
    });
    let completed = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Parallax collector shutdown requested");
            None
        }
        result = tasks.join_next() => result,
    };
    tasks.abort_all();
    completed.transpose()?.unwrap_or(Ok(()))
}

async fn load_sessions(pool: &PgPool) -> Result<Vec<CollectorSession>> {
    let rows = sqlx::query_as::<_, (Uuid, String, Vec<u8>, String)>(
        "SELECT v.id, v.rivian_vehicle_id, c.encrypted_tokens, \
         (SELECT value FROM panda.system_config WHERE key = 'age_key') \
         FROM panda.vehicles v JOIN panda.vehicle_credentials c ON c.vehicle_id = v.id \
         WHERE v.rivian_vehicle_id IS NOT NULL ORDER BY v.display_priority, v.created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut sessions = Vec::with_capacity(rows.len());
    let mut identities = std::collections::HashMap::new();
    for (vehicle_id, rivian_vehicle_id, encrypted, key) in rows {
        let identity = match identities.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let identity = entry
                    .key()
                    .parse::<Identity>()
                    .map_err(|_| anyhow::anyhow!("database Age identity is invalid"))?;
                entry.insert(identity)
            }
        };
        let tokens = decrypt_tokens(&encrypted, identity)?;
        tokens.validate()?;
        sessions.push(CollectorSession {
            vehicle_id,
            rivian_vehicle_id,
            tokens,
        });
    }
    Ok(sessions)
}

async fn run_vehicle(pool: PgPool, session: CollectorSession) -> Result<()> {
    enum Step {
        Connect,
        Delay(Duration),
    }
    let mut retry = RetrySchedule::default();
    let mut step = Step::Connect;
    loop {
        step = match step {
            Step::Delay(duration) => {
                tokio::time::sleep(duration).await;
                retry.advance();
                Step::Connect
            }
            Step::Connect => match collect_connection(&pool, &session).await {
                Ok(()) => {
                    retry.reset();
                    Step::Connect
                }
                Err(error) => {
                    set_collector_state(
                        &pool,
                        session.vehicle_id,
                        "error",
                        Some(&error.to_string()),
                    )
                    .await?;
                    let delay = retry.delay();
                    tracing::warn!(vehicle_id=%session.vehicle_id, %error, retry_seconds=delay.as_secs(), "Parallax collector disconnected");
                    Step::Delay(delay)
                }
            },
        };
    }
}

async fn collect_connection(pool: &PgPool, session: &CollectorSession) -> Result<()> {
    let (_tx, mut context) =
        watch::channel(crate::ingestion::worker::ActiveSessionContext::default());
    collect_connection_with_context(pool, session, &mut context, None).await
}

async fn collect_connection_with_context(
    pool: &PgPool,
    session: &CollectorSession,
    active_sessions: &mut watch::Receiver<crate::ingestion::worker::ActiveSessionContext>,
    telemetry_tx: Option<&mpsc::Sender<(String, TelemetryEvent)>>,
) -> Result<()> {
    let request = transport::connection_request(&session.tokens)?;
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await?;
    websocket
        .send(transport::connection_init(&session.tokens, Uuid::new_v4()))
        .await?;

    wait_for_ack(&mut websocket).await?;
    websocket
        .send(Message::Text(
            subscription_message(&session.rivian_vehicle_id)
                .to_string()
                .into(),
        ))
        .await?;
    set_collector_state(pool, session.vehicle_id, "connected", None).await?;
    ingestion_capture::record(
        session.vehicle_id,
        CaptureKind::ParallaxConnection,
        json!({ "event": "subscribed", "topics": VEHICLE_STATE_TOPICS.len() }),
    );

    drive_subscribed_socket(pool, session, active_sessions, telemetry_tx, &mut websocket).await
}

async fn drive_subscribed_socket<S>(
    pool: &PgPool,
    session: &CollectorSession,
    active_sessions: &mut watch::Receiver<crate::ingestion::worker::ActiveSessionContext>,
    telemetry_tx: Option<&mpsc::Sender<(String, TelemetryEvent)>>,
    websocket: &mut S,
) -> Result<()>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            message = websocket.next() => {
                let Some(message) = message else {
                    anyhow::bail!("Parallax socket ended");
                };
                match SocketAction::classify(message?) {
                    SocketAction::Reply(message) => websocket.send(message).await?,
                    SocketAction::Envelope(envelope) => {
                        let context = active_sessions.borrow_and_update().clone();
                        if persist_envelope(pool, session.vehicle_id, &envelope, &context, telemetry_tx).await.is_err() {
                            tracing::debug!(vehicle_id=%session.vehicle_id, "Parallax frame rejected by typed decoder");
                            let _ = sqlx::query("UPDATE panda.parallax_collector_state SET decode_error_count=decode_error_count+1,last_frame_at=now(),updated_at=now() WHERE vehicle_id=$1")
                                .bind(session.vehicle_id).execute(pool).await;
                        }
                    }
                    SocketAction::Rejected => {
                        ingestion_capture::record(session.vehicle_id, CaptureKind::ParallaxConnection,
                            json!({ "event": "subscription_rejected" }));
                        anyhow::bail!("Parallax subscription rejected")
                    }
                    SocketAction::Completed => {
                        ingestion_capture::record(session.vehicle_id, CaptureKind::ParallaxConnection,
                            json!({ "event": "subscription_completed" }));
                        anyhow::bail!("Parallax subscription completed")
                    }
                    SocketAction::Closed(frame) => {
                        // Rivian closes every subscription when its connection
                        // TTL runs out. That is a routine renewal, not a
                        // failure: returning Ok resets the backoff so the
                        // collector resubscribes immediately instead of
                        // sitting offline for up to two minutes.
                        let renewal =
                            crate::ingestion::ws_client::is_rivian_connection_ttl_expired(frame.as_ref());
                        ingestion_capture::record(
                            session.vehicle_id,
                            CaptureKind::ParallaxConnection,
                            json!({
                                "event": if renewal { "ttl_expired" } else { "closed" },
                                "close_code": frame.as_ref().map(|f| u16::from(f.code)),
                                "close_reason": frame.as_ref().map(|f| f.reason.to_string()),
                            }),
                        );
                        set_collector_state(pool, session.vehicle_id, "disconnected", None)
                            .await?;
                        if renewal {
                            return Ok(());
                        }
                        anyhow::bail!("Parallax socket closed: {frame:?}");
                    }
                    SocketAction::Ignore => {}
                }
            }
            _ = heartbeat.tick() => {
                touch_collector_heartbeat(pool, session.vehicle_id).await?;
            }
        }
    }
}

async fn touch_collector_heartbeat(pool: &PgPool, vehicle_id: Uuid) -> Result<()> {
    let count = AcquisitionWrite::new("panda.parallax_collector_state")
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .update(pool, &[("vehicle_id", vehicle_id)], "status = 'connected'")
        .await?;
    anyhow::ensure!(
        count == 1,
        "Parallax collector heartbeat state is missing or disconnected"
    );
    Ok(())
}

async fn wait_for_ack<S>(websocket: &mut S) -> Result<()>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut acknowledged = false;
    while !acknowledged {
        let frame = tokio::time::timeout_at(deadline, websocket.next())
            .await
            .context("timed out waiting for Parallax acknowledgement")?
            .context("socket ended before Parallax acknowledgement")??;
        match frame {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(&text)?;
                acknowledged = value.get("type").and_then(Value::as_str) == Some("connection_ack");
            }
            Message::Ping(payload) => websocket.send(Message::Pong(payload)).await?,
            _ => {}
        }
    }
    Ok(())
}

fn subscription_message(vehicle_id: &str) -> Value {
    json!({
        "id": SUBSCRIPTION_ID,
        "type": "subscribe",
        "payload": {
            "operationName": "ParallaxMessages",
            "variables": { "vehicleId": vehicle_id, "rvms": VEHICLE_STATE_TOPICS },
            "query": "subscription ParallaxMessages($vehicleId: String!, $rvms: [String!]) { parallaxMessages(vehicleId: $vehicleId, rvms: $rvms) { payload timestamp rvm } }"
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvelopeOutcome {
    Empty,
    Ignored,
    Decoded,
    Rejected,
}

impl EnvelopeOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Ignored => "ignored",
            Self::Decoded => "decoded",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForwardOutcome {
    NotApplicable,
    NotAttached,
    NoCanonicalData,
    Stale,
    Enqueued,
    Full,
    Closed,
    NotEvaluated,
}

impl ForwardOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::NotAttached => "not_attached",
            Self::NoCanonicalData => "no_canonical_data",
            Self::Stale => "stale",
            Self::Enqueued => "enqueued",
            Self::Full => "full",
            Self::Closed => "closed",
            Self::NotEvaluated => "not_evaluated",
        }
    }
}

/// Facts gathered while handling one envelope, for an ingestion capture.
#[derive(Debug)]
struct EnvelopeReport {
    forward_outcome: ForwardOutcome,
    decoded: Option<Value>,
    notes: Vec<String>,
}

impl EnvelopeReport {
    fn new() -> Self {
        Self {
            forward_outcome: ForwardOutcome::NotApplicable,
            decoded: None,
            notes: Vec::new(),
        }
    }
}

/// Raw payload bytes kept in a capture, per envelope.
const CAPTURE_PAYLOAD_MAX_BYTES: usize = 4096;

/// Raw payloads are captured only for topics that cannot carry location or
/// network identity, so a capture stays shareable.
fn capture_payload_allowed(topic: &str) -> bool {
    !matches!(topic, "dynamics.vehicle.gnss" | "vehicle.network.state")
}

/// Every entry of a body frame with the canonical field it maps to (or
/// `null`), so a capture shows positions the decoder does not know.
fn body_entries(topic: &str, payload: &[u8]) -> Option<Value> {
    let encode = |position: Option<i32>,
                  state: Option<i32>,
                  name: fn(i32) -> Option<&'static str>| {
        json!({"position":position,"state":state,"field":position.and_then(name)})
    };
    let entries = match topic {
        "body.closures.states" => ClosureStates::decode(payload)
            .ok()?
            .states
            .into_iter()
            .map(|entry| encode(entry.position, entry.state, closure_field_name))
            .collect(),
        "body.locks.states" => LockStates::decode(payload)
            .ok()?
            .states
            .into_iter()
            .map(|entry| encode(entry.position, entry.state, lock_field_name))
            .collect(),
        _ => return None,
    };
    Some(Value::Array(entries))
}

fn capture_envelope(
    vehicle_id: Uuid,
    topic: &str,
    payload: Option<&[u8]>,
    received_at: DateTime<Utc>,
    source_at: DateTime<Utc>,
    outcome: &str,
    report: &EnvelopeReport,
) {
    #[derive(serde::Serialize)]
    struct Captured<'a> {
        topic: &'a str,
        source_at: String,
        source_age_ms: i64,
        payload_bytes: Option<usize>,
        outcome: &'a str,
        forward_outcome: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload_hex: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload_truncated: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        entries: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        decoded: Option<&'a Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        notes: Option<&'a Vec<String>>,
    }
    let kept = payload
        .filter(|_| capture_payload_allowed(topic))
        .map(|bytes| &bytes[..bytes.len().min(CAPTURE_PAYLOAD_MAX_BYTES)]);
    let truncated = kept
        .zip(payload)
        .filter(|(kept, original)| kept.len() < original.len())
        .map(|_| true);
    let captured = Captured {
        topic,
        source_at: source_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        source_age_ms: (received_at - source_at).num_milliseconds(),
        payload_bytes: payload.map(<[u8]>::len),
        outcome,
        forward_outcome: report.forward_outcome.as_str(),
        payload_hex: kept.map(hex::encode),
        payload_truncated: truncated,
        entries: payload.and_then(|bytes| body_entries(topic, bytes)),
        decoded: report.decoded.as_ref(),
        notes: (!report.notes.is_empty()).then_some(&report.notes),
    };
    ingestion_capture::record(vehicle_id, CaptureKind::ParallaxEnvelope, json!(captured));
}

fn is_canonical_telemetry_topic(topic: &str) -> bool {
    matches!(
        topic,
        "vehicle.power.state"
            | "dynamics.vehicle.gnss"
            | "dynamics.vehicle.odometer"
            | "dynamics.vehicle.gear"
            | "body.closures.states"
            | "body.locks.states"
            | "dynamics.tires.state"
            | "comfort.cabin.cabin_temperatures"
            | "comfort.cabin.cabin_preconditioning_status"
            | "comfort.cabin.defrost_defog_status"
    )
}

async fn persist_envelope(
    pool: &PgPool,
    vehicle_id: Uuid,
    envelope: &Value,
    active_session: &crate::ingestion::worker::ActiveSessionContext,
    telemetry_tx: Option<&mpsc::Sender<(String, TelemetryEvent)>>,
) -> Result<()> {
    let topic = envelope
        .get("rvm")
        .and_then(Value::as_str)
        .context("missing RVM topic")?;
    let capturing = ingestion_capture::is_capturing(vehicle_id);
    let received_at = Utc::now();
    let source_at = parse_source_at(envelope.get("timestamp")).unwrap_or(received_at);
    let mut report = EnvelopeReport::new();
    let mut payload = None;
    let result: Result<&'static str> = async {
        if !is_allowlisted_topic(topic) {
            return Ok("not_allowlisted");
        }
        let encoded = envelope
            .get("payload")
            .and_then(Value::as_str)
            .context("missing Parallax payload")?;
        payload = Some(BASE64.decode(encoded)?);
        persist_allowlisted_envelope(
            pool,
            vehicle_id,
            topic,
            payload.as_deref().expect("decoded payload"),
            received_at,
            source_at,
            active_session,
            telemetry_tx,
            &mut report,
            capturing,
        )
        .await
        .map(EnvelopeOutcome::as_str)
    }
    .await;
    if capturing {
        if let Err(error) = &result {
            if payload.is_none() {
                report.forward_outcome = ForwardOutcome::NotEvaluated;
            }
            report.notes.push(format!("{error}"));
        }
        capture_envelope(
            vehicle_id,
            topic,
            payload.as_deref(),
            received_at,
            source_at,
            result
                .as_ref()
                .copied()
                .unwrap_or(EnvelopeOutcome::Rejected.as_str()),
            &report,
        );
    }
    result.map(|_| ())
}

#[allow(clippy::too_many_arguments)]
async fn persist_allowlisted_envelope(
    pool: &PgPool,
    vehicle_id: Uuid,
    topic: &str,
    payload: &[u8],
    received_at: DateTime<Utc>,
    source_at: DateTime<Utc>,
    active_session: &crate::ingestion::worker::ActiveSessionContext,
    telemetry_tx: Option<&mpsc::Sender<(String, TelemetryEvent)>>,
    report: &mut EnvelopeReport,
    capturing: bool,
) -> Result<EnvelopeOutcome> {
    let hash = Sha256::digest(payload).to_vec();
    let associated_session = matching_active_session(active_session, source_at);

    let canonical_outcome = forwarding::project_and_forward(
        topic,
        payload,
        received_at,
        source_at,
        vehicle_id,
        telemetry_tx,
        report,
        capturing,
    )?;

    let snapshot = Snapshot::decode(topic, payload, active_session)?;
    match snapshot {
        Snapshot::Network(value) => {
            let wifi = value.wifi;
            let cellular = value.cellular;
            sqlx::query(
                r#"INSERT INTO timeseries.parallax_network_samples
                   (vehicle_id, source_at, received_at, payload_hash, overall_state,
                    active_transport, wifi_status, wifi_connected, wifi_rssi_dbm,
                    wifi_link_speed_mbps, wifi_frequency_mhz, wifi_channel_width_mhz,
                    cellular_access_technology, cellular_signal_dbm, schema_version)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
                   ON CONFLICT DO NOTHING"#,
            )
            .bind(vehicle_id)
            .bind(source_at)
            .bind(received_at)
            .bind(hash)
            .bind(value.overall_state)
            .bind(value.active_transport)
            .bind(wifi.status)
            .bind(value.connected)
            .bind(wifi.rssi_dbm)
            .bind(wifi.link_speed_mbps)
            .bind(wifi.frequency_mhz)
            .bind(wifi.channel_width_mhz)
            .bind(cellular.access_technology)
            .bind(cellular.signal_dbm)
            .bind(SCHEMA_VERSION)
            .execute(pool)
            .await?;
        }
        Snapshot::Efficiency(value) => {
            sqlx::query(
                r#"INSERT INTO timeseries.parallax_efficiency_samples
                   (vehicle_id, source_at, received_at, payload_hash, reference_wh_per_km,
                    learned_wh_per_km, mode_ranges_km, schema_version)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING"#,
            )
            .bind(vehicle_id)
            .bind(source_at)
            .bind(received_at)
            .bind(hash)
            .bind(value.reference_wh_per_km)
            .bind(value.learned_wh_per_km)
            .bind(sqlx::types::Json(value.ranges))
            .bind(SCHEMA_VERSION)
            .execute(pool)
            .await?;
        }
        Snapshot::Mass(mass) => {
            if let Some(mass) = mass {
                sqlx::query(
                    r#"INSERT INTO timeseries.parallax_mass_samples
                       (vehicle_id, source_at, received_at, payload_hash, estimated_mass_kg, schema_version)
                       VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING"#,
                )
                .bind(vehicle_id).bind(source_at).bind(received_at).bind(hash)
                .bind(mass).bind(SCHEMA_VERSION).execute(pool).await?;
            }
        }
        Snapshot::Parked(windows) => {
            for (window, sample) in windows {
                persist_parked_window(
                    pool,
                    vehicle_id,
                    source_at,
                    received_at,
                    &hash,
                    window,
                    sample,
                )
                .await?;
            }
        }
        Snapshot::Breakdown {
            total_kwh,
            power_kw: current_power_kw,
            state,
        } => {
            sqlx::query(
                r#"INSERT INTO timeseries.parallax_charge_breakdown_samples
                   (vehicle_id, source_at, received_at, payload_hash, charge_session_id, total_kwh, pack_kwh,
                    thermal_kwh, duration_minutes, charging_state, completion_state, schema_version)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT DO NOTHING"#,
            )
            .bind(vehicle_id)
            .bind(source_at)
            .bind(received_at)
            .bind(hash)
            .bind(associated_session)
            .bind(total_kwh)
            .bind(None::<f64>)
            .bind(None::<f64>)
            .bind(None::<i32>)
            .bind(state)
            .bind(None::<i32>)
            .bind(SCHEMA_VERSION)
            .execute(pool)
            .await?;
            if let Some(session_id) = associated_session {
                if let Some(total_kwh) = total_kwh {
                    sqlx::query("UPDATE panda.charge_sessions SET parallax_total_charged_kwh=$1,parallax_total_energy_observed_at=$4 WHERE id=$2 AND vehicle_id=$3 AND ended_at IS NULL AND (parallax_total_energy_observed_at IS NULL OR $4>=parallax_total_energy_observed_at)")
                        .bind(total_kwh).bind(session_id).bind(vehicle_id).bind(source_at).execute(pool).await?;
                }
                if let Some(power_kw) = current_power_kw {
                    update_parallax_power(pool, vehicle_id, session_id, power_kw, source_at)
                        .await?;
                }
            }
        }
        Snapshot::Pack(pack) => {
            if let Some(session_id) = associated_session {
                sqlx::query("UPDATE panda.charge_sessions SET parallax_pack_energy_kwh=$1,parallax_pack_energy_observed_at=$4 WHERE id=$2 AND vehicle_id=$3 AND ended_at IS NULL AND (parallax_pack_energy_observed_at IS NULL OR $4>=parallax_pack_energy_observed_at)")
                    .bind(pack).bind(session_id).bind(vehicle_id).bind(source_at).execute(pool).await?;
            }
        }
        Snapshot::Curve {
            points,
            latest_power,
        } => {
            for point in points {
                sqlx::query("INSERT INTO timeseries.parallax_charge_curve_points (vehicle_id,source_at,segment_index,charge_session_id,power_kw,soc,delivered_energy_kwh,received_at,schema_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT DO NOTHING")
                        .bind(vehicle_id).bind(point.at).bind(point.index).bind(point.session)
                        .bind(point.power_kw).bind(point.soc)
                        .bind(None::<f64>).bind(received_at).bind(SCHEMA_VERSION)
                        .execute(pool).await?;
            }
            if let Some((observed_at, power)) = latest_power {
                if let Some(session_id) = matching_active_session(active_session, observed_at) {
                    update_parallax_power(pool, vehicle_id, session_id, power, observed_at).await?;
                }
            }
        }
        Snapshot::Remaining(minutes) => {
            if let Some(session_id) = associated_session {
                sqlx::query("UPDATE panda.charge_sessions SET parallax_time_remaining_minutes=$1,parallax_time_observed_at=$4 WHERE id=$2 AND vehicle_id=$3 AND ended_at IS NULL AND (parallax_time_observed_at IS NULL OR $4>=parallax_time_observed_at)")
                    .bind(minutes).bind(session_id).bind(vehicle_id).bind(source_at).execute(pool).await?;
            }
        }
        Snapshot::Status(state) => {
            if let Some(session_id) = associated_session {
                sqlx::query("UPDATE panda.charge_sessions SET parallax_charger_status=$1,parallax_status_observed_at=$4 WHERE id=$2 AND vehicle_id=$3 AND ended_at IS NULL AND (parallax_status_observed_at IS NULL OR $4>=parallax_status_observed_at)")
                    .bind(state).bind(session_id).bind(vehicle_id).bind(source_at).execute(pool).await?;
            }
        }
        Snapshot::Cold {
            available,
            limited,
            impact_km,
        } => {
            sqlx::query(
                r#"INSERT INTO timeseries.parallax_cold_weather_samples
                   (vehicle_id, source_at, received_at, payload_hash, available_soc_pct,
                    cold_limited_soc_pct, cold_range_impact_km, schema_version)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING"#,
            )
            .bind(vehicle_id)
            .bind(source_at)
            .bind(received_at)
            .bind(hash)
            .bind(available)
            .bind(limited)
            .bind(impact_km)
            .bind(SCHEMA_VERSION)
            .execute(pool)
            .await?;
        }
        Snapshot::Rejected => {
            record_decode_error(pool, vehicle_id).await?;
            return Ok(EnvelopeOutcome::Rejected);
        }
        Snapshot::Empty | Snapshot::Ignored => {
            record_empty_frame(pool, vehicle_id).await?;
            return Ok(if matches!(snapshot, Snapshot::Empty) {
                EnvelopeOutcome::Empty
            } else {
                EnvelopeOutcome::Ignored
            });
        }
        Snapshot::Canonical => {
            let outcome = canonical_outcome.unwrap_or(EnvelopeOutcome::Ignored);
            if outcome != EnvelopeOutcome::Decoded {
                record_empty_frame(pool, vehicle_id).await?;
                return Ok(outcome);
            }
        }
    }

    sqlx::query(
        r#"UPDATE panda.parallax_collector_state
           SET last_event_at = $2, last_frame_at=$2, last_meaningful_frame_at=$2,
               status = 'connected', last_error = NULL, updated_at = now()
           WHERE vehicle_id = $1"#,
    )
    .bind(vehicle_id)
    .bind(received_at)
    .execute(pool)
    .await?;
    Ok(canonical_outcome.unwrap_or(EnvelopeOutcome::Decoded))
}

async fn record_decode_error(pool: &PgPool, vehicle_id: Uuid) -> Result<()> {
    AcquisitionWrite::new("panda.parallax_collector_state")
        .field(
            "decode_error_count",
            Stored::Sql("decode_error_count + 1"),
            MergeWrite::Replace,
        )
        .field(
            "last_error",
            Stored::Sql("'unsupported or malformed charging schema'"),
            MergeWrite::Replace,
        )
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .update(pool, &[("vehicle_id", vehicle_id)], "")
        .await?;
    Ok(())
}

async fn record_empty_frame(pool: &PgPool, vehicle_id: Uuid) -> Result<()> {
    AcquisitionWrite::new("panda.parallax_collector_state")
        .field(
            "empty_frame_count",
            Stored::Sql("empty_frame_count + 1"),
            MergeWrite::Replace,
        )
        .field("last_frame_at", Stored::Sql("now()"), MergeWrite::Replace)
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .update(pool, &[("vehicle_id", vehicle_id)], "")
        .await?;
    Ok(())
}

fn matching_active_session(
    context: &crate::ingestion::worker::ActiveSessionContext,
    source_at: DateTime<Utc>,
) -> Option<Uuid> {
    let id = context.session_id?;
    if context
        .started_at
        .is_some_and(|started| source_at < started)
        || context.ended_at.is_some_and(|ended| source_at > ended)
    {
        None
    } else {
        Some(id)
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist_parked_window(
    pool: &PgPool,
    vehicle_id: Uuid,
    source_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
    hash: &[u8],
    window: &str,
    value: ParkedEnergyWindow,
) -> Result<()> {
    let start = value
        .duration_minutes
        .map(|minutes| source_at - chrono::Duration::minutes(i64::from(minutes)));
    let mut row = AcquisitionWrite::new("timeseries.parallax_parked_energy_samples")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field("source_at", Stored::Time(Some(source_at)), MergeWrite::Keep)
        .field(
            "received_at",
            Stored::Time(Some(received_at)),
            MergeWrite::Keep,
        )
        .field("payload_hash", Stored::Bytes(hash), MergeWrite::Keep)
        .field(
            "period_window",
            Stored::Text(Some(window)),
            MergeWrite::Keep,
        )
        .field("parked_started_at", Stored::Time(start), MergeWrite::Keep)
        .field(
            "duration_minutes",
            Stored::Int(value.duration_minutes),
            MergeWrite::Keep,
        )
        .field(
            "schema_version",
            Stored::Int(Some(SCHEMA_VERSION)),
            MergeWrite::Keep,
        );
    for (column, reading) in [
        ("total_kwh", value.total_kwh),
        ("vehicle_systems_kwh", value.vehicle_systems_kwh),
        ("outlets_kwh", value.outlets_kwh),
        ("climate_kwh", value.climate_kwh),
        ("gear_guard_kwh", value.gear_guard_kwh),
        ("total_range_impact_km", value.total_range_impact_km),
        (
            "vehicle_systems_range_impact_km",
            value.vehicle_systems_range_impact_km,
        ),
        ("outlets_range_impact_km", value.outlets_range_impact_km),
        ("climate_range_impact_km", value.climate_range_impact_km),
        (
            "gear_guard_range_impact_km",
            value.gear_guard_range_impact_km,
        ),
    ] {
        row = row.field(column, Stored::Float(f64_opt(reading)), MergeWrite::Keep);
    }
    row.upsert(pool, "", "").await?;
    Ok(())
}

async fn set_collector_state(
    pool: &PgPool,
    vehicle_id: Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let error = error.map(|text| text.chars().take(500).collect::<String>());
    AcquisitionWrite::new("panda.parallax_collector_state")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field("status", Stored::Text(Some(status)), MergeWrite::Replace)
        .field(
            "connected_at",
            Stored::Sql(if status == "connected" {
                "now()"
            } else {
                "NULL"
            }),
            MergeWrite::Expression(
                "COALESCE(panda.parallax_collector_state.connected_at, EXCLUDED.connected_at)",
            ),
        )
        .field(
            "last_error",
            Stored::Text(error.as_deref()),
            MergeWrite::Replace,
        )
        .field(
            "schema_version",
            Stored::Int(Some(SCHEMA_VERSION)),
            MergeWrite::Replace,
        )
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .upsert(pool, "(vehicle_id)", "")
        .await?;
    Ok(())
}

fn parse_source_at(value: Option<&Value>) -> Option<DateTime<Utc>> {
    match value? {
        Value::String(text) => DateTime::parse_from_rfc3339(text)
            .map(|at| at.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                text.parse::<i64>()
                    .ok()
                    .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
            }),
        value => value
            .as_i64()
            .and_then(|millis| Utc.timestamp_millis_opt(millis).single()),
    }
}

fn without_signal_sentinel(value: Option<i32>) -> Option<i32> {
    value.filter(|value| (-150..=0).contains(value))
}

fn f64_opt(value: Option<f32>) -> Option<f64> {
    value.map(f64::from).filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::{
        trip_detector::{compute_distance_odometer_or_gps, TripDetectorState, TripEvent},
        trip_signals::TripSignalFusion,
    };

    #[test]
    fn decoded_r2_power_and_sparse_gnss_complete_a_trip() {
        let vehicle_id = Uuid::new_v4();
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut detector = TripDetectorState::new(vehicle_id);
        let power = decode_vehicle_telemetry(
            "vehicle.power.state",
            &PowerStateMessage { state: Some(4) }.encode_to_vec(),
            at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        detector.process(&fusion.fuse(&power));
        for (seconds, longitude) in [
            (10, -97.0),
            (40, -96.999),
            (70, -96.998),
            (100, -96.997),
            (130, -96.996),
        ] {
            let ts = at + chrono::Duration::seconds(seconds);
            let fix = decode_vehicle_telemetry(
                "dynamics.vehicle.gnss",
                &GnssState {
                    latitude: Some(30.0),
                    longitude: Some(longitude),
                    altitude_m: None,
                }
                .encode_to_vec(),
                ts,
                vehicle_id,
            )
            .unwrap()
            .unwrap();
            let transition = detector.process(&fusion.fuse(&fix));
            if seconds == 70 {
                assert!(matches!(transition, TripEvent::TripStarted { .. }));
            }
        }
        let stopped_at = at + chrono::Duration::seconds(140);
        let sleep = decode_vehicle_telemetry(
            "vehicle.power.state",
            &PowerStateMessage { state: Some(1) }.encode_to_vec(),
            stopped_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        let TripEvent::TripEnded { trip } = detector.process(&fusion.fuse(&sleep)) else {
            panic!("expected completed R2 trip")
        };
        assert!(
            compute_distance_odometer_or_gps(
                trip.start_odometer_mi,
                trip.end_odometer_mi,
                &trip.points
            ) >= 0.1
        );
    }

    #[test]
    fn parked_energy_wire_contract_decodes_units() {
        let window = ParkedEnergyWindow {
            total_kwh: Some(1.25),
            vehicle_systems_kwh: Some(0.75),
            climate_kwh: Some(0.5),
            total_range_impact_km: Some(4.2),
            duration_minutes: Some(480),
            ..Default::default()
        };
        let payload = ParkedEnergyDistributions {
            hours_8: Some(window),
            ..Default::default()
        }
        .encode_to_vec();
        let decoded = ParkedEnergyDistributions::decode(payload.as_slice()).unwrap();
        let decoded = decoded.hours_8.unwrap();
        assert_eq!(decoded.duration_minutes, Some(480));
        assert_eq!(decoded.total_kwh, Some(1.25));
        assert_eq!(decoded.total_range_impact_km, Some(4.2));
    }

    #[test]
    fn network_decoder_does_not_model_identifiers() {
        let payload = NetworkState {
            overall_state: Some(1),
            active_transport: Some(4),
            wifi: Some(WifiState {
                status: Some(2),
                rssi_dbm: Some(-56),
                link_speed_mbps: Some(117),
                frequency_mhz: Some(2437),
                channel_width_mhz: Some(20),
                ..Default::default()
            }),
            cellular: Some(CellularState {
                access_technology: Some("LTE".into()),
                signal_dbm: Some(-255),
            }),
        }
        .encode_to_vec();
        let decoded = NetworkState::decode(payload.as_slice()).unwrap();
        assert_eq!(decoded.wifi.unwrap().rssi_dbm, Some(-56));
        assert_eq!(
            without_signal_sentinel(decoded.cellular.unwrap().signal_dbm),
            None
        );
    }

    #[test]
    fn subscription_uses_one_allowlist_for_every_vehicle() {
        let expected = VEHICLE_STATE_TOPICS
            .iter()
            .map(|topic| Value::String((*topic).into()))
            .collect::<Vec<_>>();
        for vehicle_id in ["vehicle-a", "vehicle-b", "vehicle-c"] {
            let message = subscription_message(vehicle_id);
            let topics = message["payload"]["variables"]["rvms"].as_array().unwrap();
            assert_eq!(topics, &expected);
        }
        assert!(expected.iter().any(|topic| topic == "vehicle.power.state"));
        assert!(expected
            .iter()
            .any(|topic| topic == "dynamics.vehicle.gnss"));
        assert!(expected
            .iter()
            .any(|topic| topic == "energy_edge_compute.graphs.parked_energy_distributions"));
    }

    #[test]
    fn unsolicited_topics_cannot_cross_the_allowlist() {
        assert!(VEHICLE_STATE_TOPICS
            .iter()
            .all(|topic| is_allowlisted_topic(topic)));
        assert!(!is_allowlisted_topic("unknown.vehicle.topic"));
        assert!(!is_canonical_telemetry_topic("unknown.vehicle.topic"));
    }

    #[test]
    fn captured_vehicle_payloads_decode_with_verified_units() {
        let efficiency = EfficiencyState::decode(
            BASE64
                .decode("CM8BEPgBGgUIARCTBBoFCAIQkwQaBQgDENcDGgUIBBDhAxoFCAUQrQMaBQgGEOQDGgUIBxCtAxoFCAgQ5AMaBQgJEK0DGgUIChDkAw==")
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(efficiency.reference_wh_per_km, Some(207));
        assert_eq!(efficiency.learned_wh_per_km, Some(248));
        assert_eq!(efficiency.mode_ranges[0].full_charge_range_km, Some(531));

        let mass = MassEstimate::decode(BASE64.decode("CNgY").unwrap().as_slice()).unwrap();
        assert_eq!(mass.estimated_mass_kg, Some(3160));

        let network = NetworkState::decode(
            BASE64
                .decode("CAESBAgBEAISBAgCEAISBAgDEAISBAgEEAIYBCIoCAIQAhoIRGF2aXNJb1Q4BEDI//////////8BSBpQ7BJYFGAEaAJwAioYCgRBVCZUEgNMVEUYAyCB/v////////8B")
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let wifi = network.wifi.unwrap();
        assert_eq!(wifi.rssi_dbm, Some(-56));
        assert_eq!(wifi.link_speed_mbps, Some(26));
        assert_eq!(wifi.frequency_mhz, Some(2412));

        let charge = ChargeBreakdown::decode(
            BASE64
                .decode("DWdmtkEVZ2auQS0AAIA/MKsBWgBgAWgB")
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert!((charge.total_kwh.unwrap() - 22.8).abs() < 0.01);

        let cold = ColdWeatherSoc::decode(BASE64.decode("CEI=").unwrap().as_slice()).unwrap();
        assert_eq!(cold.available_soc_pct, Some(66));
    }

    #[test]
    fn charging_topic_fixtures_decode_only_proven_fields() {
        let battery = HvBatteryState::decode(
            [
                0x0a, 0x12, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49, 0x40, 0x11, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x39, 0x40,
            ]
            .as_slice(),
        )
        .unwrap();
        assert_eq!(battery.charge_state.unwrap().pack_energy_kwh, Some(25.0));

        let time = ChargingTimeEstimation::decode([0x08, 0xd8, 0x13].as_slice()).unwrap();
        assert_eq!(time.remaining_seconds, Some(2520));

        let status =
            ChargingStatus::decode([0x08, 0x01, 0x10, 0x02, 0x18, 0x03].as_slice()).unwrap();
        assert_eq!(status.plug_connection_status, Some(1));
        assert_eq!(status.display_status, Some(2));
        assert_eq!(status.evse_type, Some(3));

        let graph = ChargingGraphGlobal::decode(
            [
                0x0a, 0x0f, 0x08, 0x32, 0x15, 0x00, 0x00, 0x30, 0x41, 0x18, 0xe8, 0x07, 0x20, 0xd0,
                0x0f, 0x30, 0x03,
            ]
            .as_slice(),
        )
        .unwrap();
        assert_eq!(graph.segments.len(), 1);
        assert_eq!(graph.segments[0].start_unix_ms, Some(1000));
        assert_eq!(graph.segments[0].power_kw, Some(11.0));
        assert_eq!(graph.segments[0].soc, Some(50));
    }

    #[test]
    fn r2_vehicle_topics_decode_into_timestamped_partial_events() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();
        let power = decode_vehicle_telemetry(
            "vehicle.power.state",
            &PowerStateMessage { state: Some(4) }.encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(power.vehicle_id, vehicle_id);
        assert_eq!(power.ts, source_at);
        assert_eq!(power.power_state, Some(PowerState::Go));
        assert_eq!(power.power_state_ts, Some(source_at));

        let gnss = decode_vehicle_telemetry(
            "dynamics.vehicle.gnss",
            &GnssState {
                latitude: Some(40.0),
                longitude: Some(-105.0),
                altitude_m: Some(1_600.0),
            }
            .encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(gnss.latitude, Some(40.0));
        assert_eq!(gnss.longitude, Some(-105.0));
        assert_eq!(gnss.altitude_m, Some(1_600.0));
        assert_eq!(gnss.location_ts, Some(source_at));

        let odometer = decode_vehicle_telemetry(
            "dynamics.vehicle.odometer",
            &OdometerState {
                kilometers: Some(100),
            }
            .encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert!((odometer.odometer_miles.unwrap() - 62.1371).abs() < 0.001);
        assert_eq!(odometer.odometer_miles_ts, Some(source_at));
    }

    #[test]
    fn r2_body_climate_and_tire_topics_map_public_fixture_shapes() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();
        let body = decode_vehicle_telemetry(
            "body.closures.states",
            &ClosureStates {
                states: vec![ClosureState {
                    position: Some(1),
                    state: Some(2),
                }],
            }
            .encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(body.door_front_left_closed, Some(true));

        let tire = decode_vehicle_telemetry(
            "dynamics.tires.state",
            &TireStates {
                states: vec![TireState {
                    position: Some(1),
                    status: Some(1),
                    pressure_bar: Some(2.5),
                }],
            }
            .encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert!((tire.tire_fl_psi.unwrap() - 36.2594).abs() < 0.001);
        assert_eq!(tire.tire_fl_status.as_deref(), Some("OK"));
        assert_eq!(tire.tire_fl_valid, Some(true));

        let climate = decode_vehicle_telemetry(
            "comfort.cabin.cabin_temperatures",
            &CabinTemperatures {
                cabin_c: Some(21.5),
                driver_c: Some(20.0),
            }
            .encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(climate.cabin_temp_c, Some(21.5));
        assert_eq!(climate.driver_temp_c, Some(20.0));
    }

    #[test]
    fn preconditioning_empty_and_known_enum_states_are_canonicalized() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();
        let empty = decode_vehicle_telemetry(
            "comfort.cabin.cabin_preconditioning_status",
            &[],
            source_at,
            vehicle_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(empty.cabin_precon_status.as_deref(), Some("off"));

        for (state, expected) in [
            (0, "off"),
            (1, "initiate"),
            (2, "initiate"),
            (3, "off"),
            (4, "active"),
        ] {
            let event = decode_vehicle_telemetry(
                "comfort.cabin.cabin_preconditioning_status",
                &PreconditioningState {
                    status: Some(state),
                }
                .encode_to_vec(),
                source_at,
                vehicle_id,
            )
            .unwrap()
            .unwrap();
            assert_eq!(event.cabin_precon_status.as_deref(), Some(expected));
        }
    }

    #[test]
    fn unknown_or_missing_preconditioning_state_has_no_canonical_update() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();
        let mut notes = Vec::new();
        assert!(decode_vehicle_telemetry_with_notes(
            "comfort.cabin.cabin_preconditioning_status",
            &PreconditioningState { status: Some(8) }.encode_to_vec(),
            source_at,
            vehicle_id,
            &mut notes,
        )
        .unwrap()
        .is_none());
        assert_eq!(notes, vec!["skipped unknown preconditioning status 8"]);

        notes.clear();
        assert!(decode_vehicle_telemetry_with_notes(
            "comfort.cabin.cabin_preconditioning_status",
            &[0xa0, 0x06, 0x01],
            source_at,
            vehicle_id,
            &mut notes,
        )
        .unwrap()
        .is_none());
        assert_eq!(
            notes,
            vec!["skipped preconditioning frame with missing status"]
        );

        assert!(decode_vehicle_telemetry(
            "comfort.cabin.cabin_preconditioning_status",
            &[0x0a, 0xff],
            source_at,
            vehicle_id,
        )
        .is_err());
    }

    fn closure_frame(entries: &[(i32, Option<i32>)]) -> Vec<u8> {
        ClosureStates {
            states: entries
                .iter()
                .map(|&(position, state)| ClosureState {
                    position: Some(position),
                    state,
                })
                .collect(),
        }
        .encode_to_vec()
    }

    #[test]
    fn capture_field_names_agree_with_the_decoder_mappings() {
        for position in -1..=40 {
            let mut event = TelemetryEvent::empty(Uuid::nil(), Utc::now());
            assert_eq!(
                closure_field(&mut event, position).is_some()
                    || position == CLOSURE_CHARGE_PORT_POSITION,
                closure_field_name(position).is_some(),
                "closure position {position}"
            );
            assert_eq!(
                set_lock(&mut event, position, true).is_ok(),
                lock_field_name(position).is_some(),
                "lock position {position}"
            );
        }
    }

    #[test]
    fn capture_lists_every_body_entry_including_unmapped_positions() {
        let entries = body_entries(
            "body.closures.states",
            &closure_frame(&[(1, Some(1)), (20, Some(2)), (10000, None)]),
        )
        .unwrap();
        assert_eq!(
            entries,
            json!([
                { "position": 1, "state": 1, "field": "door_front_left_closed" },
                { "position": 20, "state": 2, "field": null },
                { "position": 10000, "state": null, "field": null },
            ])
        );
        assert!(body_entries("vehicle.power.state", &[]).is_none());
    }

    fn lock_frame(entries: &[(i32, Option<i32>)]) -> Vec<u8> {
        LockStates {
            states: entries
                .iter()
                .map(|&(position, state)| LockState {
                    position: Some(position),
                    state,
                })
                .collect(),
        }
        .encode_to_vec()
    }

    #[test]
    fn r1_lock_frame_keeps_mapped_locks_and_notes_extra_positions() {
        // Observed R1S frame mid-lock: doors still unlocked (2), every other
        // position already locked (1).
        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.locks.states",
            &lock_frame(&[
                (1, Some(2)),
                (2, Some(2)),
                (3, Some(2)),
                (4, Some(2)),
                (5, Some(1)),
                (6, Some(1)),
                (7, Some(1)),
                (8, Some(1)),
                (9, Some(1)),
                (14, Some(1)),
                (15, Some(1)),
            ]),
            Utc::now(),
            Uuid::new_v4(),
            &mut notes,
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.door_front_left_locked, Some(false));
        assert_eq!(event.door_rear_right_locked, Some(false));
        assert_eq!(event.closure_frunk_locked, Some(true));
        assert_eq!(event.closure_liftgate_locked, Some(true));
        assert_eq!(event.closure_tailgate_locked, Some(true));
        assert_eq!(event.side_bin_left_locked, Some(true));
        assert_eq!(event.side_bin_right_locked, Some(true));
        assert_eq!(
            notes,
            [14, 15]
                .iter()
                .map(|p| format!("skipped unmapped lock position {p}"))
                .collect::<Vec<_>>()
        );

        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.locks.states",
            &lock_frame(&[(1, Some(9)), (2, None), (5, Some(1))]),
            Utc::now(),
            Uuid::new_v4(),
            &mut notes,
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.door_front_left_locked, None);
        assert_eq!(event.closure_frunk_locked, Some(true));
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn gear_frames_map_to_legacy_gear_strings() {
        for (raw, expected) in [(1, "park"), (2, "reverse"), (3, "neutral"), (4, "drive")] {
            let event = decode_vehicle_telemetry(
                "dynamics.vehicle.gear",
                &GearState { gear: Some(raw) }.encode_to_vec(),
                Utc::now(),
                Uuid::new_v4(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(event.gear_status.as_deref(), Some(expected));
        }
        assert!(is_allowlisted_topic("dynamics.vehicle.gear"));

        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "dynamics.vehicle.gear",
            &GearState { gear: Some(9) }.encode_to_vec(),
            Utc::now(),
            Uuid::new_v4(),
            &mut notes,
        )
        .unwrap();
        assert!(event.is_none());
        assert_eq!(notes, vec!["skipped unknown gear 9"]);
    }

    #[test]
    fn defrost_status_four_is_off_and_unknown_values_are_noted() {
        // Observed R1S frame `08 04` while legacy reported "Off".
        let event = decode_vehicle_telemetry(
            "comfort.cabin.defrost_defog_status",
            &[0x08, 0x04],
            Utc::now(),
            Uuid::new_v4(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.defrost_active, Some(false));

        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "comfort.cabin.defrost_defog_status",
            &[0x08, 0x07],
            Utc::now(),
            Uuid::new_v4(),
            &mut notes,
        )
        .unwrap();
        assert!(event.is_none());
        assert_eq!(notes, vec!["skipped unknown defrost status 7"]);
    }

    #[test]
    fn charge_port_follows_legacy_charge_port_state() {
        // Observed R1S sequence: closed, opening, open, closing, closed. Legacy
        // reported close, opening, open, closing, close at the same moments.
        for (status, open) in [(2, false), (4, false), (1, true), (5, false), (3, true)] {
            let event = decode_vehicle_telemetry(
                "body.closures.states",
                &closure_frame(&[(10, Some(status))]),
                Utc::now(),
                Uuid::new_v4(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(event.charge_port_open, Some(open), "status {status}");
            assert!(event.closure_transitions.is_none());
        }
        assert_eq!(closure_field_name(10), Some("charge_port_open"));
    }

    #[test]
    fn capture_never_keeps_location_or_network_payloads() {
        assert!(!capture_payload_allowed("dynamics.vehicle.gnss"));
        assert!(!capture_payload_allowed("vehicle.network.state"));
        assert!(capture_payload_allowed("body.closures.states"));
        assert!(capture_payload_allowed("vehicle.power.state"));
    }

    #[test]
    fn r2_closure_frame_decodes_observed_shape_and_reports_unexpected_entries() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();

        // Observed R2 frame while the liftgate closes and the front-left
        // window is open: rear glass (16) and the stateless sentinel (10000)
        // are expected skips and must not produce diagnostics notes.
        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.closures.states",
            &closure_frame(&[
                (1, Some(2)),
                (2, Some(2)),
                (3, Some(2)),
                (4, Some(2)),
                (5, Some(1)),
                (7, Some(3)),
                (12, Some(1)),
                (13, Some(2)),
                (14, Some(2)),
                (15, Some(2)),
                (16, Some(2)),
                (10000, None),
            ]),
            source_at,
            vehicle_id,
            &mut notes,
        )
        .unwrap()
        .unwrap();
        assert!(notes.is_empty(), "unexpected notes: {notes:?}");
        assert_eq!(event.door_front_left_closed, Some(true));
        assert_eq!(event.door_rear_right_closed, Some(true));
        assert_eq!(event.closure_frunk_closed, Some(false));
        // AJAR while the powered liftgate closes is still "not closed".
        assert_eq!(event.closure_liftgate_closed, Some(false));
        assert_eq!(event.window_fl_closed, Some(false));
        assert_eq!(event.window_rr_closed, Some(true));

        // Unexpected states and positions are skipped without discarding the
        // frame, and each one is reported for ingestion diagnostics.
        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.closures.states",
            &closure_frame(&[
                (1, Some(4)),
                (2, None),
                (5, Some(2)),
                (20, Some(2)),
                (10000, None),
            ]),
            source_at,
            vehicle_id,
            &mut notes,
        )
        .unwrap()
        .unwrap();
        // OPENING is not closed; a missing status means the closure is not
        // fitted and is skipped silently.
        assert_eq!(event.door_front_left_closed, Some(false));
        assert_eq!(event.door_front_right_closed, None);
        assert_eq!(event.closure_frunk_closed, Some(true));
        assert_eq!(
            notes,
            vec!["skipped unmapped closure position 20 with state 2"]
        );
    }

    #[test]
    fn r1_closure_frame_maps_app_positions_and_skips_absent_closures() {
        // Observed R1S frame with the frunk and liftgate open: the tailgate
        // (6), side bins (8, 9), and tonneau (11) are not fitted, and
        // position 10 is the charge port door.
        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.closures.states",
            &closure_frame(&[
                (1, Some(2)),
                (2, Some(2)),
                (3, Some(2)),
                (4, Some(2)),
                (5, Some(1)),
                (6, None),
                (7, Some(1)),
                (8, None),
                (9, None),
                (10, Some(2)),
                (11, None),
                (12, Some(2)),
                (13, Some(2)),
                (14, Some(2)),
                (15, Some(2)),
                (10000, Some(2)),
            ]),
            Utc::now(),
            Uuid::new_v4(),
            &mut notes,
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.door_rear_right_closed, Some(true));
        assert_eq!(event.closure_frunk_closed, Some(false));
        assert_eq!(event.closure_liftgate_closed, Some(false));
        assert_eq!(event.closure_tailgate_closed, None);
        assert_eq!(event.side_bin_left_closed, None);
        assert_eq!(event.side_bin_right_closed, None);
        assert_eq!(event.tonneau_closed, None);
        assert_eq!(event.window_rr_closed, Some(true));
        assert_eq!(event.charge_port_open, Some(false));
        assert!(notes.is_empty(), "{notes:?}");

        // An R1T: tailgate closing, left side bin open, right side bin and
        // tonneau closed.
        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.closures.states",
            &closure_frame(&[(6, Some(5)), (8, Some(1)), (9, Some(2)), (11, Some(2))]),
            Utc::now(),
            Uuid::new_v4(),
            &mut notes,
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.closure_tailgate_closed, Some(false));
        assert_eq!(
            event.closure_transitions,
            Some(std::collections::BTreeMap::from([(
                "closure_tailgate_closed".to_owned(),
                ClosureTransition::Closing,
            )]))
        );
        assert_eq!(event.side_bin_left_closed, Some(false));
        assert_eq!(event.side_bin_right_closed, Some(true));
        assert_eq!(event.tonneau_closed, Some(true));
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn closure_transitional_states_mean_not_closed() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();
        let mut notes = Vec::new();
        let event = decode_vehicle_telemetry_with_notes(
            "body.closures.states",
            &closure_frame(&[
                (1, Some(2)),
                (5, Some(3)),
                (7, Some(3)),
                (16, Some(3)),
                (20, Some(3)),
                (10000, Some(3)),
            ]),
            source_at,
            vehicle_id,
            &mut notes,
        )
        .unwrap()
        .unwrap();
        // AJAR closures are not closed; the rear drop glass has no field.
        assert_eq!(event.door_front_left_closed, Some(true));
        assert_eq!(event.closure_frunk_closed, Some(false));
        assert_eq!(event.closure_liftgate_closed, Some(false));
        // An unknown position, or a sentinel that carries a state, is
        // reported even while in the moving state.
        assert_eq!(
            notes,
            vec!["skipped unmapped closure position 20 with state 3"]
        );
    }

    #[test]
    fn r2_decoder_rejects_bad_values_and_ignores_unknown_topics() {
        let vehicle_id = Uuid::new_v4();
        let source_at = Utc::now();
        assert!(decode_vehicle_telemetry(
            "dynamics.vehicle.gnss",
            &GnssState {
                latitude: Some(91.0),
                longitude: Some(0.0),
                altitude_m: None
            }
            .encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .is_err());
        assert!(decode_vehicle_telemetry(
            "vehicle.power.state",
            &PowerStateMessage { state: Some(99) }.encode_to_vec(),
            source_at,
            vehicle_id,
        )
        .is_err());
        assert!(
            decode_vehicle_telemetry("unlisted.topic", &[], source_at, vehicle_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unknown_charging_schema_decodes_to_no_authoritative_fields() {
        let unknown = [0xa0, 0x06, 0x01];
        assert_eq!(
            HvBatteryState::decode(unknown.as_slice())
                .unwrap()
                .charge_state,
            None
        );
        assert!(ChargingGraphGlobal::decode(unknown.as_slice())
            .unwrap()
            .segments
            .is_empty());
        assert_eq!(
            ChargingTimeEstimation::decode(unknown.as_slice())
                .unwrap()
                .remaining_seconds,
            None
        );
        let status = ChargingStatus::decode(unknown.as_slice()).unwrap();
        assert!(
            status.plug_connection_status.is_none()
                && status.display_status.is_none()
                && status.evse_type.is_none()
        );
    }

    #[test]
    fn session_association_enforces_canonical_window() {
        let started = Utc::now();
        let id = Uuid::new_v4();
        let context = crate::ingestion::worker::ActiveSessionContext {
            session_id: Some(id),
            started_at: Some(started),
            ended_at: None,
        };
        assert_eq!(matching_active_session(&context, started), Some(id));
        assert_eq!(
            matching_active_session(&context, started - chrono::Duration::seconds(1)),
            None
        );
        let terminal = crate::ingestion::worker::ActiveSessionContext {
            ended_at: Some(started + chrono::Duration::minutes(1)),
            ..context
        };
        assert_eq!(
            matching_active_session(&terminal, started + chrono::Duration::minutes(2)),
            None
        );
    }
}

#[cfg(test)]
#[path = "parallax/__tests__/contract_tests.rs"]
mod contract_tests;

#[cfg(test)]
#[path = "parallax/__tests__/persistence_tests.rs"]
mod persistence_tests;

use crate::ingestion::{
    AcquisitionMerge as MergeWrite, AcquisitionValue as Stored, AcquisitionWrite,
};

type BodyAccessor = fn(&mut TelemetryEvent) -> &mut Option<bool>;
struct BodyField {
    position: i32,
    name: &'static str,
    access: BodyAccessor,
}
const CLOSURE_FIELDS: &[BodyField] = &[
    BodyField {
        position: 1,
        name: "door_front_left_closed",
        access: |event| &mut event.door_front_left_closed,
    },
    BodyField {
        position: 2,
        name: "door_front_right_closed",
        access: |event| &mut event.door_front_right_closed,
    },
    BodyField {
        position: 3,
        name: "door_rear_left_closed",
        access: |event| &mut event.door_rear_left_closed,
    },
    BodyField {
        position: 4,
        name: "door_rear_right_closed",
        access: |event| &mut event.door_rear_right_closed,
    },
    BodyField {
        position: 5,
        name: "closure_frunk_closed",
        access: |event| &mut event.closure_frunk_closed,
    },
    BodyField {
        position: 6,
        name: "closure_tailgate_closed",
        access: |event| &mut event.closure_tailgate_closed,
    },
    BodyField {
        position: 7,
        name: "closure_liftgate_closed",
        access: |event| &mut event.closure_liftgate_closed,
    },
    BodyField {
        position: 8,
        name: "side_bin_left_closed",
        access: |event| &mut event.side_bin_left_closed,
    },
    BodyField {
        position: 9,
        name: "side_bin_right_closed",
        access: |event| &mut event.side_bin_right_closed,
    },
    BodyField {
        position: 11,
        name: "tonneau_closed",
        access: |event| &mut event.tonneau_closed,
    },
    BodyField {
        position: 12,
        name: "window_fl_closed",
        access: |event| &mut event.window_fl_closed,
    },
    BodyField {
        position: 13,
        name: "window_fr_closed",
        access: |event| &mut event.window_fr_closed,
    },
    BodyField {
        position: 14,
        name: "window_rl_closed",
        access: |event| &mut event.window_rl_closed,
    },
    BodyField {
        position: 15,
        name: "window_rr_closed",
        access: |event| &mut event.window_rr_closed,
    },
];
const LOCK_FIELDS: &[BodyField] = &[
    BodyField {
        position: 1,
        name: "door_front_left_locked",
        access: |event| &mut event.door_front_left_locked,
    },
    BodyField {
        position: 2,
        name: "door_front_right_locked",
        access: |event| &mut event.door_front_right_locked,
    },
    BodyField {
        position: 3,
        name: "door_rear_left_locked",
        access: |event| &mut event.door_rear_left_locked,
    },
    BodyField {
        position: 4,
        name: "door_rear_right_locked",
        access: |event| &mut event.door_rear_right_locked,
    },
    BodyField {
        position: 5,
        name: "closure_frunk_locked",
        access: |event| &mut event.closure_frunk_locked,
    },
    BodyField {
        position: 6,
        name: "closure_tailgate_locked",
        access: |event| &mut event.closure_tailgate_locked,
    },
    BodyField {
        position: 7,
        name: "closure_liftgate_locked",
        access: |event| &mut event.closure_liftgate_locked,
    },
    BodyField {
        position: 8,
        name: "side_bin_left_locked",
        access: |event| &mut event.side_bin_left_locked,
    },
    BodyField {
        position: 9,
        name: "side_bin_right_locked",
        access: |event| &mut event.side_bin_right_locked,
    },
];

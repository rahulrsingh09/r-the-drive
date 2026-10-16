//! Per-vehicle ingestion worker: WS + poll + trip/charge detection + DB writes.

use chrono::{DateTime, Duration, Utc};
use redis::AsyncCommands;
use sqlx::{Connection, PgConnection, PgPool};
use tokio::sync::{broadcast, mpsc, watch};
use uuid::Uuid;

use crate::{
    config::Config,
    db::vehicles::get_vehicle_owner_id,
    ingestion::{
        charge_detector::{ActiveChargeSessionSnapshot, ChargeEvent},
        closure_motion::{ClosureMotionMap, ClosureMotionTracker},
        parser, rivian_poll,
        session_store::{decrypt_tokens, RivianTokenBundle},
        trip_detector::{compute_trip_energy, TripEvent},
        trip_signals::{FusionDiagnostics, TripSignalFusion},
        ws_client::{self, WsInboundEvent, WsInboundKind},
    },
    models::{
        state_period::VehicleState,
        telemetry::{ChargerState, PowerState, TelemetryEvent},
    },
    services::{
        cost::recompute_charge_session_cost,
        geofences::match_geofence,
        ingestion_capture::{self, Kind as CaptureKind},
        trip_enrichment::{resolve_trip_location, MatchedLocation},
        trip_routes::build_route_preview,
    },
};

#[cfg(test)]
use crate::ingestion::{charge_detector::ChargeDetectorState, trip_detector::TripDetectorState};

#[cfg(test)]
mod durable_collector_tests;
#[cfg(test)]
mod telemetry_contract_tests;

mod charge_capture;
mod durable_charge_restart;
mod durable_processing;
mod state_decisions;
mod state_history;
mod telemetry_fields;
mod telemetry_gate;
mod telemetry_storage;
mod telemetry_values;
mod trip_capture;

#[cfg(test)]
use state_history::infer_vehicle_state_values;
use state_history::{
    infer_vehicle_state, lifecycle_updates, load_open_state_period, reconcile_state_period_tail,
    transition_state_period,
};
use telemetry_gate::TelemetryPersistenceGate;
use telemetry_storage::{upsert_latest_status, write_telemetry};
use telemetry_values::{
    build_snapshot, event_has_meaningful_payload, event_is_duplicate_patch, immediate_change,
    patch_event,
};

use state_decisions::{is_synthetic_control, runtime_health_update_for_ws_control};

const MIN_TRIP_DISTANCE_MILES: f64 = 0.1;
const ADVISORY_LOCK_NAMESPACE: i64 = 0x52_49_56_57; // "RIVW"
const WS_WATCHDOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
const CHARGE_DETECTOR_REHYDRATE_LOOKBACK_HOURS: i32 = 12;
const CHARGE_DETECTOR_REHYDRATE_STALENESS_MINUTES: i32 = 120;
const STATE_PERIOD_RECONCILE_LOOKBACK_HOURS: i64 = 24;
const STATE_PERIOD_RECONCILE_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(15 * 60);

/// Dropping a worker must also stop its acquisition tasks.
struct WorkerTask<T>(tokio::task::JoinHandle<T>);

impl<T> WorkerTask<T> {
    fn abort(&self) {
        self.0.abort();
    }
}

impl<T> std::future::Future for WorkerTask<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Drop for WorkerTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
async fn save_trip_checkpoint(
    pool: &PgPool,
    vehicle_id: Uuid,
    conn: &mut PgConnection,
    capture: &mut trip_capture::TripCapture,
    shutdown: &mut broadcast::Receiver<()>,
) -> bool {
    let mut delay = 1;
    loop {
        // Attempt the observed sample even if stop is pending; the supervisor
        // bounds a stuck write with its five-second shutdown deadline.
        let result = capture.save(conn).await;
        match result {
            Ok(()) => return true,
            Err(error) => {
                tracing::error!(%vehicle_id, %error, "trip checkpoint failed; retaining state and retrying");
                trip_capture::record_health(pool, vehicle_id, Some("checkpoint_retry_pending"))
                    .await;
            }
        }
        tokio::select! {
            biased;
            _ = shutdown.recv() => return false,
            _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
        }
        delay = (delay * 2).min(30);
        if conn.ping().await.is_err() {
            let replacement = tokio::select! {
                biased;
                _ = shutdown.recv() => return false,
                result = acquire_collector_lock(pool, vehicle_id) => result,
            };
            match replacement {
                Ok(Some(mut replacement)) => {
                    if let Err(error) = capture.verify_reacquired_lock(&mut replacement).await {
                        tracing::error!(%vehicle_id, %error, "trip recovery ownership changed");
                        return false;
                    }
                    *conn = replacement;
                }
                Ok(None) => return false,
                Err(_) => continue,
            }
        }
    }
}

async fn save_collector_checkpoint(
    pool: &PgPool,
    vehicle_id: Uuid,
    conn: &mut PgConnection,
    capture: &mut trip_capture::TripCapture,
    charges: &mut charge_capture::ChargeCapture,
    shutdown: &mut broadcast::Receiver<()>,
) -> bool {
    let mut delay = 1;
    loop {
        // Attempt the observed sample even if stop is pending; the supervisor
        // bounds a stuck write with its five-second shutdown deadline.
        let result = charge_capture::save_joint(conn, capture, charges).await;
        match result {
            Ok(()) => return true,
            Err(error) => {
                tracing::error!(%vehicle_id, %error, "collector checkpoint failed; retaining state and retrying");
                trip_capture::record_health(pool, vehicle_id, Some("checkpoint_retry_pending"))
                    .await;
            }
        }
        tokio::select! {
            biased;
            _ = shutdown.recv() => return false,
            _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
        }
        delay = (delay * 2).min(30);
        if conn.ping().await.is_err() {
            let replacement = tokio::select! {
                biased;
                _ = shutdown.recv() => return false,
                result = acquire_collector_lock(pool, vehicle_id) => result,
            };
            match replacement {
                Ok(Some(mut replacement)) => {
                    if let Err(error) = capture.verify_reacquired_lock(&mut replacement).await {
                        tracing::error!(%vehicle_id, %error, "collector recovery ownership changed");
                        return false;
                    }
                    if let Err(error) = charges.verify_reacquired_lock(&mut replacement).await {
                        tracing::error!(%vehicle_id, %error, "charge recovery ownership changed");
                        return false;
                    }
                    *conn = replacement;
                }
                Ok(None) => return false,
                Err(_) => continue,
            }
        }
    }
}

/// Lifecycle context shared with isolated enrichment consumers.  Canonical
/// telemetry is the only producer; consumers may associate samples but never
/// create or close a session.
#[derive(Debug, Clone, Default)]
pub struct ActiveSessionContext {
    pub session_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyChargingSessionClass {
    Null,
    Missing,
    Malformed,
    AllNull,
    Meaningful,
}

impl LegacyChargingSessionClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Missing => "missing",
            Self::Malformed => "malformed",
            Self::AllNull => "all_null",
            Self::Meaningful => "meaningful",
        }
    }
}

/// Classify legacy frames without treating empty frames as fresh data.
pub fn classify_legacy_charging_session(
    value: Option<&serde_json::Value>,
) -> LegacyChargingSessionClass {
    let Some(value) = value else {
        return LegacyChargingSessionClass::Missing;
    };
    if value.is_null() {
        return LegacyChargingSessionClass::Null;
    }
    let Some(object) = value.as_object() else {
        return LegacyChargingSessionClass::Malformed;
    };
    if object.is_empty() {
        return LegacyChargingSessionClass::AllNull;
    }
    if object.values().all(serde_json::Value::is_null) {
        LegacyChargingSessionClass::AllNull
    } else {
        LegacyChargingSessionClass::Meaningful
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ActiveChargeDetectorRow {
    session_id: Uuid,
    started_at: DateTime<Utc>,
    last_charge_ts: DateTime<Utc>,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
    soc_start: Option<f64>,
    last_soc: Option<f64>,
    charge_limit: Option<f64>,
    battery_capacity_wh: Option<f64>,
    power_samples_ts: Vec<DateTime<Utc>>,
    power_samples_kw: Vec<Option<f64>>,
}

#[derive(Debug, Clone)]
struct ActiveStatePeriod {
    id: i64,
    state: VehicleState,
    started_at: DateTime<Utc>,
}

pub async fn run_vehicle_worker(
    vehicle_id: Uuid,
    pool: PgPool,
    redis: redis::Client,
    age_key: String,
    config: Config,
    shutdown: broadcast::Receiver<()>,
) {
    tracing::info!(vehicle_id = %vehicle_id, "worker starting");
    let mut worker_shutdown = shutdown.resubscribe();

    let mut lock_conn = match acquire_collector_lock(&pool, vehicle_id).await {
        Ok(Some(conn)) => conn,
        Ok(None) => {
            increment_counter(&pool, vehicle_id, "collector_lock_skips").await;
            tracing::info!(vehicle_id = %vehicle_id, "collector lock held; worker staying passive");
            return;
        }
        Err(e) => {
            tracing::error!(vehicle_id=%vehicle_id, err=%e, "collector lock failed");
            return;
        }
    };

    let mut trip_capture = match trip_capture::TripCapture::load(&pool, vehicle_id).await {
        Ok(capture) => capture,
        Err(error) => {
            tracing::error!(%vehicle_id, %error, "trip checkpoint recovery failed; collector not started");
            trip_capture::record_health(&pool, vehicle_id, Some("checkpoint_recovery_failed"))
                .await;
            return;
        }
    };
    trip_capture.recover_stale(Utc::now());
    let charge_capture = match charge_capture::initialize(
        &mut lock_conn,
        &mut trip_capture,
        vehicle_id,
    )
    .await
    {
        Ok(capture) => capture,
        Err(error) => {
            tracing::error!(%vehicle_id, %error, "collector checkpoint initialization failed; recovery state retained");
            return;
        }
    };
    for _ in 0..16 {
        match charge_capture::drain_one(&pool, vehicle_id).await {
            Ok(true) => continue,
            Ok(false) => break,
            Err(error) => {
                tracing::error!(%vehicle_id, %error, "startup charge completion failed; durable retry retained");
                break;
            }
        }
    }
    // Persist already captured trips even when credentials or Redis are unavailable.
    for _ in 0..16 {
        match trip_capture::drain_one(&pool, vehicle_id).await {
            Ok(true) => continue,
            Ok(false) => break,
            Err(error) => {
                tracing::error!(%vehicle_id, %error, "startup trip completion failed; durable retry retained");
                trip_capture::record_health(&pool, vehicle_id, Some("completion_retry_pending"))
                    .await;
                break;
            }
        }
    }

    let identity = match age_key.parse::<age::x25519::Identity>() {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(vehicle_id=%vehicle_id, err=%e, "bad age key");
            return;
        }
    };

    // Load credentials
    let creds_row = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_optional(&pool)
    .await;

    let _tokens: RivianTokenBundle = match creds_row {
        Ok(Some(encrypted_tokens)) => match decrypt_tokens(&encrypted_tokens, &identity) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(vehicle_id=%vehicle_id, err=%e, "decrypt failed");
                upsert_health(
                    &pool,
                    vehicle_id,
                    false,
                    "error",
                    &e.to_string(),
                    Some("needs_reauth"),
                    Some("credentials_invalid"),
                )
                .await;
                return;
            }
        },
        Ok(None) => {
            tracing::error!(
                vehicle_id=%vehicle_id,
                worker_health="error",
                auth_state="needs_reauth",
                reason_code="credentials_missing",
                action="reconnect_vehicle",
                "Rivian telemetry feed unavailable: credentials are missing"
            );
            upsert_health(
                &pool,
                vehicle_id,
                false,
                "error",
                "Rivian credentials are missing; reconnect the vehicle",
                Some("needs_reauth"),
                Some("credentials_missing"),
            )
            .await;
            return;
        }
        Err(e) => {
            tracing::error!(vehicle_id=%vehicle_id, err=%e, "db error");
            return;
        }
    };

    upsert_health(
        &pool,
        vehicle_id,
        false,
        "starting",
        "Initializing Rivian telemetry collector",
        Some("authorized"),
        None,
    )
    .await;

    let (ev_tx, mut ev_rx) = mpsc::channel::<WsInboundEvent>(256);
    // Keep Parallax delivery bounded and independent from the legacy socket.
    let (parallax_tx, mut parallax_rx) = mpsc::channel::<(String, TelemetryEvent)>(256);

    // Get rivian_vehicle_id
    let riv_id: Option<String> =
        sqlx::query_scalar("SELECT rivian_vehicle_id FROM panda.vehicles WHERE id = $1")
            .bind(vehicle_id)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

    let rivian_vehicle_id = match riv_id {
        Some(id) => id,
        None => {
            tracing::error!(vehicle_id=%vehicle_id, "no rivian_vehicle_id");
            let _ = release_collector_lock(&mut lock_conn, vehicle_id).await;
            return;
        }
    };
    // Fetch owner user_id (needed for wallbox enrichment).
    let user_id: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM panda.vehicles WHERE id = $1")
            .bind(vehicle_id)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

    let redis_conn = match redis.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(err=%e, "redis connect failed");
            upsert_health(
                &pool,
                vehicle_id,
                false,
                "error",
                "Telemetry collector could not connect to Redis",
                Some("authorized"),
                None,
            )
            .await;
            let _ = release_collector_lock(&mut lock_conn, vehicle_id).await;
            return;
        }
    };

    let mut companions = tokio::task::JoinSet::new();
    let spawn_ws_loop = || {
        let ws_shutdown = shutdown.resubscribe();
        let ev_tx_ws = ev_tx.clone();
        let riv_id_clone = rivian_vehicle_id.clone();
        let ws_config = config.clone();
        let ws_pool = pool.clone();
        let ws_age_key = age_key.clone();
        WorkerTask(tokio::spawn(async move {
            ws_client::run_ws_loop(
                vehicle_id,
                riv_id_clone,
                ws_pool,
                ws_age_key,
                ev_tx_ws,
                ws_shutdown,
                ws_config,
            )
            .await;
        }))
    };
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // ── Poll tasks ───────────────────────────────────────────────────────────
    // Channel to push PowerState changes from the main event loop to the poll loop.
    let (power_state_tx, power_state_rx) = watch::channel::<Option<PowerState>>(None);
    let (charging_tx, charging_rx) = watch::channel(false);
    let (active_session_tx, active_session_rx) =
        watch::channel::<ActiveSessionContext>(ActiveSessionContext::default());

    // Repair only the bounded active tail before detector rehydration.  The
    // service is transactional and journal-idempotent; broad historical
    // canonicalization is intentionally not part of worker startup.
    if let Err(error) =
        crate::services::charge_session_repair::heal_active_tail(&pool, vehicle_id).await
    {
        tracing::warn!(vehicle_id=%vehicle_id, err=%error, "active charge-session startup healing failed");
    }

    let active_session_started_at = charge_capture
        .detector()
        .active_snapshot()
        .map(|s| s.started_at);
    let _ = active_session_tx.send(ActiveSessionContext {
        session_id: charge_capture.detector().active_session_id(),
        started_at: active_session_started_at,
        ended_at: None,
    });

    // Historical association is deliberately detached and chunked. It may
    // enrich old graph samples, but cannot delay or drive canonical ingestion.
    {
        let pool2 = pool.clone();
        companions.spawn(async move {
            for _ in 0..10 {
                match crate::services::charge_session_repair::reconcile_unassociated_parallax_samples(
                    &pool2, vehicle_id, 250,
                )
                .await
                {
                    Ok(stats) if stats.scanned == 250 => continue,
                    Ok(_) => break,
                    Err(error) => {
                        tracing::warn!(vehicle_id=%vehicle_id, err=%error, "historical Parallax session reconciliation failed");
                        break;
                    }
                }
            }
        });
    }

    // Start all acquisition companions only after repair and detector
    // rehydration have published the canonical active-session context.
    {
        let uid = user_id.unwrap_or(Uuid::nil());
        let pool2 = pool.clone();
        let client2 = http_client.clone();
        let age_key2 = age_key.clone();
        let baseline_tx = ev_tx.clone();
        companions.spawn(async move {
            rivian_poll::run_startup_polls(vehicle_id, uid, pool2, client2, age_key2, baseline_tx)
                .await;
        });
    }
    {
        let pool2 = pool.clone();
        let client2 = http_client.clone();
        let redis2 = redis.clone();
        let age_key2 = age_key.clone();
        let poll_shutdown = shutdown.resubscribe();
        companions.spawn(async move {
            rivian_poll::run_poll_loop(
                vehicle_id,
                pool2,
                client2,
                age_key2,
                rivian_poll::PollLoopSignals {
                    power_state_rx,
                    charging_rx,
                    shutdown: poll_shutdown,
                },
                redis2,
            )
            .await;
        });
    }
    let mut parallax_handle = if parallax_enabled() {
        Some(WorkerTask(crate::parallax::spawn_in_process(
            pool.clone(),
            vehicle_id,
            rivian_vehicle_id.clone(),
            age_key.clone(),
            parallax_tx,
            active_session_rx,
            shutdown.resubscribe(),
        )))
    } else {
        let _ = sqlx::query(
            "INSERT INTO panda.parallax_collector_state(vehicle_id,status,last_error,schema_version) VALUES($1,'disabled','PARALLAX_ENABLED=false',1) ON CONFLICT(vehicle_id) DO UPDATE SET status='disabled',last_error='PARALLAX_ENABLED=false',updated_at=now()",
        )
        .bind(vehicle_id)
        .execute(&pool)
        .await;
        None
    };
    // ────────────────────────────────────────────────────────────────────────

    // Rehydrate the detector before opening the socket.  This ordering is
    // important on restart: the first frame must be evaluated against the
    // existing session UUID instead of racing startup with a fresh detector.
    let mut ws_handle = spawn_ws_loop();
    let mut ws_started_at = tokio::time::Instant::now();
    let mut ws_exit_backoff = 1;
    // State periods are durable state, not a best-effort in-memory overlay.
    // Rehydrate before processing any new payload so a worker restart can close
    // the existing period instead of colliding with the partial unique index.
    let active_state_period = match reconcile_state_period_tail(&pool, vehicle_id, None).await {
        Ok(period) => period,
        Err(error) => {
            tracing::warn!(vehicle_id=%vehicle_id, err=%error, "state-period startup reconciliation failed; will retry during ingestion");
            match load_open_state_period(&pool, vehicle_id).await {
                Ok(period) => period,
                Err(error) => {
                    tracing::warn!(vehicle_id=%vehicle_id, err=%error, "state-period startup rehydration failed");
                    None
                }
            }
        }
    };
    let mut pipeline = durable_processing::CollectorPipeline::new(
        durable_processing::CollectorIo {
            vehicle_id,
            pool: pool.clone(),
            config: config.clone(),
            redis: redis_conn,
            power_state: power_state_tx,
            charging: charging_tx,
            active_session: active_session_tx,
        },
        trip_capture,
        charge_capture,
        active_state_period,
    );
    let mut last_ws_inbound_at = tokio::time::Instant::now();
    let mut state_reconcile_interval = tokio::time::interval(STATE_PERIOD_RECONCILE_INTERVAL);
    state_reconcile_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Startup reconciliation above already covered the current tail.
    state_reconcile_interval.tick().await;
    let mut trip_retry_interval = tokio::time::interval(std::time::Duration::from_secs(15));
    trip_retry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        while companions.try_join_next().is_some() {}
        let inbound = tokio::select! {
            _ = worker_shutdown.recv() => {
                break;
            }
            _ = trip_retry_interval.tick() => {
                pipeline.retry_completion().await;
                continue;
            }
            _ = state_reconcile_interval.tick() => {
                pipeline.reconcile().await;
                continue;
            }
            _ = tokio::time::sleep_until(last_ws_inbound_at + WS_WATCHDOG_TIMEOUT) => {
                tracing::warn!(
                    vehicle_id=%vehicle_id,
                    timeout_seconds=WS_WATCHDOG_TIMEOUT.as_secs(),
                    "worker watchdog detected a silent Rivian WS stream; restarting websocket collector"
                );
                upsert_health(
                    &pool,
                    vehicle_id,
                    false,
                    "stale",
                    "No Rivian WS messages received within the watchdog window; restarting websocket collector",
                    None,
                    Some("rivian_ws_silent"),
                )
                .await;
                ws_handle.abort();
                let _ = (&mut ws_handle).await;
                ws_handle = spawn_ws_loop();
                ws_started_at = tokio::time::Instant::now();
                last_ws_inbound_at = tokio::time::Instant::now();
                continue;
            }
            // Normal event path.
            msg = ev_rx.recv() => match msg {
                Some(ev) => ev,
                None => break,
            },
            parallax_event = parallax_rx.recv(), if parallax_handle.is_some() && !parallax_rx.is_closed() => match parallax_event {
                Some((topic, event)) => WsInboundEvent {
                    kind: WsInboundKind::Telemetry,
                    received_at: Utc::now(),
                    raw: String::new(),
                    message_type: Some(format!("parallax:{topic}")),
                    telemetry: Some(event),
                    charging_session: None,
                    battery_cell_type: None,
                },
                None => continue,
            },
            // Monitor the WS task; log if it exits unexpectedly.
            result = &mut ws_handle => {
                match result {
                    Ok(()) => tracing::warn!(vehicle_id=%vehicle_id, "WS task exited cleanly; restarting websocket collector"),
                    Err(e) if e.is_panic() => tracing::error!(vehicle_id=%vehicle_id, "WS task panicked; restarting websocket collector"),
                    Err(e) => tracing::warn!(vehicle_id=%vehicle_id, err=%e, "WS task exited with error; restarting websocket collector"),
                }
                upsert_health(
                    &pool,
                    vehicle_id,
                    false,
                    "degraded",
                    "Rivian websocket collector exited unexpectedly; restarting",
                    None,
                    Some("rivian_ws_restarting"),
                )
                .await;
                if ws_started_at.elapsed() >= std::time::Duration::from_secs(300) {
                    ws_exit_backoff = 1;
                }
                tokio::select! {
                    biased;
                    _ = worker_shutdown.recv() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(ws_exit_backoff)) => {}
                }
                ws_exit_backoff = (ws_exit_backoff * 2).min(60);
                ws_handle = spawn_ws_loop();
                ws_started_at = tokio::time::Instant::now();
                last_ws_inbound_at = tokio::time::Instant::now();
                continue;
            }
        };
        match pipeline
            .process(
                inbound,
                &mut lock_conn,
                &mut worker_shutdown,
                &mut last_ws_inbound_at,
            )
            .await
        {
            durable_processing::ProcessOutcome::Continue => {}
            durable_processing::ProcessOutcome::CheckpointUnavailable => break,
            durable_processing::ProcessOutcome::ChargeEnded(session) => {
                // Trigger incremental charge history sync to enrich the new session
                // with Rivian API fields (network vendor, range added, etc.).
                let pool2 = pool.clone();
                let client2 = http_client.clone();
                let age_key2 = age_key.clone();
                companions.spawn(async move {
                if let Err(e) = rivian_poll::fetch_live_session_history_for_vehicle(
                    vehicle_id,
                    Some(session.session_id),
                    &pool2,
                    &client2,
                    &age_key2,
                )
                .await
                {
                    tracing::debug!(vehicle_id=%vehicle_id, err=%e, "live charge history sync failed");
                }
                if let Err(e) = rivian_poll::fetch_charge_history_for_vehicle(
                    vehicle_id, &pool2, &client2, &age_key2,
                )
                .await
                {
                    tracing::warn!(vehicle_id=%vehicle_id, err=%e, "post-session charge history sync failed");
                }
            });
            }
        }
    }

    pipeline.flush().await;
    ws_handle.abort();
    let _ = (&mut ws_handle).await;
    if let Some(mut handle) = parallax_handle.take() {
        tokio::select! {
            _ = &mut handle => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                handle.abort();
                let _ = handle.await;
            }
        }
    }
    companions.abort_all();
    while companions.join_next().await.is_some() {}
    let _ = release_collector_lock(&mut lock_conn, vehicle_id).await;
}

async fn flush_legacy_charging_diagnostics(
    pool: &PgPool,
    vehicle_id: Uuid,
    counts: &mut [i64; 5],
    last_class: Option<LegacyChargingSessionClass>,
    last_frame_at: Option<DateTime<Utc>>,
    last_meaningful_at: Option<DateTime<Utc>>,
) {
    if counts.iter().all(|count| *count == 0) {
        return;
    }
    let result = sqlx::query(
        r#"INSERT INTO panda.parallax_collector_state (
               vehicle_id,status,schema_version,legacy_last_frame_at,
               legacy_last_meaningful_frame_at,legacy_last_classification,
               legacy_null_count,legacy_missing_count,legacy_malformed_count,
               legacy_all_null_count,legacy_meaningful_count)
           VALUES ($1,'starting',1,$2,$3,$4,$5,$6,$7,$8,$9)
           ON CONFLICT(vehicle_id) DO UPDATE SET
               legacy_last_frame_at=GREATEST(panda.parallax_collector_state.legacy_last_frame_at,EXCLUDED.legacy_last_frame_at),
               legacy_last_meaningful_frame_at=GREATEST(panda.parallax_collector_state.legacy_last_meaningful_frame_at,EXCLUDED.legacy_last_meaningful_frame_at),
               legacy_last_classification=EXCLUDED.legacy_last_classification,
               legacy_null_count=panda.parallax_collector_state.legacy_null_count+EXCLUDED.legacy_null_count,
               legacy_missing_count=panda.parallax_collector_state.legacy_missing_count+EXCLUDED.legacy_missing_count,
               legacy_malformed_count=panda.parallax_collector_state.legacy_malformed_count+EXCLUDED.legacy_malformed_count,
               legacy_all_null_count=panda.parallax_collector_state.legacy_all_null_count+EXCLUDED.legacy_all_null_count,
               legacy_meaningful_count=panda.parallax_collector_state.legacy_meaningful_count+EXCLUDED.legacy_meaningful_count"#,
    ).bind(vehicle_id).bind(last_frame_at).bind(last_meaningful_at)
     .bind(last_class.map(LegacyChargingSessionClass::as_str))
     .bind(counts[0]).bind(counts[1]).bind(counts[2]).bind(counts[3]).bind(counts[4])
     .execute(pool).await;
    if let Err(error) = result {
        tracing::debug!(vehicle_id=%vehicle_id, err=%error, "legacy charging diagnostics flush failed");
    } else {
        *counts = [0; 5];
    }
}

fn parallax_enabled() -> bool {
    !matches!(
        std::env::var("PARALLAX_ENABLED")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("false" | "0" | "no" | "off")
    )
}

async fn upsert_battery_cell_type(
    pool: &PgPool,
    vehicle_id: Uuid,
    battery_cell_type: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE panda.vehicles
         SET battery_cell_type = COALESCE($2, battery_cell_type),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(vehicle_id)
    .bind(battery_cell_type)
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_active_charge_session(
    pool: &PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    event: &TelemetryEvent,
) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    ensure_active_charge_session_on(&mut conn, vehicle_id, session_id, event).await
}

async fn ensure_active_charge_session_on(
    conn: &mut PgConnection,
    vehicle_id: Uuid,
    session_id: Uuid,
    event: &TelemetryEvent,
) -> anyhow::Result<()> {
    let location = match (event.latitude, event.longitude) {
        (Some(latitude), Some(longitude))
            if latitude.is_finite()
                && longitude.is_finite()
                && (-90.0..=90.0).contains(&latitude)
                && (-180.0..=180.0).contains(&longitude)
                && !(latitude == 0.0 && longitude == 0.0) =>
        {
            Some((latitude, longitude))
        }
        _ => None,
    };
    let saved: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO panda.charge_sessions
           (id, vehicle_id, started_at,
            location_lat, location_lng, source_location_lat, source_location_lng,
            soc_start, charge_limit, source, data_confidence)
           SELECT $1, $2,
                  COALESCE(MIN(t.ts), $3),
                  $6, $7, $6, $7,
                  COALESCE(
                      (ARRAY_AGG(t.battery_level ORDER BY t.ts)
                       FILTER (WHERE t.battery_level IS NOT NULL))[1],
                      $4
                  ),
                  COALESCE(MAX(t.battery_limit), $5),
                  'telemetry',
                  'telemetry'
           FROM timeseries.telemetry t
           WHERE t.vehicle_id = $2
             AND t.charge_session_id = $1
           ON CONFLICT (id) DO UPDATE SET
             started_at = COALESCE(panda.charge_sessions.started_at, EXCLUDED.started_at),
             location_lat = COALESCE(panda.charge_sessions.location_lat, EXCLUDED.location_lat),
             location_lng = COALESCE(panda.charge_sessions.location_lng, EXCLUDED.location_lng),
             source_location_lat = COALESCE(panda.charge_sessions.source_location_lat, EXCLUDED.source_location_lat),
             source_location_lng = COALESCE(panda.charge_sessions.source_location_lng, EXCLUDED.source_location_lng),
             soc_start = COALESCE(panda.charge_sessions.soc_start, EXCLUDED.soc_start),
             charge_limit = COALESCE(panda.charge_sessions.charge_limit, EXCLUDED.charge_limit)
           WHERE panda.charge_sessions.vehicle_id=EXCLUDED.vehicle_id RETURNING id"#,
    )
    .bind(session_id)
    .bind(vehicle_id)
    .bind(event.ts)
    .bind(event.battery_level)
    .bind(event.battery_limit)
    .bind(location.map(|(latitude, _)| latitude))
    .bind(location.map(|(_, longitude)| longitude))
    .fetch_optional(conn)
    .await?;
    anyhow::ensure!(
        saved == Some(session_id),
        "active charge session ownership mismatch"
    );
    Ok(())
}

async fn apply_charging_session_update(
    pool: &PgPool,
    redis_conn: &mut redis::aio::MultiplexedConnection,
    vehicle_id: Uuid,
    event: &parser::ChargingSessionEvent,
    received_at: DateTime<Utc>,
    active_session_id: Option<Uuid>,
) {
    let live_key = rivian_poll::live_session_redis_key(vehicle_id);
    let live = rivian_poll::normalize_charging_session(event, received_at);

    match live {
        Some(live) if active_session_id.is_some() => {
            if let Ok(json) = serde_json::to_string(&live) {
                if let Err(error) = redis_conn
                    .set_ex::<_, _, ()>(&live_key, json, rivian_poll::LIVE_SESSION_TTL_SECONDS)
                    .await
                {
                    tracing::warn!(vehicle_id=%vehicle_id, err=%error, "live session Redis write failed");
                }
            }
            if let Err(error) =
                rivian_poll::persist_live_session_data(pool, active_session_id, &live).await
            {
                tracing::warn!(vehicle_id=%vehicle_id, err=%error, "live session database update failed");
            }
        }
        Some(_) => {
            // A late legacy frame cannot recreate state after canonical
            // vehicleState ended the session.
            if let Err(error) = redis_conn.del::<_, ()>(&live_key).await {
                tracing::debug!(vehicle_id=%vehicle_id, err=%error, "late live session Redis cleanup failed");
            }
        }
        None if should_clear_live_session(event) => {
            // An explicit root null means the legacy projection is no longer
            // meaningful. Never leave a stale Redis snapshot visible.
            if let Err(error) = redis_conn.del::<_, ()>(&live_key).await {
                tracing::debug!(vehicle_id=%vehicle_id, err=%error, "null live session Redis cleanup failed");
            }
        }
        None => {
            // An all-null object or liveData:null is only an empty partial
            // update. Preserve the existing TTL snapshot; a later meaningful
            // frame can refresh it, while natural expiry handles staleness.
        }
    }

    if let Err(error) = rivian_poll::persist_charging_session_chart(
        pool,
        vehicle_id,
        active_session_id,
        &event.chart_data,
    )
    .await
    {
        tracing::warn!(vehicle_id=%vehicle_id, err=%error, "live charge curve update failed");
    }
}

fn should_clear_live_session(event: &parser::ChargingSessionEvent) -> bool {
    event.explicit_null
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeenKind {
    Payload,
    Heartbeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersistenceDecision {
    Persist,
    Suppress(SuppressionReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuppressionReason {
    Empty,
    Duplicate,
    Threshold,
}

impl SuppressionReason {
    fn counter_column(self) -> &'static str {
        match self {
            Self::Empty => "telemetry_suppressed_empty",
            Self::Duplicate => "telemetry_suppressed_duplicate",
            Self::Threshold => "telemetry_suppressed_threshold",
        }
    }
}

fn blank_event(vehicle_id: Uuid, ts: DateTime<Utc>) -> TelemetryEvent {
    TelemetryEvent::empty(vehicle_id, ts)
}

fn changed_f64(current: Option<f64>, previous: Option<f64>, threshold: f64) -> bool {
    match (current, previous) {
        (Some(c), Some(p)) => (c - p).abs() >= threshold,
        (Some(_), None) => true,
        _ => false,
    }
}

fn location_moved_m(
    previous: Option<(f64, f64)>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> Option<f64> {
    let (prev_lat, prev_lon) = previous?;
    let (lat, lon) = latitude.zip(longitude)?;
    Some(haversine_m(prev_lat, prev_lon, lat, lon))
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let radius_m = 6_371_000.0_f64;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * radius_m * a.sqrt().asin()
}

/// Batches stewardship counter increments and flushes them in a single upsert.
struct CounterBatch {
    vehicle_id: Uuid,
    counts: std::collections::HashMap<&'static str, i64>,
}

impl CounterBatch {
    fn new(vehicle_id: Uuid) -> Self {
        Self {
            vehicle_id,
            counts: std::collections::HashMap::new(),
        }
    }

    fn increment(&mut self, column: &str) {
        if let Some(col) = stewardship_counter_column(column) {
            *self.counts.entry(col).or_insert(0) += 1;
        }
    }

    /// Flush accumulated counts to the DB and reset the batch for reuse.
    async fn flush(&mut self, pool: &PgPool) {
        if self.counts.is_empty() {
            return;
        }
        let cols: Vec<&'static str> = self.counts.keys().copied().collect();
        let set_clause = cols
            .iter()
            .map(|c| format!("{c} = panda.rivian_stewardship_counters.{c} + EXCLUDED.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let col_list = cols.join(", ");
        let placeholders: Vec<String> = cols
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", i + 3))
            .collect();
        let sql = format!(
            "INSERT INTO panda.rivian_stewardship_counters (vehicle_id, day, {col_list}) \
             VALUES ($1, CURRENT_DATE, {vals}) \
             ON CONFLICT (vehicle_id, day) DO UPDATE SET {set_clause}, updated_at = now()",
            vals = placeholders.join(", "),
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(self.vehicle_id)
            .bind(chrono::Utc::now().date_naive());
        for col in &cols {
            q = q.bind(*self.counts.get(col).unwrap_or(&0));
        }
        let _ = q.execute(pool).await;
        self.counts.clear();
    }
}

/// Accumulate per-event counters into `batch`. The caller is responsible for
/// flushing `batch` to the DB periodically (every N events) to amortize the
/// cost of the upsert across many messages.
async fn handle_inbound_accounting(
    pool: &PgPool,
    vehicle_id: Uuid,
    config: &Config,
    inbound: &WsInboundEvent,
    batch: &mut CounterBatch,
) {
    if !is_synthetic_control(inbound.message_type.as_deref()) {
        batch.increment("ws_messages_received");
        match inbound.kind {
            WsInboundKind::Control => {
                batch.increment("ws_control_messages_received");
            }
            WsInboundKind::Heartbeat => {
                batch.increment("ws_heartbeats_received");
            }
            WsInboundKind::Telemetry => {
                batch.increment("ws_payload_messages_received");
            }
            WsInboundKind::ChargingSession => {
                batch.increment("ws_payload_messages_received");
            }
        }
    }
    match inbound.message_type.as_deref() {
        Some("connection_ack") => {
            batch.increment("ws_connections_opened");
            // The account card reflects a successful authenticated Rivian
            // session, rather than only the time credentials were saved.
            crate::services::external_connections::record_attempt(
                pool,
                crate::services::external_connections::RIVIAN_ACCOUNT,
            )
            .await;
            crate::services::external_connections::record_success(
                pool,
                crate::services::external_connections::RIVIAN_ACCOUNT,
            )
            .await;
        }
        Some("reconnect") => batch.increment("ws_reconnects"),
        Some("connection_init" | "subscribe") => {
            batch.increment("outbound_messages_sent");
        }
        _ => {}
    }
    if let Some(update) = runtime_health_update_for_ws_control(inbound) {
        if matches!(
            inbound.message_type.as_deref(),
            Some("ws_handshake_rejected" | "ws_schema_rejected")
        ) {
            crate::services::external_connections::record_failure(
                pool,
                crate::services::external_connections::RIVIAN_ACCOUNT,
                &update.worker_health_msg,
            )
            .await;
        }
        upsert_health(
            pool,
            vehicle_id,
            update.online,
            update.worker_health,
            &update.worker_health_msg,
            Some(update.auth_state),
            update.auth_reason_code,
        )
        .await;
    }
    if config.rivian_persist_raw_events {
        persist_raw_event(pool, batch, vehicle_id, inbound).await;
    }
}

async fn persist_raw_event(
    pool: &PgPool,
    batch: &mut CounterBatch,
    vehicle_id: Uuid,
    inbound: &WsInboundEvent,
) {
    let payload_json = serde_json::from_str::<serde_json::Value>(&inbound.raw).ok();
    let result = sqlx::query(
        r#"
        INSERT INTO panda.rivian_ws_raw_events
          (vehicle_id, received_at, event_type, message_type, payload_json, payload_text)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(vehicle_id)
    .bind(inbound.received_at)
    .bind(inbound.kind.as_str())
    .bind(inbound.message_type.as_deref())
    .bind(payload_json)
    .bind(&inbound.raw)
    .execute(pool)
    .await;

    if result.is_ok() {
        batch.increment("raw_events_persisted");
    }
}

async fn cleanup_raw_events(pool: &PgPool, retention_days: i64) {
    // Use a non-blocking advisory lock (id 0x726d_5241 = "rmRA") so that
    // concurrent workers skip the DELETE rather than pile up on the same rows.
    let got_lock: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(0x726d5241::bigint)")
        .fetch_one(pool)
        .await
        .unwrap_or(false);
    if !got_lock {
        return;
    }

    let _ = sqlx::query(
        "DELETE FROM panda.rivian_ws_raw_events WHERE received_at < now() - ($1::int * INTERVAL '1 day')",
    )
    .bind(retention_days.max(1) as i32)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        "DELETE FROM panda.rivian_parallax_events WHERE received_at < now() - ($1::int * INTERVAL '1 day')",
    )
    .bind(retention_days.max(1) as i32)
    .execute(pool)
    .await;

    // Release the session-level lock immediately so other workers can take it next cycle.
    let _ = sqlx::query("SELECT pg_advisory_unlock(0x726d5241::bigint)")
        .execute(pool)
        .await;
}

async fn increment_counter(pool: &PgPool, vehicle_id: Uuid, column: &str) {
    let Some(column) = stewardship_counter_column(column) else {
        return;
    };
    let sql = format!(
        "INSERT INTO panda.rivian_stewardship_counters (vehicle_id, day, {column}) \
         VALUES ($1, CURRENT_DATE, 1) \
         ON CONFLICT (vehicle_id, day) DO UPDATE \
         SET {column} = panda.rivian_stewardship_counters.{column} + 1, updated_at = now()"
    );
    let _ = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(vehicle_id)
        .execute(pool)
        .await;
}

fn stewardship_counter_column(column: &str) -> Option<&'static str> {
    match column {
        "ws_messages_received" => Some("ws_messages_received"),
        "ws_heartbeats_received" => Some("ws_heartbeats_received"),
        "ws_payload_messages_received" => Some("ws_payload_messages_received"),
        "ws_control_messages_received" => Some("ws_control_messages_received"),
        "ws_connections_opened" => Some("ws_connections_opened"),
        "ws_reconnects" => Some("ws_reconnects"),
        "outbound_messages_sent" => Some("outbound_messages_sent"),
        "outbound_graphql_requests" => Some("outbound_graphql_requests"),
        "telemetry_writes_persisted" => Some("telemetry_writes_persisted"),
        "telemetry_writes_suppressed" => Some("telemetry_writes_suppressed"),
        "telemetry_suppressed_duplicate" => Some("telemetry_suppressed_duplicate"),
        "telemetry_suppressed_empty" => Some("telemetry_suppressed_empty"),
        "telemetry_suppressed_threshold" => Some("telemetry_suppressed_threshold"),
        "collector_lock_skips" => Some("collector_lock_skips"),
        "raw_events_persisted" => Some("raw_events_persisted"),
        _ => None,
    }
}

fn collector_lock_key(vehicle_id: Uuid) -> i64 {
    let bytes = vehicle_id.as_bytes();
    let mut first = [0u8; 8];
    let mut second = [0u8; 8];
    first.copy_from_slice(&bytes[0..8]);
    second.copy_from_slice(&bytes[8..16]);
    i64::from_be_bytes(first) ^ i64::from_be_bytes(second) ^ ADVISORY_LOCK_NAMESPACE
}

#[cfg(test)]
async fn load_active_charge_snapshot(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> Option<ActiveChargeSessionSnapshot> {
    load_active_charge_snapshot_result(pool, vehicle_id)
        .await
        .unwrap()
}

#[cfg(test)]
async fn load_active_charge_snapshot_result(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> anyhow::Result<Option<ActiveChargeSessionSnapshot>> {
    let mut conn = pool.acquire().await?;
    load_active_charge_snapshot_on(&mut conn, vehicle_id).await
}

async fn load_active_charge_snapshot_on(
    conn: &mut PgConnection,
    vehicle_id: Uuid,
) -> anyhow::Result<Option<ActiveChargeSessionSnapshot>> {
    let row = sqlx::query_as::<_, ActiveChargeDetectorRow>(
        r#"
        WITH active_samples AS (
            SELECT
                t.charge_session_id,
                t.ts,
                t.latitude,
                t.longitude,
                t.battery_level,
                t.battery_capacity_wh,
                t.battery_limit,
                t.power_kw
            FROM timeseries.telemetry t
            LEFT JOIN panda.charge_sessions cs
              ON cs.id = t.charge_session_id
            WHERE t.vehicle_id = $1
              AND t.charge_session_id IS NOT NULL
              -- Both materialized and not-yet-materialized sessions are
              -- eligible.  The old cs.id IS NULL predicate discarded the
              -- exact row ensure_active_charge_session had just created.
              AND (cs.id IS NULL OR cs.ended_at IS NULL)
              AND t.ts >= now() - ($2::int * interval '1 hour')
        ),
        candidate_sessions AS (
            SELECT
                charge_session_id AS session_id,
                MIN(ts) AS started_at,
                MAX(ts) AS last_charge_ts,
                (ARRAY_AGG(latitude ORDER BY ts) FILTER (
                    WHERE latitude IS NOT NULL
                      AND longitude IS NOT NULL
                      AND NOT (latitude = 0 AND longitude = 0)
                ))[1] AS location_lat,
                (ARRAY_AGG(longitude ORDER BY ts) FILTER (
                    WHERE latitude IS NOT NULL
                      AND longitude IS NOT NULL
                      AND NOT (latitude = 0 AND longitude = 0)
                ))[1] AS location_lng,
                (ARRAY_AGG(battery_level ORDER BY ts) FILTER (WHERE battery_level IS NOT NULL))[1] AS soc_start,
                (ARRAY_AGG(battery_level ORDER BY ts DESC) FILTER (WHERE battery_level IS NOT NULL))[1] AS last_soc,
                MAX(battery_limit) AS charge_limit,
                (ARRAY_AGG(battery_capacity_wh ORDER BY ts DESC) FILTER (WHERE battery_capacity_wh IS NOT NULL))[1] AS battery_capacity_wh,
                ARRAY_AGG(ts ORDER BY ts) AS power_samples_ts,
                ARRAY_AGG(power_kw ORDER BY ts) AS power_samples_kw
            FROM active_samples
            GROUP BY charge_session_id
        )
        SELECT
            session_id,
            started_at,
            last_charge_ts,
            location_lat,
            location_lng,
            soc_start,
            last_soc,
            charge_limit,
            battery_capacity_wh,
            power_samples_ts,
            power_samples_kw
        FROM candidate_sessions
        WHERE last_charge_ts >= now() - ($3::int * interval '1 minute')
        ORDER BY last_charge_ts DESC
        LIMIT 1
        "#,
    )
    .bind(vehicle_id)
    .bind(CHARGE_DETECTOR_REHYDRATE_LOOKBACK_HOURS)
    .bind(CHARGE_DETECTOR_REHYDRATE_STALENESS_MINUTES)
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let power = durable_charge_restart::ReconstructedChargePower::from_samples(
        row.started_at,
        row.power_samples_ts.into_iter().zip(row.power_samples_kw),
    );
    Ok(Some(ActiveChargeSessionSnapshot {
        session_id: row.session_id,
        started_at: row.started_at,
        location_lat: row.location_lat,
        location_lng: row.location_lng,
        soc_start: row.soc_start,
        last_soc: row.last_soc,
        charge_limit: row.charge_limit,
        battery_capacity_wh: row.battery_capacity_wh,
        last_charge_ts: row.last_charge_ts,
        last_power_ts: Some(power.last_power_ts),
        energy_used_wh: power.energy_used_wh,
        peak_charge_kw: power.peak_charge_kw,
    }))
}

async fn acquire_collector_lock(
    pool: &PgPool,
    vehicle_id: Uuid,
) -> anyhow::Result<Option<PgConnection>> {
    let mut conn = pool.acquire().await?;
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(collector_lock_key(vehicle_id))
        .fetch_one(&mut *conn)
        .await?;
    // Session locks must not return to the pool after a panic or task abort.
    Ok(acquired.then(|| conn.detach()))
}

async fn release_collector_lock(conn: &mut PgConnection, vehicle_id: Uuid) -> anyhow::Result<()> {
    let _: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(collector_lock_key(vehicle_id))
        .fetch_one(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod snapshot_tests {
    use super::build_snapshot;
    use crate::models::telemetry::{ChargerState, TelemetryEvent};
    use chrono::{TimeZone, Utc};
    use serde_json::Value;
    use uuid::Uuid;

    fn event_with_partial_fields() -> TelemetryEvent {
        TelemetryEvent {
            battery_level: Some(79.0),
            battery_level_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 12, 0, 0).unwrap()),
            battery_capacity_wh: Some(135_000.0),
            battery_limit: Some(70.0),
            battery_limit_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 12, 1, 0).unwrap()),
            charger_state: Some(ChargerState::Disconnected),
            charger_state_ts: Some(Utc.with_ymd_and_hms(2026, 6, 19, 12, 2, 0).unwrap()),
            tire_fl_psi: Some(36.5),
            ..TelemetryEvent::empty(Uuid::new_v4(), Utc::now())
        }
    }

    #[test]
    fn build_snapshot_sends_closure_motion_only_when_it_changed() {
        let event = event_with_partial_fields();
        let unchanged: Value = serde_json::from_str(&build_snapshot(&event, None)).unwrap();
        assert!(unchanged["data"].get("closure_motion").is_none());

        let motion = crate::ingestion::closure_motion::ClosureMotionMap::from([(
            "closure_frunk_closed".to_owned(),
            crate::ingestion::closure_motion::ClosureMotion::Closing,
        )]);
        let moving: Value = serde_json::from_str(&build_snapshot(&event, Some(&motion))).unwrap();
        assert_eq!(
            moving["data"]["closure_motion"],
            serde_json::json!({ "closure_frunk_closed": "closing" })
        );

        let settled: Value = serde_json::from_str(&build_snapshot(
            &event,
            Some(&crate::ingestion::closure_motion::ClosureMotionMap::new()),
        ))
        .unwrap();
        assert_eq!(settled["data"]["closure_motion"], serde_json::json!({}));
    }

    #[test]
    fn build_snapshot_omits_absent_partial_fields_instead_of_emitting_nulls() {
        let payload: Value =
            serde_json::from_str(&build_snapshot(&event_with_partial_fields(), None)).unwrap();
        let data = payload["data"].as_object().unwrap();

        assert_eq!(data.get("battery_level").unwrap(), 79.0);
        assert_eq!(
            data.get("battery_level_ts").and_then(Value::as_str),
            Some("2026-06-19T12:00:00Z")
        );
        assert_eq!(data.get("battery_capacity_kwh").unwrap(), 135.0);
        assert_eq!(data.get("battery_limit").unwrap(), 70.0);
        assert_eq!(
            data.get("battery_limit_ts").and_then(Value::as_str),
            Some("2026-06-19T12:01:00Z")
        );
        assert_eq!(data.get("charger_state").unwrap(), "disconnected");
        assert_eq!(
            data.get("charger_state_ts").and_then(Value::as_str),
            Some("2026-06-19T12:02:00Z")
        );
        assert_eq!(data.get("tire_fl_psi").unwrap(), 36.5);
        assert_eq!(data.get("is_online").unwrap(), true);
        assert!(!data.contains_key("cabin_temp_c"));
        assert!(!data.contains_key("tire_fr_psi"));
        assert!(!data.contains_key("location"));
    }

    #[test]
    fn build_snapshot_includes_location_only_when_both_coordinates_exist() {
        let mut event = event_with_partial_fields();
        event.latitude = Some(30.25);

        let payload: Value = serde_json::from_str(&build_snapshot(&event, None)).unwrap();
        assert!(!payload["data"]
            .as_object()
            .unwrap()
            .contains_key("location"));

        event.longitude = Some(-97.75);
        let payload: Value = serde_json::from_str(&build_snapshot(&event, None)).unwrap();
        assert_eq!(payload["data"]["location"]["lat"], 30.25);
        assert_eq!(payload["data"]["location"]["lng"], -97.75);
    }
}

async fn last_battery_level_at(pool: &PgPool, vehicle_id: Uuid, at: DateTime<Utc>) -> Option<f64> {
    sqlx::query_scalar(
        r#"SELECT battery_level
           FROM timeseries.telemetry
           WHERE vehicle_id = $1 AND ts <= $2 AND battery_level IS NOT NULL
           ORDER BY ts DESC
           LIMIT 1"#,
    )
    .bind(vehicle_id)
    .bind(at)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

async fn persist_trip(
    pool: &PgPool,
    trip: &crate::ingestion::trip_detector::CompletedTripData,
    distance: f64,
) -> anyhow::Result<()> {
    let duration = (trip.ended_at - trip.started_at).num_seconds() as i32;
    let max_speed = trip
        .points
        .iter()
        .map(|p| p.speed_mph)
        .fold(0.0_f64, f64::max);
    let avg_speed = if duration > 0 {
        Some(distance / (duration as f64 / 3600.0))
    } else {
        None
    };

    // The detector only knows battery levels it saw in this process, and the
    // vehicle reports one only when it changes. After a restart, or on a short
    // trip with no change, fall back to the last persisted reading.
    let soc_start = match trip.soc_start {
        Some(soc) => Some(soc),
        None => last_battery_level_at(pool, trip.vehicle_id, trip.started_at).await,
    };
    let soc_end = match trip.soc_end {
        Some(soc) => Some(soc),
        None => last_battery_level_at(pool, trip.vehicle_id, trip.ended_at).await,
    };

    // Energy ensemble
    let (energy_wh, energy_strategy, efficiency_wh_per_mi) = match compute_trip_energy(
        soc_start,
        soc_end,
        trip.battery_capacity_wh,
        trip.range_start_mi,
        trip.range_end_mi,
        distance,
        None,
    ) {
        Some((wh, strat)) => {
            let eff = if distance > 0.0 {
                Some(wh / distance)
            } else {
                None
            };
            (Some(wh), Some(strat.to_string()), eff)
        }
        None => {
            // Fallback: use SOC-based efficiency
            let eff = match (soc_start, soc_end, trip.battery_capacity_wh) {
                (Some(s0), Some(s1), Some(cap)) if distance > 0.0 && s0 > s1 => {
                    Some(((s0 - s1) / 100.0) * cap / distance)
                }
                _ => None,
            };
            (None, None, eff)
        }
    };

    let start = trip.points.first();
    let end = trip.points.last();

    // The periodic location reconciler enriches trips after durable capture.
    let start_match = MatchedLocation::none();
    let end_match = MatchedLocation::none();

    let route_points = trip
        .points
        .iter()
        .map(|point| (point.lat, point.lng))
        .collect::<Vec<_>>();
    let route_preview = serde_json::to_value(build_route_preview(&route_points))?;

    let mut tx = pool.begin().await?;
    if distance >= MIN_TRIP_DISTANCE_MILES {
        sqlx::query(
        r#"INSERT INTO panda.trips
           (id, vehicle_id, started_at, ended_at,
            start_lat, start_lng, end_lat, end_lng,
            distance_miles, duration_seconds,
            soc_start, soc_end,
            efficiency_wh_per_mile, max_speed_mph, avg_speed_mph, drive_mode,
            start_odometer_mi, end_odometer_mi,
            start_position_ts, end_position_ts,
            start_geofence_id, end_geofence_id,
            start_address_id, end_address_id,
            range_start_mi, range_end_mi,
            power_max_kw, power_min_kw,
            elevation_gain_m, elevation_loss_m,
            inside_temp_avg_c, outside_temp_c, regen_wh, energy_wh, energy_strategy,
            route_preview, route_preview_version)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,1) ON CONFLICT(id) DO NOTHING"#,
    )
        .bind(trip.trip_id)
        .bind(trip.vehicle_id)
        .bind(trip.started_at)
        .bind(trip.ended_at)
        .bind(start.map(|p| p.lat))
        .bind(start.map(|p| p.lng))
        .bind(end.map(|p| p.lat))
        .bind(end.map(|p| p.lng))
        .bind(distance)
        .bind(duration)
        .bind(soc_start)
        .bind(soc_end)
        .bind(efficiency_wh_per_mi)
        .bind(max_speed)
        .bind(avg_speed)
        .bind(trip.dominant_drive_mode.as_deref())
        .bind(trip.start_odometer_mi)
        .bind(trip.end_odometer_mi)
        .bind(start.map(|p| p.ts))
        .bind(end.map(|p| p.ts))
        .bind(start_match.geofence_id)
        .bind(end_match.geofence_id)
        .bind(start_match.address_id)
        .bind(end_match.address_id)
        .bind(trip.range_start_mi)
        .bind(trip.range_end_mi)
        .bind(trip.power_max_kw)
        .bind(trip.power_min_kw)
        .bind(trip.elevation_gain_m)
        .bind(trip.elevation_loss_m)
        .bind(trip.inside_temp_avg_c)
        .bind(Option::<f64>::None)
        .bind(trip.regen_wh)
        .bind(energy_wh)
        .bind(energy_strategy.as_deref())
        .bind(route_preview)
        .execute(&mut *tx)
        .await?;

        let identity: (Uuid, DateTime<Utc>, Option<DateTime<Utc>>) =
            sqlx::query_as("SELECT vehicle_id,started_at,ended_at FROM panda.trips WHERE id=$1")
                .bind(trip.trip_id)
                .fetch_one(&mut *tx)
                .await?;
        anyhow::ensure!(
            identity.0 == trip.vehicle_id
                && identity.1.timestamp_micros() == trip.started_at.timestamp_micros()
                && identity.2.map(|at| at.timestamp_micros())
                    == Some(trip.ended_at.timestamp_micros()),
            "trip identity collision; retaining completion for investigation"
        );
        sqlx::query(
            "INSERT INTO panda.weather_enrichment_jobs(trip_id) VALUES($1) ON CONFLICT(trip_id) DO NOTHING",
        ).bind(trip.trip_id).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM panda.pending_trip_completions WHERE trip_id=$1 AND vehicle_id=$2")
        .bind(trip.trip_id)
        .bind(trip.vehicle_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(())
}

/// Where a completed session happened. Charging frames rarely carry GPS, so
/// when the detector never saw a fix, use the row's stored location and then
/// the vehicle's nearest telemetry fix, so the saved place and address can
/// still be matched.
async fn charge_session_coordinates(
    pool: &PgPool,
    session: &crate::ingestion::charge_detector::CompletedChargeSession,
) -> (Option<f64>, Option<f64>) {
    if let (Some(lat), Some(lng)) = (session.location_lat, session.location_lng) {
        return (Some(lat), Some(lng));
    }

    let stored = sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
        "SELECT location_lat, location_lng FROM panda.charge_sessions WHERE id = $1",
    )
    .bind(session.session_id)
    .fetch_optional(pool)
    .await;
    match stored {
        Ok(Some((Some(lat), Some(lng)))) => return (Some(lat), Some(lng)),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(session_id=%session.session_id, error=%error, "worker.charge_session_stored_location_failed");
        }
    }

    match crate::services::trip_enrichment::nearest_telemetry_fix(
        pool,
        session.vehicle_id,
        session.started_at,
    )
    .await
    {
        Ok(Some((lat, lng))) => (Some(lat), Some(lng)),
        Ok(None) => (None, None),
        Err(error) => {
            tracing::warn!(session_id=%session.session_id, error=%error, "worker.charge_session_telemetry_fix_failed");
            (None, None)
        }
    }
}

async fn persist_charge_session(
    pool: &PgPool,
    session: &crate::ingestion::charge_detector::CompletedChargeSession,
) -> anyhow::Result<()> {
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let owner_id = get_vehicle_owner_id(pool, session.vehicle_id).await?;
    let (location_lat, location_lng) = charge_session_coordinates(pool, session).await;
    let location_match = match (owner_id, location_lat, location_lng) {
        (Some(user_id), Some(lat), Some(lon)) => {
            resolve_trip_location(pool, &http_client, user_id, lat, lon).await?
        }
        _ => MatchedLocation::none(),
    };

    let mut conn = pool.begin().await?;
    crate::services::charge_sessions::lock_charge_history(&mut conn, session.vehicle_id).await?;
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.charge_sessions WHERE id=$1 AND vehicle_id=$2)",
    )
    .bind(session.session_id)
    .bind(session.vehicle_id)
    .fetch_one(&mut *conn)
    .await?;
    anyhow::ensure!(
        present,
        "charge session was canonicalized before optional enrichment"
    );
    persist_charge_session_record(
        &mut conn,
        session,
        location_lat,
        location_lng,
        &location_match,
    )
    .await?;
    conn.commit().await?;

    if let Some(user_id) = owner_id {
        sqlx::query(
            r#"INSERT INTO panda.charge_session_user_annotations
               (charge_session_id, user_id, geofence_id, address_id, is_home, computed_at)
               VALUES ($1, $2, $3, $4, $5, now())
               ON CONFLICT (charge_session_id, user_id) DO UPDATE
               SET geofence_id = EXCLUDED.geofence_id,
                   address_id = EXCLUDED.address_id,
                   is_home = EXCLUDED.is_home,
                   computed_at = now(),
                   updated_at = now()"#,
        )
        .bind(session.session_id)
        .bind(user_id)
        .bind(location_match.geofence_id)
        .bind(location_match.address_id)
        .bind(location_match.is_home)
        .execute(pool)
        .await?;
    }

    let _ = recompute_charge_session_cost(pool, session.session_id).await?;
    Ok(())
}

async fn persist_charge_session_record(
    conn: &mut PgConnection,
    session: &crate::ingestion::charge_detector::CompletedChargeSession,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
    location_match: &MatchedLocation,
) -> anyhow::Result<()> {
    let duration_minutes = ((session.ended_at - session.started_at).num_seconds() / 60) as i32;
    let energy_added_kwh = session.energy_added_wh.map(|wh| wh / 1000.0);
    let saved: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO panda.charge_sessions
           (id, vehicle_id, started_at, ended_at,
            location_lat, location_lng, source_location_lat, source_location_lng,
            is_home,
            soc_start, soc_end, charge_limit, duration_minutes,
            kwh_added, max_charge_rate_kw, cost_usd,
            energy_added_wh, energy_used_wh,
            avg_charge_rate_kw, peak_voltage,
            geofence_id, address_id, cost_profile_id, cost_method,
            charger_type, source, data_confidence)
              VALUES ($1,$2,$3,$4,$5,$6,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,'telemetry','telemetry')
              ON CONFLICT (id) DO UPDATE SET
                  started_at = LEAST(charge_sessions.started_at, EXCLUDED.started_at),
                  ended_at = EXCLUDED.ended_at,
                  location_lat = COALESCE(EXCLUDED.location_lat, charge_sessions.location_lat),
                  location_lng = COALESCE(EXCLUDED.location_lng, charge_sessions.location_lng),
                  source_location_lat = COALESCE(EXCLUDED.source_location_lat, charge_sessions.source_location_lat),
                  source_location_lng = COALESCE(EXCLUDED.source_location_lng, charge_sessions.source_location_lng),
                  is_home = COALESCE(EXCLUDED.is_home, charge_sessions.is_home),
                  soc_start = COALESCE(charge_sessions.soc_start, EXCLUDED.soc_start),
                  soc_end = EXCLUDED.soc_end,
                  charge_limit = COALESCE(EXCLUDED.charge_limit, charge_sessions.charge_limit),
                  duration_minutes = EXCLUDED.duration_minutes,
                  kwh_added = COALESCE(EXCLUDED.kwh_added, charge_sessions.kwh_added),
                  max_charge_rate_kw = COALESCE(EXCLUDED.max_charge_rate_kw, charge_sessions.max_charge_rate_kw),
                  energy_added_wh = COALESCE(EXCLUDED.energy_added_wh, charge_sessions.energy_added_wh),
                  energy_used_wh = COALESCE(EXCLUDED.energy_used_wh, charge_sessions.energy_used_wh),
                  avg_charge_rate_kw = COALESCE(EXCLUDED.avg_charge_rate_kw, charge_sessions.avg_charge_rate_kw),
                  peak_voltage = COALESCE(EXCLUDED.peak_voltage, charge_sessions.peak_voltage),
                  geofence_id = COALESCE(EXCLUDED.geofence_id, charge_sessions.geofence_id),
                  address_id = COALESCE(EXCLUDED.address_id, charge_sessions.address_id),
                  charger_type = COALESCE(EXCLUDED.charger_type, charge_sessions.charger_type),
                  source = COALESCE(charge_sessions.source, EXCLUDED.source),
                  data_confidence = COALESCE(charge_sessions.data_confidence, EXCLUDED.data_confidence)
                  WHERE charge_sessions.vehicle_id=EXCLUDED.vehicle_id RETURNING id"#,
    )
    .bind(session.session_id)
    .bind(session.vehicle_id)
    .bind(session.started_at)
    .bind(session.ended_at)
    .bind(location_lat)
    .bind(location_lng)
    .bind(location_match.is_home)
    .bind(session.soc_start)
    .bind(session.soc_end)
    .bind(session.charge_limit)
    .bind(duration_minutes)
    .bind(energy_added_kwh)
    .bind(session.peak_charge_kw)
    .bind(Option::<f64>::None)
    .bind(session.energy_added_wh)
    .bind(session.energy_used_wh)
    .bind(session.avg_charge_rate_kw)
    .bind(session.peak_charge_kw)
    .bind(location_match.geofence_id)
    .bind(location_match.address_id)
    .bind(Option::<Uuid>::None)
    .bind(Option::<&str>::None)
    .bind(session.charger_type.as_deref())
    .fetch_optional(conn)
    .await?;
    anyhow::ensure!(
        saved == Some(session.session_id),
        "completed charge session ownership mismatch"
    );

    Ok(())
}

#[allow(dead_code)]
async fn match_point(
    pool: &PgPool,
    user_id: Uuid,
    lat: f64,
    lon: f64,
) -> anyhow::Result<MatchedLocation> {
    let matched = match_geofence(pool, user_id, lat, lon).await?;

    Ok(match matched {
        Some(geofence) => MatchedLocation {
            geofence_id: Some(geofence.id),
            address_id: geofence.address_id,
            is_home: Some(geofence.is_home),
        },
        None => {
            // No named geofence — reverse-geocode so trips get road/city labels.
            let address_id = reverse_geocode_and_store(pool, lat, lon).await;
            MatchedLocation {
                geofence_id: None,
                address_id,
                is_home: None,
            }
        }
    })
}

/// Call Nominatim reverse geocoding, store the result in `panda.addresses`,
/// and return the row's UUID.  Returns `None` on any network / DB error so
/// callers can degrade gracefully.
#[allow(dead_code)]
async fn reverse_geocode_and_store(pool: &PgPool, lat: f64, lon: f64) -> Option<Uuid> {
    crate::services::outbound_policy::require_optional_traffic().ok()?;
    let slot = crate::services::nominatim::acquire_slot(
        crate::services::nominatim::NominatimLane::BackgroundReverseGeocode,
    )
    .await;

    // ── HTTP request ──────────────────────────────────────────────────────
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error=%e, "worker.reverse_geocode_client_build_failed");
            return None;
        }
    };

    let lat_s = lat.to_string();
    let lon_s = lon.to_string();
    let resp = match client
        .get("https://nominatim.openstreetmap.org/reverse")
        .header(
            reqwest::header::USER_AGENT,
            "panda-api/0.1 (contact: support@docs.panda.invalid)",
        )
        .query(&[
            ("format", "jsonv2"),
            ("addressdetails", "1"),
            ("lat", lat_s.as_str()),
            ("lon", lon_s.as_str()),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            crate::services::nominatim::report_outcome(None, None).await;
            tracing::warn!(error=%e, lat=%lat, lon=%lon, "worker.reverse_geocode_request_failed");
            return None;
        }
    };

    let status = resp.status();
    let retry_after = crate::services::nominatim::retry_after_from_headers(resp.headers());
    crate::services::nominatim::report_outcome(Some(status.as_u16()), retry_after).await;

    if !status.is_success() {
        tracing::warn!(
            status=%status,
            lat=%lat,
            lon=%lon,
            lane=?slot.lane,
            queued_ms=slot.queued_ms,
            gate_wait_ms=slot.gate_wait_ms,
            scheduled_interval_ms=slot.effective_interval_ms,
            retry_after_s = retry_after.map(|duration| duration.as_secs()),
            "worker.reverse_geocode_http_error"
        );
        return None;
    }

    let raw: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error=%e, "worker.reverse_geocode_decode_failed");
            return None;
        }
    };

    // ── parse ─────────────────────────────────────────────────────────────
    let display_name = raw.get("display_name")?.as_str()?.to_string();
    let osm_id = raw.get("osm_id").and_then(|v| v.as_i64());
    let addr = raw.get("address").and_then(|v| v.as_object());

    let road: Option<String> = addr
        .and_then(|a| {
            a.get("road")
                .or_else(|| a.get("pedestrian"))
                .or_else(|| a.get("footway"))
        })
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let city: Option<String> = addr
        .and_then(|a| {
            a.get("city")
                .or_else(|| a.get("town"))
                .or_else(|| a.get("village"))
        })
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let state: Option<String> = addr
        .and_then(|a| a.get("state"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let postcode: Option<String> = addr
        .and_then(|a| a.get("postcode"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let country: Option<String> = addr
        .and_then(|a| a.get("country"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // ── persist ───────────────────────────────────────────────────────────
    let result: Result<Uuid, sqlx::Error> = if let Some(oid) = osm_id {
        sqlx::query_scalar(
            r#"INSERT INTO panda.addresses
               (display_name, osm_id, latitude, longitude, road, city, state, postcode, country, raw)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (osm_id) DO UPDATE SET
                 display_name = EXCLUDED.display_name,
                 road = EXCLUDED.road,
                 city = EXCLUDED.city,
                 state = EXCLUDED.state,
                 postcode = EXCLUDED.postcode,
                 country = EXCLUDED.country,
                 raw = EXCLUDED.raw
               RETURNING id"#,
        )
        .bind(display_name)
        .bind(oid)
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
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(error=%e, "worker.reverse_geocode_db_insert_failed");
            None
        }
    }
}

/// Which lifecycle signals an inbound frame may drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LifecycleUpdates {
    state_period: bool,
    power_state: bool,
    charging: bool,
}

async fn open_software_version(
    pool: &PgPool,
    vehicle_id: Uuid,
    version: &str,
    installed_at: chrono::DateTime<Utc>,
) {
    let current_open = sqlx::query_scalar::<_, Option<String>>(
        r#"SELECT version
           FROM panda.software_versions
           WHERE vehicle_id = $1 AND observed_until IS NULL
           ORDER BY installed_at DESC
           LIMIT 1"#,
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if current_open.as_ref().and_then(|v| v.as_deref()) == Some(version) {
        return;
    }

    if current_open.is_some() {
        let _ = sqlx::query(
            r#"UPDATE panda.software_versions
               SET observed_until = $1
               WHERE vehicle_id = $2 AND observed_until IS NULL"#,
        )
        .bind(installed_at)
        .bind(vehicle_id)
        .execute(pool)
        .await;
    }

    let _ = sqlx::query(
        r#"INSERT INTO panda.software_versions (vehicle_id, version, installed_at)
           VALUES ($1, $2, $3)"#,
    )
    .bind(vehicle_id)
    .bind(version)
    .bind(installed_at)
    .execute(pool)
    .await;
}

async fn close_software_version(
    pool: &PgPool,
    vehicle_id: Uuid,
    version: &str,
    observed_until: chrono::DateTime<Utc>,
) {
    let _ = sqlx::query(
        r#"UPDATE panda.software_versions
           SET observed_until = $1
           WHERE vehicle_id = $2 AND version = $3 AND observed_until IS NULL"#,
    )
    .bind(observed_until)
    .bind(vehicle_id)
    .bind(version)
    .execute(pool)
    .await;
}

async fn upsert_health(
    pool: &PgPool,
    vehicle_id: Uuid,
    online: bool,
    health: &str,
    msg: &str,
    auth_state: Option<&str>,
    auth_reason_code: Option<&str>,
) {
    let _ = sqlx::query(
        r#"INSERT INTO panda.vehicle_runtime_state
           (vehicle_id, is_online, worker_health, worker_health_msg, auth_state, auth_reason_code, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,now())
           ON CONFLICT (vehicle_id) DO UPDATE
           SET is_online=$2,
               worker_health=$3,
               worker_health_msg=$4,
               auth_state=CASE
                   WHEN panda.vehicle_runtime_state.auth_state = 'needs_reauth'
                        AND $5 = 'authorized'
                     THEN panda.vehicle_runtime_state.auth_state
                   ELSE COALESCE($5, panda.vehicle_runtime_state.auth_state)
               END,
               auth_reason_code=CASE
                   WHEN panda.vehicle_runtime_state.auth_state = 'needs_reauth'
                        AND $5 = 'authorized'
                     THEN panda.vehicle_runtime_state.auth_reason_code
                   WHEN $5 IS NULL AND $6 IS NULL THEN panda.vehicle_runtime_state.auth_reason_code
                   ELSE $6
               END,
               updated_at=now()"#,
    )
    .bind(vehicle_id)
    .bind(online)
    .bind(health)
    .bind(msg)
    .bind(auth_state)
    .bind(auth_reason_code)
    .execute(pool)
    .await;
}

async fn upsert_seen(
    pool: &PgPool,
    vehicle_id: Uuid,
    online: bool,
    ts: DateTime<Utc>,
    received_at: DateTime<Utc>,
    kind: SeenKind,
) {
    let (payload_at, heartbeat_at, payload_received_at, heartbeat_received_at) = match kind {
        SeenKind::Payload => (Some(ts), None, Some(received_at), None),
        SeenKind::Heartbeat => (None, Some(ts), None, Some(received_at)),
    };
    let _ = sqlx::query(
        r#"INSERT INTO panda.vehicle_runtime_state
           (vehicle_id, is_online, last_event_at, last_seen_at, last_payload_at, last_heartbeat_at,
            last_ws_received_at, last_ws_payload_received_at, last_ws_heartbeat_received_at, updated_at)
           VALUES ($1,$2,$3,$3,$4,$5,$6,$7,$8,now())
           ON CONFLICT (vehicle_id) DO UPDATE
           SET is_online=$2,
               last_event_at=$3,
               last_seen_at=$3,
               last_payload_at=COALESCE($4, panda.vehicle_runtime_state.last_payload_at),
               last_heartbeat_at=COALESCE($5, panda.vehicle_runtime_state.last_heartbeat_at),
               last_ws_received_at=$6,
               last_ws_payload_received_at=COALESCE($7, panda.vehicle_runtime_state.last_ws_payload_received_at),
               last_ws_heartbeat_received_at=COALESCE($8, panda.vehicle_runtime_state.last_ws_heartbeat_received_at),
               updated_at=now()"#,
    )
    .bind(vehicle_id)
    .bind(online)
    .bind(ts)
    .bind(payload_at)
    .bind(heartbeat_at)
    .bind(received_at)
    .bind(payload_received_at)
    .bind(heartbeat_received_at)
    .execute(pool)
    .await;
}

async fn mark_persisted(pool: &PgPool, vehicle_id: Uuid, ts: DateTime<Utc>) {
    let _ = sqlx::query(
        r#"INSERT INTO panda.vehicle_runtime_state
           (vehicle_id, last_persisted_at, updated_at)
           VALUES ($1,$2,now())
           ON CONFLICT (vehicle_id) DO UPDATE
           SET last_persisted_at=$2, updated_at=now()"#,
    )
    .bind(vehicle_id)
    .bind(ts)
    .execute(pool)
    .await;
}

#[cfg(test)]
mod live_session_tests {
    use super::should_clear_live_session;
    use crate::ingestion::parser::parse_charging_session_message;

    #[test]
    fn only_explicit_root_null_clears_live_snapshot() {
        let explicit_null = parse_charging_session_message(
            r#"{"type":"next","payload":{"data":{"chargingSession":null}}}"#,
        )
        .unwrap()
        .unwrap();
        let empty_partial = parse_charging_session_message(
            r#"{"type":"next","payload":{"data":{"chargingSession":{"liveData":null,"chartData":[]}}}}"#,
        )
        .unwrap()
        .unwrap();

        assert!(should_clear_live_session(&explicit_null));
        assert!(!should_clear_live_session(&empty_partial));
    }
}

#[cfg(test)]
mod stewardship_tests {
    use super::*;
    use chrono::Duration;

    fn event(vehicle_id: Uuid, offset_seconds: i64) -> TelemetryEvent {
        blank_event(vehicle_id, Utc::now() + Duration::seconds(offset_seconds))
    }

    #[test]
    fn empty_event_is_suppressed() {
        let vehicle_id = Uuid::new_v4();
        let mut gate = TelemetryPersistenceGate::new(vehicle_id);

        assert_eq!(
            gate.decide(&event(vehicle_id, 0)),
            PersistenceDecision::Suppress(SuppressionReason::Empty)
        );
    }

    #[test]
    fn duplicate_sparse_payload_is_suppressed_after_first_persist() {
        let vehicle_id = Uuid::new_v4();
        let mut gate = TelemetryPersistenceGate::new(vehicle_id);
        let mut first = event(vehicle_id, 0);
        first.battery_level = Some(72.0);

        assert_eq!(gate.decide(&first), PersistenceDecision::Persist);

        let mut duplicate = event(vehicle_id, 30);
        duplicate.battery_level = Some(72.0);
        assert_eq!(
            gate.decide(&duplicate),
            PersistenceDecision::Suppress(SuppressionReason::Duplicate)
        );
    }

    #[test]
    fn material_soc_change_persists_when_parked() {
        let vehicle_id = Uuid::new_v4();
        let mut gate = TelemetryPersistenceGate::new(vehicle_id);
        let mut first = event(vehicle_id, 0);
        first.power_state = Some(PowerState::Ready);
        first.battery_level = Some(72.0);
        assert_eq!(gate.decide(&first), PersistenceDecision::Persist);

        let mut changed = event(vehicle_id, 300);
        changed.battery_level = Some(71.4);
        assert_eq!(gate.decide(&changed), PersistenceDecision::Persist);
    }

    #[test]
    fn small_parked_soc_change_is_threshold_suppressed() {
        let vehicle_id = Uuid::new_v4();
        let mut gate = TelemetryPersistenceGate::new(vehicle_id);
        let mut first = event(vehicle_id, 0);
        first.power_state = Some(PowerState::Ready);
        first.battery_level = Some(72.0);
        assert_eq!(gate.decide(&first), PersistenceDecision::Persist);

        let mut changed = event(vehicle_id, 300);
        changed.battery_level = Some(71.8);
        assert_eq!(
            gate.decide(&changed),
            PersistenceDecision::Suppress(SuppressionReason::Threshold)
        );
    }

    #[test]
    fn driving_location_threshold_persists() {
        let vehicle_id = Uuid::new_v4();
        let mut gate = TelemetryPersistenceGate::new(vehicle_id);
        let mut first = event(vehicle_id, 0);
        first.power_state = Some(PowerState::Drive);
        first.latitude = Some(30.0);
        first.longitude = Some(-97.0);
        assert_eq!(gate.decide(&first), PersistenceDecision::Persist);

        let mut moved = event(vehicle_id, 6);
        moved.latitude = Some(30.0003);
        moved.longitude = Some(-97.0);
        assert_eq!(gate.decide(&moved), PersistenceDecision::Persist);
    }

    #[test]
    fn ws_schema_rejection_keeps_auth_state_authorized() {
        let inbound = WsInboundEvent {
            kind: WsInboundKind::Control,
            received_at: Utc::now(),
            raw: serde_json::json!({
                "type": "ws_schema_rejected",
                "reason": "Cannot query field \"vehiclePowerOutput\" on type \"VehicleState\".",
            })
            .to_string(),
            message_type: Some("ws_schema_rejected".into()),
            telemetry: None,
            charging_session: None,
            battery_cell_type: None,
        };

        let update = runtime_health_update_for_ws_control(&inbound).expect("update");
        assert_eq!(update.worker_health, "degraded");
        assert_eq!(update.auth_state, "authorized");
        assert_eq!(update.auth_reason_code, Some("rivian_ws_schema_rejected"));
        assert!(update.worker_health_msg.contains("vehiclePowerOutput"));
    }

    #[test]
    fn ws_no_active_subscriptions_is_degraded_not_auth_failure() {
        let inbound = WsInboundEvent {
            kind: WsInboundKind::Control,
            received_at: Utc::now(),
            raw: serde_json::json!({
                "type": "ws_no_active_subscriptions",
                "reason": "Socket with no active subscriptions, disconnecting",
            })
            .to_string(),
            message_type: Some("ws_no_active_subscriptions".into()),
            telemetry: None,
            charging_session: None,
            battery_cell_type: None,
        };

        let update = runtime_health_update_for_ws_control(&inbound).expect("update");
        assert_eq!(update.worker_health, "degraded");
        assert_eq!(update.auth_state, "authorized");
        assert_eq!(
            update.auth_reason_code,
            Some("rivian_ws_no_active_subscriptions")
        );
    }

    #[test]
    fn parallax_power_frames_cannot_end_an_active_charge() {
        let mut event = blank_event(Uuid::nil(), Utc::now());
        event.power_state = Some(PowerState::Ready);
        let blocked = LifecycleUpdates {
            state_period: false,
            power_state: false,
            charging: false,
        };
        assert_eq!(lifecycle_updates(&event, true, true), blocked);
        event.power_state = Some(PowerState::Sleep);
        assert_eq!(lifecycle_updates(&event, true, true), blocked);
    }

    #[test]
    fn lifecycle_updates_keep_legacy_and_idle_parallax_behaviour() {
        let all = LifecycleUpdates {
            state_period: true,
            power_state: true,
            charging: true,
        };
        let mut event = blank_event(Uuid::nil(), Utc::now());
        event.power_state = Some(PowerState::Ready);
        // Legacy power frames drive the lifecycle, charge or not.
        assert_eq!(lifecycle_updates(&event, false, true), all);
        // Without an open session, Parallax power behaves as before.
        assert_eq!(lifecycle_updates(&event, true, false), all);
        // Parallax frames that carry charger state still count.
        event.charger_state = Some(ChargerState::Charging);
        assert_eq!(lifecycle_updates(&event, true, true), all);
        // A frame with no lifecycle fields changes nothing, from either
        // source.
        let empty = blank_event(Uuid::nil(), Utc::now());
        let none = LifecycleUpdates {
            state_period: false,
            power_state: false,
            charging: false,
        };
        assert_eq!(lifecycle_updates(&empty, true, false), none);
        assert_eq!(lifecycle_updates(&empty, false, false), none);
    }

    #[test]
    fn partial_legacy_frames_do_not_fragment_the_state_timeline() {
        // Observed R1S legacy frame: a door change with no power state.
        let mut door_only = blank_event(Uuid::nil(), Utc::now());
        door_only.door_front_left_closed = Some(false);
        let updates = lifecycle_updates(&door_only, false, true);
        assert!(!updates.state_period);
        assert!(!updates.power_state);
        // Mid-charge, it must not drop the live-session poll cadence either.
        assert!(!updates.charging);

        // Standby is reported as power "standby", which is a real state.
        let mut standby = blank_event(Uuid::nil(), Utc::now());
        standby.power_state = Some(PowerState::Unknown);
        assert!(lifecycle_updates(&standby, false, false).state_period);

        // Charger and offline evidence still move the timeline without power.
        let mut charging = blank_event(Uuid::nil(), Utc::now());
        charging.charger_state = Some(ChargerState::Charging);
        let updates = lifecycle_updates(&charging, false, false);
        assert!(updates.state_period && updates.charging && !updates.power_state);
        let mut offline = blank_event(Uuid::nil(), Utc::now());
        offline.is_online = Some(false);
        assert!(lifecycle_updates(&offline, false, false).state_period);
        // A disconnected charger alone says nothing about the vehicle state.
        let mut disconnected = blank_event(Uuid::nil(), Utc::now());
        disconnected.charger_state = Some(ChargerState::Disconnected);
        let updates = lifecycle_updates(&disconnected, false, false);
        assert!(!updates.state_period && updates.charging);
    }

    #[test]
    fn state_inference_keeps_charging_and_offline_precedence_stable() {
        assert_eq!(
            infer_vehicle_state_values(
                Some(&PowerState::Sleep),
                Some(&ChargerState::Charging),
                None,
                None,
                Some(true),
            ),
            VehicleState::Charging
        );
        assert_eq!(
            infer_vehicle_state_values(Some(&PowerState::Ready), None, None, None, Some(false)),
            VehicleState::Offline
        );
    }
}

fn field_age(
    event: &TelemetryEvent,
    present: bool,
    observed_at: Option<DateTime<Utc>>,
) -> Option<i64> {
    present.then(|| (event.ts - observed_at.unwrap_or(event.ts)).num_seconds())
}

/// Legacy WebSocket traffic as a capture record: vehicle-state fields with
/// their source timestamps, charging-session fields, or connection events.
/// Location values are reduced to a presence marker.
fn legacy_capture_fields(inbound: &WsInboundEvent) -> Option<(CaptureKind, serde_json::Value)> {
    let raw = serde_json::from_str::<serde_json::Value>(&inbound.raw).ok();
    match inbound.kind {
        WsInboundKind::Heartbeat => None,
        WsInboundKind::Control => Some((
            CaptureKind::LegacyConnection,
            serde_json::json!({
                "message_type": inbound.message_type,
                "detail": raw.filter(serde_json::Value::is_object),
            }),
        )),
        WsInboundKind::Telemetry | WsInboundKind::ChargingSession => {
            let data = raw.as_ref()?.pointer("/payload/data")?;
            let mut fields = serde_json::Map::new();
            if let Some(state) = data
                .get("vehicleState")
                .and_then(serde_json::Value::as_object)
            {
                for (key, value) in state {
                    let Some(value) = value.as_object() else {
                        continue;
                    };
                    let time_stamp = value.get("timeStamp").cloned();
                    if key == "gnssLocation" {
                        fields.insert(
                            "gnssLocationPresent".into(),
                            serde_json::json!({ "timeStamp": time_stamp }),
                        );
                    } else {
                        fields.insert(
                            key.clone(),
                            serde_json::json!({ "value": value.get("value"), "timeStamp": time_stamp }),
                        );
                    }
                }
            }
            let charging_session = data
                .get("chargingSession")
                .and_then(serde_json::Value::as_object)
                .map(|session| {
                    session
                        .iter()
                        .filter(|(_, value)| !value.is_null())
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<serde_json::Map<_, _>>()
                });
            Some((
                CaptureKind::LegacyFrame,
                serde_json::json!({
                    "frame": inbound.kind.as_str(),
                    "message_type": inbound.message_type,
                    "fields": fields,
                    "charging_session": charging_session,
                }),
            ))
        }
    }
}

fn persistence_label(decision: &PersistenceDecision) -> &'static str {
    match decision {
        PersistenceDecision::Persist => "persisted",
        PersistenceDecision::Suppress(SuppressionReason::Empty) => "suppressed_empty",
        PersistenceDecision::Suppress(SuppressionReason::Duplicate) => "suppressed_duplicate",
        PersistenceDecision::Suppress(SuppressionReason::Threshold) => "suppressed_threshold",
    }
}

/// What the worker did with one inbound sample, for a capture.
fn ingestion_capture_fields(
    event: &TelemetryEvent,
    message_type: Option<&str>,
    is_parallax: bool,
    charge_event: &ChargeEvent,
    persistence: &PersistenceDecision,
    lifecycle: LifecycleUpdates,
    charge_active: bool,
) -> serde_json::Value {
    let state_period = if lifecycle.state_period {
        serde_json::to_value(infer_vehicle_state(event)).unwrap_or_default()
    } else if is_parallax && charge_active && event.charger_state.is_none() {
        "skipped_parallax_charge_guard".into()
    } else {
        "no_lifecycle_fields".into()
    };
    serde_json::json!({
        "source": match (is_parallax, message_type) {
            (true, kind) => kind.unwrap_or("parallax"),
            (false, Some("baseline")) => "baseline",
            (false, _) => "legacy",
        },
        "event_ts": event.ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "sample_age_ms": (Utc::now() - event.ts).num_milliseconds(),
        "values": ingestion_capture::present_fields(event),
        "persistence": persistence_label(persistence),
        "state_period": state_period,
        "power_tx_sent": lifecycle.power_state,
        "charging_tx_sent": lifecycle.charging,
        "charge_active": charge_active,
        "charge_lifecycle_eligible": !is_parallax,
        "charge_transition": match charge_event {
            ChargeEvent::SessionStarted => "started",
            ChargeEvent::SessionEnded(_) => "ended",
            ChargeEvent::NoChange => "no_change",
        },
        "actively_charging": event.is_actively_charging(),
    })
}

/// Why the trip detector did what it did for one sample. Never includes
/// coordinates, odometer readings, or completed-trip contents.
fn trip_diagnostics_fields(
    event: &TelemetryEvent,
    trip_sample: &TelemetryEvent,
    fusion: &FusionDiagnostics,
    trip_active: bool,
    start_decision: &str,
    trip_event: &TripEvent,
) -> serde_json::Value {
    serde_json::json!({
        "trip_active": trip_active,
        "fused_power": trip_sample.power_state.is_some() && event.power_state.is_none(),
        "derived_speed": trip_sample.speed_mph.is_some() && event.speed_mph.is_none(),
        "effective_power": trip_sample.power_state,
        "power_origin": fusion.power_origin,
        "power_policy": fusion.power_policy,
        "power_outcome": fusion.power_outcome,
        "reported_power_outcome": fusion.reported_power_outcome,
        "power_age_seconds": field_age(trip_sample, trip_sample.power_state.is_some(), trip_sample.power_state_ts),
        "speed_mph": trip_sample.speed_mph,
        "speed_origin": fusion.speed_origin,
        "reported_speed_outcome": fusion.reported_speed_outcome,
        "speed_age_seconds": field_age(trip_sample, trip_sample.speed_mph.is_some(), trip_sample.speed_mph_ts),
        "gnss_outcome": fusion.gnss_outcome,
        "gnss_interval_seconds": fusion.gnss_interval_seconds,
        "gnss_distance_miles": fusion.gnss_distance_miles,
        "moving_segments": fusion.moving_segments,
        "odometer_outcome": fusion.odometer_outcome,
        "odometer_interval_seconds": fusion.odometer_interval_seconds,
        "odometer_delta_miles": fusion.odometer_delta_miles,
        "start_decision": start_decision,
        "trip_transition": trip_event.transition_name(),
    })
}

fn record_trip_diagnostics(
    vehicle_id: Uuid,
    event: &TelemetryEvent,
    trip_sample: &TelemetryEvent,
    fusion: &FusionDiagnostics,
    trip_active: bool,
    start_decision: &str,
    trip_event: &TripEvent,
) {
    ingestion_capture::record(
        vehicle_id,
        CaptureKind::Trip,
        trip_diagnostics_fields(
            event,
            trip_sample,
            fusion,
            trip_active,
            start_decision,
            trip_event,
        ),
    );
}

#[cfg(test)]
mod diagnostic_capture_tests {
    use super::*;

    #[test]
    fn completed_trip_diagnostics_explain_decision_without_trip_contents() {
        let at = Utc::now();
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut source = TelemetryEvent::empty(Uuid::nil(), at);
        source.power_state = Some(PowerState::Go);
        source.speed_mph = Some(30.0);
        source.latitude = Some(33.1234567);
        source.longitude = Some(-97.7654321);
        source.odometer_miles = Some(123456.789);
        detector.process(&source);
        let mut stopped = source.clone();
        stopped.ts += Duration::seconds(60);
        stopped.power_state = Some(PowerState::Sleep);
        let transition = detector.process(&stopped);
        assert!(matches!(&transition, TripEvent::TripEnded { .. }));
        let fields = trip_diagnostics_fields(
            &stopped,
            &stopped,
            &FusionDiagnostics::default(),
            false,
            "already_active",
            &transition,
        );
        assert_eq!(fields["trip_transition"], "ended");
        assert_eq!(fields["speed_mph"], 30.0);
        assert_eq!(fields["start_decision"], "already_active");
        let output = ingestion_capture::sanitize(fields).to_string();
        for private in [
            "33.1234567",
            "97.7654321",
            "123456.789",
            "CompletedTripData",
            "points",
        ] {
            assert!(
                !output.contains(private),
                "private trip contents in diagnostics: {private}"
            );
        }
    }

    #[test]
    fn legacy_frames_keep_values_and_timestamps_but_not_location() {
        let inbound = WsInboundEvent {
            kind: WsInboundKind::Telemetry,
            received_at: Utc::now(),
            raw: serde_json::json!({
                "type": "next",
                "payload": { "data": { "vehicleState": {
                    "doorFrontLeftClosed": { "value": "open", "timeStamp": "2026-09-30T21:10:47.971Z" },
                    "gnssLocation": { "latitude": 33.1234567, "longitude": -97.7654321, "timeStamp": "2026-09-30T21:10:47.000Z" },
                    "gnssSpeed": null,
                } } }
            })
            .to_string(),
            message_type: Some("next".into()),
            telemetry: None,
            charging_session: None,
            battery_cell_type: None,
        };
        let (kind, fields) = legacy_capture_fields(&inbound).unwrap();
        assert_eq!(kind, CaptureKind::LegacyFrame);
        assert_eq!(
            fields["fields"]["doorFrontLeftClosed"],
            serde_json::json!({ "value": "open", "timeStamp": "2026-09-30T21:10:47.971Z" })
        );
        assert_eq!(
            fields["fields"]["gnssLocationPresent"]["timeStamp"],
            "2026-09-30T21:10:47.000Z"
        );
        assert!(fields["fields"].get("gnssSpeed").is_none());
        let text = ingestion_capture::sanitize(fields).to_string();
        assert!(!text.contains("33.1234567") && !text.contains("97.7654321"));
    }

    #[test]
    fn ingestion_fields_explain_the_parallax_charge_guard() {
        let mut event = TelemetryEvent::empty(Uuid::nil(), Utc::now());
        event.power_state = Some(PowerState::Ready);
        event.latitude = Some(33.1234567);
        let lifecycle = lifecycle_updates(&event, true, true);
        let fields = ingestion_capture_fields(
            &event,
            Some("parallax:vehicle.power.state"),
            true,
            &ChargeEvent::NoChange,
            &PersistenceDecision::Persist,
            lifecycle,
            true,
        );
        assert_eq!(fields["source"], "parallax:vehicle.power.state");
        assert_eq!(fields["state_period"], "skipped_parallax_charge_guard");
        assert_eq!(fields["power_tx_sent"], false);
        assert_eq!(fields["persistence"], "persisted");
        assert_eq!(fields["values"]["power_state"], "ready");
        assert!(fields["values"].get("latitude").is_none());
    }
}

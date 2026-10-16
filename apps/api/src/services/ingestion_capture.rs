//! Owner-started ingestion captures.
//!
//! A capture records what the ingestion pipeline saw and decided for one
//! vehicle, so an owner can download one shareable file instead of reading
//! host logs. Each vehicle keeps only its most recent capture: starting a new
//! one replaces the previous capture's events.
//!
//! Recording is fire-and-forget. Hot paths check an in-memory registry and
//! hand records to one batch writer through a bounded channel, so ingestion
//! never waits on capture storage. Records never contain coordinates,
//! credentials, VINs, or the vehicle id.

mod read_model;
mod writer;
pub use read_model::{export, export_filename, export_rows, status, CaptureStatus, Export};
use writer::{insert_marker, run_expiry, run_purge, run_writer};

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock, RwLock,
    },
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Captures stop on their own after this long.
pub const CAPTURE_DURATION: chrono::Duration = chrono::Duration::hours(1);
/// Finished captures are purged after this long.
pub const CAPTURE_RETENTION: chrono::Duration = chrono::Duration::hours(24);
/// Hard ceiling on stored events per capture.
pub const MAX_EVENTS_PER_CAPTURE: u64 = 50_000;
/// Version of the downloadable file format.
pub const FORMAT_VERSION: u32 = 1;

const CHANNEL_CAPACITY: usize = 4096;
const BATCH_ROWS: usize = 200;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const EXPIRY_INTERVAL: Duration = Duration::from_secs(15);
const PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ParallaxEnvelope,
    ParallaxConnection,
    LegacyFrame,
    LegacyConnection,
    Ingestion,
    Trip,
    Poll,
    CaptureStarted,
    CaptureStopped,
    Truncated,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParallaxEnvelope => "parallax_envelope",
            Self::ParallaxConnection => "parallax_connection",
            Self::LegacyFrame => "legacy_frame",
            Self::LegacyConnection => "legacy_connection",
            Self::Ingestion => "ingestion",
            Self::Trip => "trip",
            Self::Poll => "poll",
            Self::CaptureStarted => "capture_started",
            Self::CaptureStopped => "capture_stopped",
            Self::Truncated => "truncated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    User,
    Expired,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug)]
struct Record {
    vehicle_id: Uuid,
    capture_id: Uuid,
    recorded_at: DateTime<Utc>,
    kind: Kind,
    fields: Value,
}

#[derive(Debug, Clone)]
struct Window {
    capture_id: Uuid,
    ends_at: DateTime<Utc>,
    dropped: Arc<AtomicU64>,
}

struct Capture {
    windows: RwLock<HashMap<Uuid, Window>>,
    tx: mpsc::Sender<Record>,
}

static CAPTURE: OnceLock<Capture> = OnceLock::new();

fn active_window(vehicle_id: Uuid, now: DateTime<Utc>) -> Option<Window> {
    let capture = CAPTURE.get()?;
    let windows = capture.windows.read().ok()?;
    windows
        .get(&vehicle_id)
        .filter(|window| window.ends_at > now)
        .cloned()
}

/// Whether a capture is running for this vehicle. Cheap enough for hot paths;
/// callers use it to skip building records nobody will store.
pub fn is_capturing(vehicle_id: Uuid) -> bool {
    active_window(vehicle_id, Utc::now()).is_some()
}

/// Record one capture event. A no-op when no capture is running (or the
/// service was never started, as in tests and CLI binaries). Never blocks:
/// a full channel drops the record and counts it.
pub fn record(vehicle_id: Uuid, kind: Kind, fields: Value) {
    let now = Utc::now();
    let Some(window) = active_window(vehicle_id, now) else {
        return;
    };
    let Some(capture) = CAPTURE.get() else {
        return;
    };
    let record = Record {
        vehicle_id,
        capture_id: window.capture_id,
        recorded_at: now,
        kind,
        fields: sanitize(fields),
    };
    if capture.tx.try_send(record).is_err() {
        window.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// Keys that must never appear in a capture, at any depth.
const FORBIDDEN_KEYS: &[&str] = &[
    "vehicle_id",
    "vin",
    "latitude",
    "longitude",
    "lat",
    "lng",
    "lon",
    "altitude_m",
    "gnssLocation",
    "location",
    "name",
    "email",
    "token",
    "u-sess",
    "a-sess",
    "csrf",
    "password",
    "authorization",
];

fn is_forbidden_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    FORBIDDEN_KEYS
        .iter()
        .any(|forbidden| lower == forbidden.to_ascii_lowercase())
        || ["token", "secret", "password", "csrf"]
            .iter()
            .any(|needle| lower.contains(needle))
}

/// Remove identifying and location keys at any depth. Call sites already
/// avoid them; this is the backstop that keeps the file shareable.
pub fn sanitize(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, _)| !is_forbidden_key(key))
                .map(|(key, value)| (key, sanitize(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize).collect()),
        other => other,
    }
}

/// Non-null fields of a serialized value, without identity or location.
pub fn present_fields<T: serde::Serialize>(value: &T) -> Value {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => sanitize(Value::Object(
            map.into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect::<Map<_, _>>(),
        )),
        _ => Value::Null,
    }
}

/// Start the batch writer, expiry, and purge tasks, and restore any capture
/// that was running when the process stopped.
pub async fn init(pool: PgPool) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let capture = Capture {
        windows: RwLock::new(HashMap::new()),
        tx,
    };
    if CAPTURE.set(capture).is_err() {
        anyhow::bail!("ingestion capture service already initialised");
    }

    sqlx::query(
        "UPDATE panda.vehicle_ingestion_diagnostics \
         SET stopped_at = enabled_until, stop_reason = 'expired', updated_at = now() \
         WHERE stopped_at IS NULL AND enabled_until <= now()",
    )
    .execute(&pool)
    .await?;
    let open = sqlx::query_as::<_, (Uuid, Uuid, DateTime<Utc>)>(
        "SELECT vehicle_id, capture_id, enabled_until FROM panda.vehicle_ingestion_diagnostics \
         WHERE stopped_at IS NULL",
    )
    .fetch_all(&pool)
    .await?;
    for (vehicle_id, capture_id, ends_at) in open {
        register(vehicle_id, capture_id, ends_at);
    }

    tokio::spawn(run_writer(pool.clone(), rx));
    tokio::spawn(run_expiry(pool.clone()));
    tokio::spawn(run_purge(pool));
    Ok(())
}

fn register(vehicle_id: Uuid, capture_id: Uuid, ends_at: DateTime<Utc>) {
    if let Some(capture) = CAPTURE.get() {
        if let Ok(mut windows) = capture.windows.write() {
            windows.insert(
                vehicle_id,
                Window {
                    capture_id,
                    ends_at,
                    dropped: Arc::new(AtomicU64::new(0)),
                },
            );
        }
    }
}

fn unregister(vehicle_id: Uuid) -> Option<Window> {
    CAPTURE
        .get()
        .and_then(|capture| capture.windows.write().ok()?.remove(&vehicle_id))
}

/// Start a new capture, replacing the vehicle's previous one.
pub async fn start(pool: &PgPool, vehicle_id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
    unregister(vehicle_id);
    let capture_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM panda.vehicle_ingestion_capture_events WHERE vehicle_id = $1")
        .bind(vehicle_id)
        .execute(&mut *tx)
        .await?;
    let (started_at, ends_at) = sqlx::query_as::<_, (DateTime<Utc>, DateTime<Utc>)>(
        "INSERT INTO panda.vehicle_ingestion_diagnostics \
             (vehicle_id, capture_id, started_at, enabled_until, enabled_by, stopped_at, stop_reason, dropped_events) \
         VALUES ($1, $2, now(), now() + $3::interval, $4, NULL, NULL, 0) \
         ON CONFLICT (vehicle_id) DO UPDATE SET \
             capture_id = EXCLUDED.capture_id, started_at = EXCLUDED.started_at, \
             enabled_until = EXCLUDED.enabled_until, enabled_by = EXCLUDED.enabled_by, \
             stopped_at = NULL, stop_reason = NULL, dropped_events = 0, updated_at = now() \
         RETURNING started_at, enabled_until",
    )
    .bind(vehicle_id)
    .bind(capture_id)
    .bind(CAPTURE_DURATION)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    insert_marker(
        &mut *tx,
        vehicle_id,
        capture_id,
        started_at,
        Kind::CaptureStarted,
        serde_json::json!({ "ends_at": ends_at }),
    )
    .await?;
    tx.commit().await?;
    register(vehicle_id, capture_id, ends_at);
    Ok(())
}

/// Stop the running capture, keeping its events for download. Returns false
/// when no capture was running.
pub async fn stop(pool: &PgPool, vehicle_id: Uuid, reason: StopReason) -> anyhow::Result<bool> {
    let window = unregister(vehicle_id);
    let dropped = window
        .as_ref()
        .map_or(0, |window| window.dropped.load(Ordering::Relaxed));
    let stopped = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
        "UPDATE panda.vehicle_ingestion_diagnostics \
         SET stopped_at = LEAST(now(), enabled_until), stop_reason = $2, \
             dropped_events = dropped_events + $3, updated_at = now() \
         WHERE vehicle_id = $1 AND stopped_at IS NULL \
         RETURNING capture_id, stopped_at",
    )
    .bind(vehicle_id)
    .bind(reason.as_str())
    .bind(i64::try_from(dropped).unwrap_or(i64::MAX))
    .fetch_optional(pool)
    .await?;
    let Some((capture_id, stopped_at)) = stopped else {
        return Ok(false);
    };
    insert_marker(
        pool,
        vehicle_id,
        capture_id,
        stopped_at,
        Kind::CaptureStopped,
        serde_json::json!({ "reason": reason.as_str(), "dropped_events": dropped }),
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_is_a_noop_without_a_running_capture() {
        // The service is never initialised in unit tests.
        let vehicle_id = Uuid::new_v4();
        assert!(!is_capturing(vehicle_id));
        record(vehicle_id, Kind::Ingestion, json!({ "source": "legacy" }));
        assert!(!is_capturing(vehicle_id));
    }

    #[test]
    fn sanitize_strips_identity_location_and_credentials_at_any_depth() {
        let cleaned = sanitize(json!({
            "vehicle_id": "x",
            "topic": "body.closures.states",
            "decoded": {
                "latitude": 1.0,
                "longitude": 2.0,
                "speed_mph": 3.0,
                "door_front_left_closed": false,
            },
            "fields": { "gnssLocation": { "latitude": 1.0 }, "doorFrontLeftClosed": { "value": "open" } },
            "headers": { "U-Sess": "secret", "csrfToken": "t", "appSessionToken": "t" },
            "entries": [{ "vin": "7PD", "position": 1 }],
        }));
        assert_eq!(
            cleaned,
            json!({
                "topic": "body.closures.states",
                "decoded": { "speed_mph": 3.0, "door_front_left_closed": false },
                "fields": { "doorFrontLeftClosed": { "value": "open" } },
                "headers": {},
                "entries": [{ "position": 1 }],
            })
        );
    }
}

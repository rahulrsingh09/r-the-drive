//! Charge state and completion delivery committed with the trip checkpoint.
use anyhow::{ensure, Context};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

use super::state_decisions::SampleSource;
use crate::ingestion::charge_detector::{ChargeDetectorState, ChargeEvent, CompletedChargeSession};
use crate::models::telemetry::TelemetryEvent;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    sequence: u64,
    detector: ChargeDetectorState,
    last_observation: Option<Observation>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    at: DateTime<Utc>,
    received_at: DateTime<Utc>,
    source: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    version: u32,
    session: CompletedChargeSession,
}

pub(super) struct ChargeCapture {
    checkpoint: Checkpoint,
    pending: Vec<CompletedChargeSession>,
    persisted: Option<serde_json::Value>,
}

impl ChargeCapture {
    pub(super) fn from_detector(detector: ChargeDetectorState) -> Self {
        Self {
            checkpoint: Checkpoint {
                version: 1,
                sequence: 0,
                detector,
                last_observation: None,
            },
            pending: Vec::new(),
            persisted: None,
        }
    }

    #[cfg(test)]
    pub(super) async fn load(pool: &PgPool, vehicle_id: Uuid) -> anyhow::Result<Option<Self>> {
        let mut conn = pool.acquire().await?;
        Self::load_on(&mut conn, vehicle_id).await
    }

    async fn load_on(conn: &mut PgConnection, vehicle_id: Uuid) -> anyhow::Result<Option<Self>> {
        let stored: Option<(serde_json::Value, Option<Uuid>)> = sqlx::query_as(
            "SELECT snapshot,active_session_id FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
        )
        .bind(vehicle_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((value, active_id)) = stored else {
            return Ok(None);
        };
        let checkpoint: Checkpoint = serde_json::from_value(value.clone())
            .context("invalid charge checkpoint; refusing to discard recovery state")?;
        ensure!(
            checkpoint.version == 1,
            "unsupported charge checkpoint version"
        );
        ensure!(
            checkpoint.detector.vehicle_id() == vehicle_id,
            "charge checkpoint vehicle mismatch"
        );
        ensure!(
            checkpoint.detector.active_session_id() == active_id,
            "charge checkpoint session mismatch"
        );
        if let Some(id) = active_id {
            let owner: Option<Uuid> =
                sqlx::query_scalar("SELECT vehicle_id FROM panda.charge_sessions WHERE id=$1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?;
            ensure!(
                owner == Some(vehicle_id),
                "charge checkpoint session ownership mismatch"
            );
        }
        Ok(Some(Self {
            checkpoint,
            pending: Vec::new(),
            persisted: Some(value),
        }))
    }

    pub(super) fn detector(&self) -> &ChargeDetectorState {
        &self.checkpoint.detector
    }

    pub(super) fn process(
        &mut self,
        event: &TelemetryEvent,
        source: SampleSource,
        received_at: DateTime<Utc>,
    ) -> anyhow::Result<ChargeEvent> {
        ensure!(
            event.vehicle_id == self.detector().vehicle_id(),
            "charge observation vehicle mismatch"
        );
        self.checkpoint.sequence = self
            .checkpoint
            .sequence
            .checked_add(1)
            .context("charge sequence exhausted")?;
        self.checkpoint.last_observation = Some(Observation {
            at: event.ts,
            received_at,
            source: match source {
                SampleSource::Socket => "socket",
                SampleSource::Baseline => "baseline",
                SampleSource::Parallax => "parallax",
            }
            .into(),
        });
        let transition = if source.is_parallax() {
            ChargeEvent::NoChange
        } else {
            self.checkpoint.detector.process(event)
        };
        if let ChargeEvent::SessionEnded(session) = &transition {
            self.pending.push(session.clone());
        }
        Ok(transition)
    }

    pub(super) async fn write(&self, conn: &mut PgConnection) -> anyhow::Result<serde_json::Value> {
        let value = serde_json::to_value(&self.checkpoint)?;
        if let Some(active) = self.detector().active_snapshot() {
            let mut start = TelemetryEvent::empty(self.detector().vehicle_id(), active.started_at);
            start.latitude = active.location_lat;
            start.longitude = active.location_lng;
            start.battery_level = active.soc_start;
            start.battery_limit = active.charge_limit;
            super::ensure_active_charge_session_on(
                conn,
                self.detector().vehicle_id(),
                active.session_id,
                &start,
            )
            .await?;
        }
        for session in &self.pending {
            let mut start = TelemetryEvent::empty(session.vehicle_id, session.started_at);
            start.latitude = session.location_lat;
            start.longitude = session.location_lng;
            start.battery_level = session.soc_start;
            start.battery_limit = session.charge_limit;
            super::ensure_active_charge_session_on(
                conn,
                session.vehicle_id,
                session.session_id,
                &start,
            )
            .await?;
            let saved: Option<Uuid> = sqlx::query_scalar(
                "INSERT INTO panda.pending_charge_completions(session_id,vehicle_id,session) VALUES($1,$2,$3) \
                 ON CONFLICT(session_id) DO UPDATE SET session=EXCLUDED.session \
                 WHERE pending_charge_completions.vehicle_id=EXCLUDED.vehicle_id \
                   AND pending_charge_completions.session=EXCLUDED.session RETURNING session_id",
            )
            .bind(session.session_id).bind(session.vehicle_id).bind(serde_json::to_value(Completion { version: 1, session: session.clone() })?)
            .fetch_optional(&mut *conn).await?;
            ensure!(
                saved == Some(session.session_id),
                "conflicting pending charge completion"
            );
        }
        let saved: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO panda.active_charge_checkpoints(vehicle_id,active_session_id,snapshot) VALUES($1,$2,$3) \
             ON CONFLICT(vehicle_id) DO UPDATE SET active_session_id=$2,snapshot=$3,updated_at=now() \
             WHERE active_charge_checkpoints.snapshot=$4 OR active_charge_checkpoints.snapshot=$3 RETURNING vehicle_id",
        )
        .bind(self.detector().vehicle_id()).bind(self.detector().active_session_id()).bind(&value).bind(&self.persisted)
        .fetch_optional(&mut *conn).await?;
        ensure!(
            saved == Some(self.detector().vehicle_id()),
            "charge checkpoint ownership changed"
        );
        Ok(value)
    }

    pub(super) fn acknowledge(&mut self, value: serde_json::Value) {
        self.persisted = Some(value);
        self.pending.clear();
    }

    pub(super) async fn verify_reacquired_lock(
        &self,
        conn: &mut PgConnection,
    ) -> anyhow::Result<()> {
        let stored: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT snapshot FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
        )
        .bind(self.detector().vehicle_id())
        .fetch_optional(conn)
        .await?;
        ensure!(
            stored == self.persisted || stored == Some(serde_json::to_value(&self.checkpoint)?),
            "charge checkpoint advanced under another collector; refusing stale write"
        );
        Ok(())
    }
}

pub(super) async fn drain_one(pool: &PgPool, vehicle_id: Uuid) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    crate::services::charge_sessions::lock_charge_history(&mut tx, vehicle_id).await?;
    let pending: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT session_id,session FROM panda.pending_charge_completions \
         WHERE vehicle_id=$1 ORDER BY created_at,session_id LIMIT 1 FOR UPDATE SKIP LOCKED",
    )
    .bind(vehicle_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((session_id, value)) = pending else {
        return Ok(false);
    };
    let completion: Completion = serde_json::from_value(value)?;
    ensure!(
        completion.version == 1,
        "unsupported pending charge completion version"
    );
    let session = completion.session;
    ensure!(
        session.session_id == session_id && session.vehicle_id == vehicle_id,
        "pending charge identity mismatch"
    );
    super::persist_charge_session_record(
        &mut tx,
        &session,
        session.location_lat,
        session.location_lng,
        &super::MatchedLocation::none(),
    )
    .await?;
    sqlx::query(
        "DELETE FROM panda.pending_charge_completions WHERE session_id=$1 AND vehicle_id=$2",
    )
    .bind(session_id)
    .bind(vehicle_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

pub(super) async fn save_joint(
    conn: &mut PgConnection,
    trips: &mut super::trip_capture::TripCapture,
    charges: &mut ChargeCapture,
) -> anyhow::Result<()> {
    let mut tx = conn.begin().await?;
    crate::services::charge_sessions::lock_charge_history(&mut tx, charges.detector().vehicle_id())
        .await?;
    let trip_value = trips.write(&mut tx).await?;
    let charge_value = charges.write(&mut tx).await?;
    tx.commit().await?;
    trips.acknowledge(trip_value);
    charges.acknowledge(charge_value);
    Ok(())
}

pub(super) async fn initialize(
    conn: &mut PgConnection,
    trips: &mut super::trip_capture::TripCapture,
    vehicle_id: Uuid,
) -> anyhow::Result<ChargeCapture> {
    let mut tx = conn.begin().await?;
    crate::services::charge_sessions::lock_charge_history(&mut tx, vehicle_id).await?;
    let mut charges = match ChargeCapture::load_on(&mut tx, vehicle_id).await? {
        Some(capture) => capture,
        None => {
            let detector = match super::load_active_charge_snapshot_on(&mut tx, vehicle_id).await? {
                Some(snapshot) => {
                    tracing::info!(%vehicle_id, "initializing durable charge state from legacy telemetry; historical omissions cannot be reconstructed");
                    ChargeDetectorState::from_snapshot(vehicle_id, snapshot)
                }
                None => ChargeDetectorState::new(vehicle_id),
            };
            ChargeCapture::from_detector(detector)
        }
    };
    let trip_value = trips.write(&mut tx).await?;
    let charge_value = charges.write(&mut tx).await?;
    tx.commit().await?;
    trips.acknowledge(trip_value);
    charges.acknowledge(charge_value);
    Ok(charges)
}

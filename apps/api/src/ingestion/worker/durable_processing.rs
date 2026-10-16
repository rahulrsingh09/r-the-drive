//! Ordered processing of one envelope under the collector's session lock.
use super::state_decisions::{EnvelopeRoute, SampleSource};
use super::*;
use crate::ingestion::charge_detector::CompletedChargeSession;

pub(super) struct CollectorIo {
    pub vehicle_id: Uuid,
    pub pool: PgPool,
    pub config: Config,
    pub redis: redis::aio::MultiplexedConnection,
    pub power_state: watch::Sender<Option<PowerState>>,
    pub charging: watch::Sender<bool>,
    pub active_session: watch::Sender<ActiveSessionContext>,
}

pub(super) enum ProcessOutcome {
    Continue,
    ChargeEnded(Box<CompletedChargeSession>),
    CheckpointUnavailable,
}

pub(super) struct CollectorPipeline {
    io: CollectorIo,
    intake: IntakeStage,
    trips: TripStage,
    charges: ChargeStage,
    telemetry: TelemetryStage,
    lifecycle: LifecycleStage,
}

impl CollectorPipeline {
    pub(super) fn new(
        io: CollectorIo,
        capture: trip_capture::TripCapture,
        charge_capture: charge_capture::ChargeCapture,
        active_period: Option<ActiveStatePeriod>,
    ) -> Self {
        let vehicle_id = io.vehicle_id;
        Self {
            io,
            intake: IntakeStage::new(vehicle_id),
            trips: TripStage {
                capture,
                fusion: TripSignalFusion::default(),
            },
            charges: ChargeStage {
                capture: charge_capture,
            },
            telemetry: TelemetryStage {
                gate: TelemetryPersistenceGate::new(vehicle_id),
                motion: ClosureMotionTracker::default(),
            },
            lifecycle: LifecycleStage {
                active_period,
                software: None,
                available_version: None,
            },
        }
    }

    pub(super) async fn process(
        &mut self,
        inbound: WsInboundEvent,
        lock: &mut PgConnection,
        shutdown: &mut broadcast::Receiver<()>,
        last_ws_inbound_at: &mut tokio::time::Instant,
    ) -> ProcessOutcome {
        let source = SampleSource::of(&inbound);
        let capturing = ingestion_capture::is_capturing(self.io.vehicle_id);
        self.intake
            .observe(&self.io, &inbound, source, capturing, last_ws_inbound_at)
            .await;
        match EnvelopeRoute::of(&inbound) {
            EnvelopeRoute::Projection => {
                if let Some(update) = inbound.charging_session.as_ref() {
                    apply_charging_session_update(
                        &self.io.pool,
                        &mut self.io.redis,
                        self.io.vehicle_id,
                        update,
                        inbound.received_at,
                        self.charges.capture.detector().active_session_id(),
                    )
                    .await;
                }
                ProcessOutcome::Continue
            }
            EnvelopeRoute::Heartbeat => {
                let event = inbound
                    .telemetry
                    .as_ref()
                    .expect("heartbeat route has telemetry");
                upsert_seen(
                    &self.io.pool,
                    self.io.vehicle_id,
                    event.is_online.unwrap_or(true),
                    event.ts,
                    inbound.received_at,
                    SeenKind::Heartbeat,
                )
                .await;
                ProcessOutcome::Continue
            }
            EnvelopeRoute::Telemetry => {
                let frame = TelemetryFrame {
                    raw: inbound.telemetry.expect("telemetry route has a sample"),
                    received_at: inbound.received_at,
                    message_type: inbound.message_type,
                    charging_session: inbound.charging_session,
                    source,
                    capturing,
                };
                let staged = self.trips.prepare(&self.io, frame);
                let associated = match self.charges.associate(staged) {
                    Ok(sample) => sample,
                    Err(error) => {
                        tracing::error!(vehicle_id=%self.io.vehicle_id, %error, "charge observation could not be staged");
                        return ProcessOutcome::CheckpointUnavailable;
                    }
                };
                if !save_collector_checkpoint(
                    &self.io.pool,
                    self.io.vehicle_id,
                    lock,
                    &mut self.trips.capture,
                    &mut self.charges.capture,
                    shutdown,
                )
                .await
                {
                    return ProcessOutcome::CheckpointUnavailable;
                }
                self.charges
                    .publish_context(&associated, &self.io.active_session);
                self.charges.materialize(&mut self.io, &associated).await;
                let persistence = self
                    .telemetry
                    .publish_and_store(&mut self.io, &associated, &mut self.intake.counters)
                    .await;
                self.lifecycle
                    .observe(&self.io, &associated, persistence)
                    .await;
                match associated.charge_event {
                    ChargeEvent::SessionEnded(session) => {
                        let live_key = rivian_poll::live_session_redis_key(self.io.vehicle_id);
                        if let Err(error) = self.io.redis.del::<_, ()>(&live_key).await {
                            tracing::debug!(vehicle_id=%self.io.vehicle_id, err=%error, "live session Redis delete on canonical end failed");
                        }
                        if let Err(error) =
                            charge_capture::drain_one(&self.io.pool, self.io.vehicle_id).await
                        {
                            tracing::warn!(vehicle_id=%self.io.vehicle_id, session_id=%session.session_id, %error, "charge completion persistence failed; durable retry retained");
                            trip_capture::record_health(
                                &self.io.pool,
                                self.io.vehicle_id,
                                Some("charge_completion_retry_pending"),
                            )
                            .await;
                        }
                        if let Err(error) = persist_charge_session(&self.io.pool, &session).await {
                            tracing::warn!(vehicle_id=%self.io.vehicle_id, session_id=%session.session_id, error=%error, "worker.persist_charge_session_failed");
                        }
                        ProcessOutcome::ChargeEnded(Box::new(session))
                    }
                    _ => ProcessOutcome::Continue,
                }
            }
        }
    }

    pub(super) async fn retry_completion(&self) {
        let charge_error = match charge_capture::drain_one(&self.io.pool, self.io.vehicle_id).await
        {
            Ok(_) => None,
            Err(error) => {
                tracing::error!(vehicle_id=%self.io.vehicle_id, %error, "charge completion retry failed; durable event retained");
                Some("charge_completion_retry_pending")
            }
        };
        let error = match trip_capture::drain_one(&self.io.pool, self.io.vehicle_id).await {
            Ok(_) => None,
            Err(error) => {
                tracing::error!(vehicle_id=%self.io.vehicle_id, %error, "completed trip persistence failed; durable retry retained");
                Some("completion_retry_pending")
            }
        };
        trip_capture::record_health(&self.io.pool, self.io.vehicle_id, error.or(charge_error))
            .await;
    }

    pub(super) async fn reconcile(&mut self) {
        match reconcile_state_period_tail(
            &self.io.pool,
            self.io.vehicle_id,
            self.lifecycle.active_period.clone(),
        )
        .await
        {
            Ok(period) => self.lifecycle.active_period = period,
            Err(error) => {
                tracing::warn!(vehicle_id=%self.io.vehicle_id, err=%error, "state-period tail reconciliation failed")
            }
        }
    }

    pub(super) async fn flush(&mut self) {
        self.intake.flush(&self.io.pool, self.io.vehicle_id).await;
    }
}

struct TelemetryFrame {
    raw: TelemetryEvent,
    received_at: DateTime<Utc>,
    message_type: Option<String>,
    charging_session: Option<parser::ChargingSessionEvent>,
    source: SampleSource,
    capturing: bool,
}

// A staged frame is published only after both detector checkpoints commit.
struct StagedTelemetry {
    frame: TelemetryFrame,
    trip_id: Option<Uuid>,
}

struct AssociatedTelemetry {
    checkpointed: StagedTelemetry,
    charge_event: ChargeEvent,
    session_id: Option<Uuid>,
    charge_active: bool,
}

struct TripStage {
    capture: trip_capture::TripCapture,
    fusion: TripSignalFusion,
}

impl TripStage {
    fn prepare(&mut self, io: &CollectorIo, frame: TelemetryFrame) -> StagedTelemetry {
        let replay = self.capture.is_replay(frame.raw.ts);
        let (sample, fusion) = if replay {
            (frame.raw.clone(), FusionDiagnostics::default())
        } else {
            self.fusion
                .fuse_from(&frame.raw, frame.source.is_parallax())
        };
        let decision = self.capture.detector().start_decision(&sample);
        let transition = self.capture.process(&sample);
        if frame.capturing {
            record_trip_diagnostics(
                io.vehicle_id,
                &frame.raw,
                &sample,
                &fusion,
                self.capture.detector().active_trip_id().is_some(),
                decision,
                &transition,
            );
        }
        let trip_id = match (&transition, replay) {
            (_, true) => None,
            (TripEvent::TripEnded { trip }, false) => Some(trip.trip_id),
            _ => self.capture.detector().active_trip_id(),
        };
        StagedTelemetry { frame, trip_id }
    }
}

struct ChargeStage {
    capture: charge_capture::ChargeCapture,
}

impl ChargeStage {
    fn associate(&mut self, checkpointed: StagedTelemetry) -> anyhow::Result<AssociatedTelemetry> {
        let frame = &checkpointed.frame;
        let transition = self
            .capture
            .process(&frame.raw, frame.source, frame.received_at)?;
        let session_id = match &transition {
            ChargeEvent::SessionEnded(session) => Some(session.session_id),
            _ => self.capture.detector().active_session_id(),
        };
        Ok(AssociatedTelemetry {
            checkpointed,
            charge_event: transition,
            session_id,
            charge_active: self.capture.detector().active_session_id().is_some(),
        })
    }

    fn publish_context(
        &self,
        sample: &AssociatedTelemetry,
        signal: &watch::Sender<ActiveSessionContext>,
    ) {
        let context = match &sample.charge_event {
            ChargeEvent::SessionEnded(session) => ActiveSessionContext {
                session_id: None,
                started_at: Some(session.started_at),
                ended_at: Some(session.ended_at),
            },
            _ => ActiveSessionContext {
                session_id: self.capture.detector().active_session_id(),
                started_at: self
                    .capture
                    .detector()
                    .active_snapshot()
                    .map(|active| active.started_at),
                ended_at: None,
            },
        };
        let _ = signal.send(context);
    }

    async fn materialize(&self, io: &mut CollectorIo, sample: &AssociatedTelemetry) {
        let frame = &sample.checkpointed.frame;
        if let Some(session_id) = sample.session_id.filter(|_| frame.source.is_parallax()) {
            if let Err(error) =
                ensure_active_charge_session(&io.pool, io.vehicle_id, session_id, &frame.raw).await
            {
                tracing::warn!(vehicle_id=%io.vehicle_id, charge_session_id=%session_id, err=%error, "active charge session materialization failed");
            }
        }
        if let Some(update) = frame.charging_session.as_ref() {
            apply_charging_session_update(
                &io.pool,
                &mut io.redis,
                io.vehicle_id,
                update,
                frame.received_at,
                sample.session_id,
            )
            .await;
        }
    }
}

struct TelemetryStage {
    gate: TelemetryPersistenceGate,
    motion: ClosureMotionTracker,
}

impl TelemetryStage {
    async fn publish_and_store(
        &mut self,
        io: &mut CollectorIo,
        sample: &AssociatedTelemetry,
        counters: &mut CounterBatch,
    ) -> PersistenceDecision {
        let frame = &sample.checkpointed.frame;
        let event = &frame.raw;
        let motion = self.motion.observe(event, Utc::now());
        let snapshot = build_snapshot(event, motion.as_ref());
        let topic = format!("vehicle:{}:status", io.vehicle_id);
        if let Err(error) = io.redis.publish::<_, _, ()>(&topic, &snapshot).await {
            tracing::debug!(vehicle_id=%io.vehicle_id, err=%error, "redis publish failed");
        }
        upsert_seen(
            &io.pool,
            io.vehicle_id,
            event.is_online.unwrap_or(true),
            event.ts,
            frame.received_at,
            SeenKind::Payload,
        )
        .await;
        if let Err(error) = upsert_latest_status(&io.pool, event).await {
            tracing::warn!(vehicle_id=%io.vehicle_id, err=%error, "latest-status upsert failed");
        }
        let selection = io
            .config
            .rivian_suppress_duplicate_telemetry
            .then(|| self.gate.select(event));
        let decision = selection
            .as_ref()
            .map_or(PersistenceDecision::Persist, |selection| selection.decision);
        match decision {
            PersistenceDecision::Persist => {
                if write_telemetry(
                    &io.pool,
                    event,
                    sample.checkpointed.trip_id,
                    sample.session_id,
                )
                .await
                .is_ok()
                {
                    if let Some(selection) = selection {
                        self.gate.advance(selection);
                    }
                    counters.increment("telemetry_writes_persisted");
                    mark_persisted(&io.pool, io.vehicle_id, event.ts).await;
                }
            }
            PersistenceDecision::Suppress(reason) => {
                if let Some(selection) = selection {
                    self.gate.advance(selection);
                }
                counters.increment("telemetry_writes_suppressed");
                counters.increment(reason.counter_column());
            }
        }
        decision
    }
}

struct LifecycleStage {
    active_period: Option<ActiveStatePeriod>,
    software: Option<(String, DateTime<Utc>)>,
    available_version: Option<String>,
}

impl LifecycleStage {
    async fn observe(
        &mut self,
        io: &CollectorIo,
        sample: &AssociatedTelemetry,
        persistence: PersistenceDecision,
    ) {
        let frame = &sample.checkpointed.frame;
        let event = &frame.raw;
        let updates = lifecycle_updates(event, frame.source.is_parallax(), sample.charge_active);
        if updates.state_period {
            match transition_state_period(
                &io.pool,
                io.vehicle_id,
                self.active_period.clone(),
                infer_vehicle_state(event),
                event.ts,
            )
            .await
            {
                Ok(period) => self.active_period = period,
                Err(error) => {
                    tracing::warn!(vehicle_id=%io.vehicle_id, event_ts=%event.ts, err=%error, "state-period transition failed; durable state will be retried")
                }
            }
        }
        if updates.power_state {
            let _ = io.power_state.send(event.power_state.clone());
        }
        if updates.charging {
            let _ = io.charging.send(event.is_actively_charging());
        }
        if frame.capturing {
            ingestion_capture::record(
                io.vehicle_id,
                CaptureKind::Ingestion,
                ingestion_capture_fields(
                    event,
                    frame.message_type.as_deref(),
                    frame.source.is_parallax(),
                    &sample.charge_event,
                    &persistence,
                    updates,
                    sample.charge_active,
                ),
            );
        }
        if let Some(version) = &event.ota_current_version {
            if self.software.as_ref().map(|(old, _)| old) != Some(version) {
                if let Some((previous, _)) = self.software.take() {
                    close_software_version(&io.pool, io.vehicle_id, &previous, event.ts).await;
                }
                open_software_version(&io.pool, io.vehicle_id, version, event.ts).await;
                self.software = Some((version.clone(), event.ts));
            }
        }
        if let Some(version) = &event.ota_available_version {
            if self.available_version.as_ref() != Some(version) {
                tracing::info!(vehicle_id=%io.vehicle_id, version=%version, "OTA available version changed");
                self.available_version = Some(version.clone());
            }
        }
    }
}

struct IntakeStage {
    counters: CounterBatch,
    events: u64,
    legacy: LegacyDiagnostics,
}

struct LegacyDiagnostics {
    counts: [i64; 5],
    class: Option<LegacyChargingSessionClass>,
    frame_at: Option<DateTime<Utc>>,
    meaningful_at: Option<DateTime<Utc>>,
    last_log: tokio::time::Instant,
}

impl IntakeStage {
    fn new(vehicle_id: Uuid) -> Self {
        Self {
            counters: CounterBatch::new(vehicle_id),
            events: 0,
            legacy: LegacyDiagnostics {
                counts: [0; 5],
                class: None,
                frame_at: None,
                meaningful_at: None,
                last_log: tokio::time::Instant::now() - std::time::Duration::from_secs(60),
            },
        }
    }

    async fn flush(&mut self, pool: &PgPool, vehicle_id: Uuid) {
        self.counters.flush(pool).await;
        flush_legacy_charging_diagnostics(
            pool,
            vehicle_id,
            &mut self.legacy.counts,
            self.legacy.class,
            self.legacy.frame_at,
            self.legacy.meaningful_at,
        )
        .await;
    }

    async fn observe(
        &mut self,
        io: &CollectorIo,
        inbound: &WsInboundEvent,
        source: SampleSource,
        capturing: bool,
        last_ws_inbound_at: &mut tokio::time::Instant,
    ) {
        if capturing && !source.is_parallax() {
            if let Some((kind, fields)) = legacy_capture_fields(inbound) {
                ingestion_capture::record(io.vehicle_id, kind, fields);
            }
        }
        if source.accounts_for_socket() {
            *last_ws_inbound_at = tokio::time::Instant::now();
            handle_inbound_accounting(
                &io.pool,
                io.vehicle_id,
                &io.config,
                inbound,
                &mut self.counters,
            )
            .await;
        }
        self.events += 1;
        if self.events.is_multiple_of(500) {
            cleanup_raw_events(&io.pool, io.config.rivian_raw_event_retention_days).await;
        }
        if self.events.is_multiple_of(50) {
            self.flush(&io.pool, io.vehicle_id).await;
        }
        self.legacy.observe(io.vehicle_id, inbound);
        if let Some(cell_type) = inbound.battery_cell_type.as_deref() {
            if let Err(error) = upsert_battery_cell_type(&io.pool, io.vehicle_id, cell_type).await {
                tracing::warn!(vehicle_id=%io.vehicle_id, err=%error, "battery cell type upsert failed");
            }
        }
    }
}

impl LegacyDiagnostics {
    fn observe(&mut self, vehicle_id: Uuid, inbound: &WsInboundEvent) {
        if inbound.kind != WsInboundKind::ChargingSession {
            return;
        }
        let Ok(raw) = serde_json::from_str::<serde_json::Value>(&inbound.raw) else {
            return;
        };
        let class = classify_legacy_charging_session(raw.pointer("/payload/data/chargingSession"));
        self.counts[match class {
            LegacyChargingSessionClass::Null => 0,
            LegacyChargingSessionClass::Missing => 1,
            LegacyChargingSessionClass::Malformed => 2,
            LegacyChargingSessionClass::AllNull => 3,
            LegacyChargingSessionClass::Meaningful => 4,
        }] += 1;
        self.frame_at = Some(inbound.received_at);
        if class == LegacyChargingSessionClass::Meaningful {
            self.meaningful_at = Some(inbound.received_at);
        }
        if self.class != Some(class)
            && self.last_log.elapsed() >= std::time::Duration::from_secs(60)
        {
            tracing::debug!(%vehicle_id, classification=class.as_str(), "legacy chargingSession frame classification changed");
            self.last_log = tokio::time::Instant::now();
        }
        self.class = Some(class);
    }
}

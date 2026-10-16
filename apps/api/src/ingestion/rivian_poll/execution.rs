//! Ordered startup execution and adaptive periodic polling.
use super::*;
use crate::models::telemetry::PowerState;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupStage {
    Refresh,
    Baseline,
    Enrichment,
    Wallboxes,
    History,
    Schedule,
    Complete,
}

impl StartupStage {
    fn next(self) -> Self {
        match self {
            Self::Refresh => Self::Baseline,
            Self::Baseline => Self::Enrichment,
            Self::Enrichment => Self::Wallboxes,
            Self::Wallboxes => Self::History,
            Self::History => Self::Schedule,
            Self::Schedule | Self::Complete => Self::Complete,
        }
    }
}

pub(super) struct VehiclePoll<'a> {
    pub vehicle_id: Uuid,
    pub pool: &'a PgPool,
    pub client: &'a reqwest::Client,
    pub age_key: &'a str,
}

impl VehiclePoll<'_> {
    pub async fn startup(&self, user_id: Uuid, baseline_tx: &mpsc::Sender<WsInboundEvent>) {
        let Self {
            vehicle_id,
            pool,
            client,
            age_key,
        } = *self;
        let mut stage = StartupStage::Refresh;
        tracing::info!(%vehicle_id, "startup polls begin");
        loop {
            match stage {
                StartupStage::Refresh => {
                    if let Ok((_, tokens)) = load_vehicle_tokens(vehicle_id, pool, age_key).await {
                        if let Err(error) =
                            try_refresh_csrf(vehicle_id, &tokens, client, pool, age_key).await
                        {
                            tracing::debug!(%vehicle_id, err=%error, "boot CSRF rotation failed (will retry on first poll)");
                        }
                    }
                }
                StartupStage::Baseline => self.baseline(baseline_tx).await,
                StartupStage::Enrichment => {
                    let result =
                        fetch_vehicle_enrichment_for_vehicle(vehicle_id, pool, client, age_key)
                            .await;
                    self.report("vehicle_enrichment", &result, serde_json::Value::Null);
                }
                StartupStage::Wallboxes => {
                    let result =
                        fetch_wallboxes_for_vehicle(user_id, vehicle_id, pool, client, age_key)
                            .await;
                    self.report("wallboxes", &result, serde_json::Value::Null);
                }
                StartupStage::History => self.startup_history().await,
                StartupStage::Schedule => {
                    let result =
                        fetch_charging_schedule_for_vehicle(vehicle_id, pool, client, age_key)
                            .await;
                    self.report("charging_schedule", &result, serde_json::Value::Null);
                }
                StartupStage::Complete => break,
            }
            stage = stage.next();
        }
        tracing::info!(%vehicle_id, "startup polls complete");
    }

    fn report<T>(&self, operation: &str, result: &Result<T>, detail: serde_json::Value) {
        capture_poll(self.vehicle_id, operation, result, detail);
        if let Err(error) = result {
            if is_auth_error(error) {
                tracing::info!(vehicle_id=%self.vehicle_id, operation, err=%error, "poll skipped: authentication required");
            } else {
                tracing::warn!(vehicle_id=%self.vehicle_id, operation, err=%error, "poll failed");
            }
        }
    }

    async fn baseline(&self, baseline_tx: &mpsc::Sender<WsInboundEvent>) {
        let Self {
            vehicle_id,
            pool,
            client,
            age_key,
        } = *self;
        let baseline = with_vehicle_auth_retry(
            vehicle_id,
            pool,
            client,
            age_key,
            "GetVehicleState",
            |rivian_id, tokens, _, client| {
                Box::pin(async move {
                    ReadOperation::Baseline
                        .execute::<serde_json::Value>(client, tokens, rivian_id)
                        .await
                })
            },
        )
        .await;
        capture_poll(
            vehicle_id,
            "vehicle_state_baseline",
            &baseline,
            serde_json::Value::Null,
        );
        match baseline {
            Ok(data) => {
                increment_poll_counter(pool, vehicle_id).await;
                let raw = serde_json::json!({"type":"next","payload":{"data":data}}).to_string();
                if let Ok(Some(event)) = parser::parse_ws_message(&raw, vehicle_id) {
                    let baseline_event = WsInboundEvent {
                        kind: WsInboundKind::Telemetry,
                        received_at: Utc::now(),
                        // Kept so an ingestion capture records the baseline's
                        // fields and their source timestamps.
                        raw,
                        message_type: Some("baseline".into()),
                        telemetry: Some(event),
                        charging_session: None,
                        battery_cell_type: None,
                    };
                    if let Err(error) = baseline_tx.try_send(baseline_event) {
                        let count =
                            BASELINE_FORWARD_DROP_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                        let reason = match &error {
                            mpsc::error::TrySendError::Full(_) => "full",
                            mpsc::error::TrySendError::Closed(_) => "closed",
                        };
                        tracing::debug!(
                            vehicle_id=%vehicle_id,
                            source="vehicle_state_baseline",
                            reason,
                            "vehicle-state baseline could not reach canonical worker"
                        );
                        if count == 1 || count.is_power_of_two() {
                            tracing::warn!(
                                vehicle_id=%vehicle_id,
                                source="vehicle_state_baseline",
                                reason,
                                sampled_drop_count=count,
                                "vehicle-state baseline could not reach canonical worker"
                            );
                        }
                    }
                }
            }
            Err(error) => {
                tracing::debug!(vehicle_id=%vehicle_id, err=%error, "vehicle-state baseline unavailable")
            }
        }
    }

    async fn startup_history(&self) {
        let Self {
            vehicle_id,
            pool,
            client,
            age_key,
        } = *self;
        // Check whether this vehicle has already been fully backfilled.
        let needs_full_backfill: bool = sqlx::query_scalar(
            "SELECT history_backfilled_at IS NULL FROM panda.vehicles WHERE id = $1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(true);

        if needs_full_backfill {
            tracing::info!(vehicle_id=%vehicle_id, "starting full charge history backfill");

            match charge_backfill::run(pool, client, age_key, vehicle_id).await {
                Ok(count) => {
                    tracing::info!(vehicle_id=%vehicle_id, count, "full backfill complete");
                }
                Err(ChargeBackfillError::AlreadyRunning) => {
                    tracing::info!(vehicle_id=%vehicle_id, "full backfill already running; running incremental charge sync");
                    match fetch_charge_history_for_vehicle(vehicle_id, pool, client, age_key).await
                    {
                        Ok(n) => {
                            tracing::info!(vehicle_id=%vehicle_id, enriched=%n, "incremental charge history sync complete")
                        }
                        Err(e) => {
                            if is_auth_error(&e) {
                                tracing::info!(vehicle_id=%vehicle_id, err=%e, "fetch_charge_history skipped: authentication required")
                            } else {
                                tracing::warn!(vehicle_id=%vehicle_id, err=%e, "fetch_charge_history failed")
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(vehicle_id=%vehicle_id, err=%e, "full backfill failed");
                }
            }
        } else {
            // Incremental enrich: just reconcile any sessions that appeared since last run.
            let history = fetch_charge_history_for_vehicle(vehicle_id, pool, client, age_key).await;
            let detail = history.as_ref().map_or(
                serde_json::Value::Null,
                |n| serde_json::json!({ "reconciled": n }),
            );
            capture_poll(vehicle_id, "charge_history", &history, detail);
            match history {
                Ok(n) => {
                    tracing::info!(vehicle_id=%vehicle_id, enriched=%n, "incremental charge history sync complete")
                }
                Err(e) => {
                    if is_auth_error(&e) {
                        tracing::info!(vehicle_id=%vehicle_id, err=%e, "fetch_charge_history skipped: authentication required")
                    } else {
                        tracing::warn!(vehicle_id=%vehicle_id, err=%e, "fetch_charge_history failed")
                    }
                }
            }
        }
    }

    pub async fn periodic(&self, signals: PollLoopSignals) {
        let PollLoopSignals {
            mut power_state_rx,
            mut charging_rx,
            mut shutdown,
        } = signals;
        let mut cadence = PollCadence::new();
        let mut state = PeriodicState::Waiting;
        loop {
            state = match state {
                PeriodicState::Waiting => {
                    let delay =
                        PollCadence::delay(power_state_rx.borrow().as_ref(), *charging_rx.borrow());
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = power_state_rx.changed() => {},
                        _ = charging_rx.changed() => {},
                        _ = shutdown.recv() => return,
                    }
                    if cadence.due(power_state_rx.borrow().as_ref()) {
                        PeriodicState::Syncing
                    } else {
                        PeriodicState::Waiting
                    }
                }
                PeriodicState::Syncing => {
                    cadence.attempted();
                    let result = fetch_charge_history_for_vehicle(
                        self.vehicle_id,
                        self.pool,
                        self.client,
                        self.age_key,
                    )
                    .await;
                    let detail = result.as_ref().map_or(
                        serde_json::Value::Null,
                        |n| serde_json::json!({"reconciled":n}),
                    );
                    self.report("charge_history_periodic", &result, detail);
                    PeriodicState::Waiting
                }
            };
        }
    }
}

enum PeriodicState {
    Waiting,
    Syncing,
}
struct PollCadence {
    last_attempt: tokio::time::Instant,
}
impl PollCadence {
    fn new() -> Self {
        Self {
            last_attempt: tokio::time::Instant::now(),
        }
    }
    fn delay(power: Option<&PowerState>, charging: bool) -> Duration {
        if charging {
            Duration::from_secs(30)
        } else {
            crate::ingestion::poller::poll_interval(power)
        }
    }
    fn due(&self, power: Option<&PowerState>) -> bool {
        let interval = match power {
            Some(PowerState::Charging | PowerState::Ready) => 300,
            _ => 1800,
        };
        self.last_attempt.elapsed() >= Duration::from_secs(interval)
    }
    fn attempted(&mut self) {
        self.last_attempt = tokio::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn provider_contract_periodic_deadlines_follow_power_and_attempt_time() {
        let mut cadence = PollCadence::new();
        assert_eq!(PollCadence::delay(None, true), Duration::from_secs(30));
        for power in [None, Some(PowerState::Ready), Some(PowerState::Charging)] {
            assert_eq!(
                PollCadence::delay(power.as_ref(), false),
                crate::ingestion::poller::poll_interval(power.as_ref())
            );
        }
        tokio::time::advance(Duration::from_secs(299)).await;
        assert!(!cadence.due(Some(&PowerState::Ready)));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(cadence.due(Some(&PowerState::Ready)));
        assert!(cadence.due(Some(&PowerState::Charging)));
        assert!(!cadence.due(None));
        cadence.attempted();
        assert!(!cadence.due(Some(&PowerState::Charging)));
        tokio::time::advance(Duration::from_secs(1800)).await;
        assert!(cadence.due(None));
    }
}

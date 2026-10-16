use super::*;

pub(super) async fn flush_legacy_charging_diagnostics(
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

/// Batches stewardship counter increments and flushes them in a single upsert.
pub(super) struct CounterBatch {
    vehicle_id: Uuid,
    pub(super) counts: std::collections::BTreeMap<&'static str, i64>,
}

impl CounterBatch {
    pub(super) fn new(vehicle_id: Uuid) -> Self {
        Self {
            vehicle_id,
            counts: std::collections::BTreeMap::new(),
        }
    }

    pub(super) fn increment(&mut self, column: &str) {
        if let Some(col) = stewardship_counter_column(column) {
            *self.counts.entry(col).or_insert(0) += 1;
        }
    }

    /// Flush accumulated counts to the DB and reset the batch for reuse.
    pub(super) async fn flush(&mut self, pool: &PgPool) {
        if self.counts.is_empty() {
            return;
        }
        let _ = self.write(pool).await;
        self.counts.clear();
    }
    async fn write(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
            "INSERT INTO panda.rivian_stewardship_counters(vehicle_id,day",
        );
        for column in self.counts.keys() {
            query.push(",").push(*column);
        }
        query
            .push(") VALUES(")
            .push_bind(self.vehicle_id)
            .push(",CURRENT_DATE");
        for count in self.counts.values() {
            query.push(",").push_bind(*count);
        }
        query.push(") ON CONFLICT(vehicle_id,day) DO UPDATE SET ");
        for (index, column) in self.counts.keys().enumerate() {
            if index > 0 {
                query.push(",");
            }
            query
                .push(*column)
                .push("=panda.rivian_stewardship_counters.")
                .push(*column)
                .push("+EXCLUDED.")
                .push(*column);
        }
        query
            .push(",updated_at=now()")
            .build()
            .execute(pool)
            .await?;
        Ok(())
    }
}

/// Accumulate per-event counters into `batch`. The caller is responsible for
/// flushing `batch` to the DB periodically (every N events) to amortize the
/// cost of the upsert across many messages.
pub(super) async fn handle_inbound_accounting(
    pool: &PgPool,
    vehicle_id: Uuid,
    config: &Config,
    inbound: &WsInboundEvent,
    batch: &mut CounterBatch,
) {
    let plan = AccountingPlan::of(inbound);
    for counter in plan.counters {
        batch.increment(counter);
    }
    if plan.connected {
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

pub(super) async fn persist_raw_event(
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

pub(super) async fn cleanup_raw_events(pool: &PgPool, retention_days: i64) {
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

pub(super) async fn increment_counter(pool: &PgPool, vehicle_id: Uuid, column: &str) {
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

pub(super) fn stewardship_counter_column(column: &str) -> Option<&'static str> {
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

struct AccountingPlan {
    counters: Vec<&'static str>,
    connected: bool,
}
impl AccountingPlan {
    fn of(inbound: &WsInboundEvent) -> Self {
        let mut counters = Vec::with_capacity(3);
        if !is_synthetic_control(inbound.message_type.as_deref()) {
            counters.push("ws_messages_received");
            counters.push(match inbound.kind {
                WsInboundKind::Control => "ws_control_messages_received",
                WsInboundKind::Heartbeat => "ws_heartbeats_received",
                WsInboundKind::Telemetry | WsInboundKind::ChargingSession => {
                    "ws_payload_messages_received"
                }
            });
        }
        let extra = match inbound.message_type.as_deref() {
            Some("connection_ack") => Some("ws_connections_opened"),
            Some("reconnect") => Some("ws_reconnects"),
            Some("connection_init" | "subscribe") => Some("outbound_messages_sent"),
            _ => None,
        };
        counters.extend(extra);
        Self {
            counters,
            connected: inbound.message_type.as_deref() == Some("connection_ack"),
        }
    }
}

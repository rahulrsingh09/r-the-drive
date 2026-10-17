use super::*;

pub(super) async fn flush_legacy_charging_diagnostics(
    pool: &PgPool,
    vehicle_id: Uuid,
    counts: &mut [i64; 5],
    last_class: Option<LegacyChargingSessionClass>,
    last_frame_at: Option<DateTime<Utc>>,
    last_meaningful_at: Option<DateTime<Utc>>,
) {
    if *counts == [0; 5] {
        return;
    }
    let mut row = AcquisitionWrite::new("panda.parallax_collector_state")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field("status", Stored::Text(Some("starting")), MergeWrite::Keep)
        .field("schema_version", Stored::Int(Some(1)), MergeWrite::Keep)
        .field(
            "legacy_last_frame_at",
            Stored::Time(last_frame_at),
            MergeWrite::Greatest,
        )
        .field(
            "legacy_last_meaningful_frame_at",
            Stored::Time(last_meaningful_at),
            MergeWrite::Greatest,
        )
        .field(
            "legacy_last_classification",
            Stored::Text(last_class.map(LegacyChargingSessionClass::as_str)),
            MergeWrite::Replace,
        );
    for (column, count) in [
        "legacy_null_count",
        "legacy_missing_count",
        "legacy_malformed_count",
        "legacy_all_null_count",
        "legacy_meaningful_count",
    ]
    .into_iter()
    .zip(*counts)
    {
        row = row.field(column, Stored::Count(count), MergeWrite::Sum);
    }
    match row.upsert(pool, "(vehicle_id)", "").await {
        Ok(_) => counts.fill(0),
        Err(error) => {
            tracing::debug!(%vehicle_id, err=%error, "legacy charging diagnostics flush failed")
        }
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
        match self.write(pool).await {
            Ok(()) => self.counts.clear(),
            Err(error) => {
                tracing::debug!(vehicle_id=%self.vehicle_id,%error,"stewardship flush failed; batch retained")
            }
        }
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
    let parsed = serde_json::from_str::<serde_json::Value>(&inbound.raw).ok();
    let write = AcquisitionWrite::new("panda.rivian_ws_raw_events")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field(
            "received_at",
            Stored::Time(Some(inbound.received_at)),
            MergeWrite::Keep,
        )
        .field(
            "event_type",
            Stored::Text(Some(inbound.kind.as_str())),
            MergeWrite::Keep,
        )
        .field(
            "message_type",
            Stored::Text(inbound.message_type.as_deref()),
            MergeWrite::Keep,
        )
        .field(
            "payload_json",
            Stored::Json(parsed.as_ref()),
            MergeWrite::Keep,
        )
        .field(
            "payload_text",
            Stored::Text(Some(&inbound.raw)),
            MergeWrite::Keep,
        );
    if write.insert(pool).await.is_ok() {
        batch.increment("raw_events_persisted");
    }
}

pub(super) async fn cleanup_raw_events(pool: &PgPool, retention_days: i64) {
    let Ok(days) = i32::try_from(retention_days.max(1)) else {
        tracing::warn!(
            retention_days,
            "raw retention window cannot be represented; cleanup skipped"
        );
        return;
    };
    if let Err(error) = purge_raw_window(pool, days).await {
        tracing::debug!(%error,"raw retention transaction failed");
    }
}

async fn purge_raw_window(pool: &PgPool, days: i32) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(0x726d5241::bigint)")
        .fetch_one(&mut *tx)
        .await?;
    if !locked {
        return Ok(());
    }
    for statement in [
        "DELETE FROM panda.rivian_ws_raw_events WHERE received_at < now()-($1::int*interval '1 day')",
        "DELETE FROM panda.rivian_parallax_events WHERE received_at < now()-($1::int*interval '1 day')",
    ] {
        sqlx::query("SAVEPOINT raw_retention_table").execute(&mut *tx).await?;
        if let Err(error)=sqlx::query(statement).bind(days).execute(&mut *tx).await {
            sqlx::query("ROLLBACK TO SAVEPOINT raw_retention_table").execute(&mut *tx).await?;
            tracing::debug!(%error,"raw retention table skipped after write failure");
        }
        sqlx::query("RELEASE SAVEPOINT raw_retention_table").execute(&mut *tx).await?;
    }
    tx.commit().await
}

pub(super) async fn increment_counter(pool: &PgPool, vehicle_id: Uuid, column: &str) {
    if let Some(column) = stewardship_counter_column(column) {
        let _ = AcquisitionWrite::new("panda.rivian_stewardship_counters")
            .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
            .field("day", Stored::Sql("CURRENT_DATE"), MergeWrite::Keep)
            .field(column, Stored::Count(1), MergeWrite::Sum)
            .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
            .upsert(pool, "(vehicle_id, day)", "")
            .await;
    }
}

pub(super) fn stewardship_counter_column(column: &str) -> Option<&'static str> {
    const COLUMNS: &[&str] = &[
        "ws_messages_received",
        "ws_heartbeats_received",
        "ws_payload_messages_received",
        "ws_control_messages_received",
        "ws_connections_opened",
        "ws_reconnects",
        "outbound_messages_sent",
        "outbound_graphql_requests",
        "telemetry_writes_persisted",
        "telemetry_writes_suppressed",
        "telemetry_suppressed_duplicate",
        "telemetry_suppressed_empty",
        "telemetry_suppressed_threshold",
        "collector_lock_skips",
        "raw_events_persisted",
    ];
    COLUMNS.iter().copied().find(|allowed| *allowed == column)
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

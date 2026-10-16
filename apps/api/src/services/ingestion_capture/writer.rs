use super::*;

pub(super) async fn insert_marker<'e, E>(
    executor: E,
    vehicle_id: Uuid,
    capture_id: Uuid,
    recorded_at: DateTime<Utc>,
    kind: Kind,
    fields: Value,
) -> sqlx::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO panda.vehicle_ingestion_capture_events \
             (vehicle_id, capture_id, recorded_at, kind, fields) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(vehicle_id)
    .bind(capture_id)
    .bind(recorded_at)
    .bind(kind.as_str())
    .bind(fields)
    .execute(executor)
    .await?;
    Ok(())
}

/// Per-capture stored-row accounting for the writer.
#[derive(Debug, Default)]
struct CapLedger {
    counts: HashMap<Uuid, (u64, bool)>,
}

enum Admit {
    Store,
    /// Store a single truncation marker instead of this record.
    Truncate,
    Drop,
}

impl CapLedger {
    fn is_known(&self, capture_id: Uuid) -> bool {
        self.counts.contains_key(&capture_id)
    }

    fn seed(&mut self, capture_id: Uuid, stored: u64) {
        self.counts.entry(capture_id).or_insert((stored, false));
    }

    fn admit(&mut self, capture_id: Uuid, cap: u64) -> Admit {
        let (count, truncated) = self.counts.entry(capture_id).or_insert((0, false));
        if *count < cap {
            *count += 1;
            Admit::Store
        } else if !*truncated {
            *truncated = true;
            Admit::Truncate
        } else {
            Admit::Drop
        }
    }
}

pub(super) async fn run_writer(pool: PgPool, mut rx: mpsc::Receiver<Record>) {
    let mut ledger = CapLedger::default();
    let mut batch = Vec::with_capacity(BATCH_ROWS);
    let mut flush = tokio::time::interval(FLUSH_INTERVAL);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            record = rx.recv() => {
                let Some(record) = record else { break };
                batch.push(record);
                if batch.len() < BATCH_ROWS {
                    continue;
                }
            }
            _ = flush.tick() => {
                if batch.is_empty() {
                    continue;
                }
            }
        }
        let records = std::mem::take(&mut batch);
        if let Err(error) = write_batch(&pool, &mut ledger, records).await {
            tracing::warn!(err = %error, "ingestion capture batch write failed");
        }
    }
}

async fn write_batch(
    pool: &PgPool,
    ledger: &mut CapLedger,
    records: Vec<Record>,
) -> anyhow::Result<()> {
    for record in &records {
        if !ledger.is_known(record.capture_id) {
            let stored = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM panda.vehicle_ingestion_capture_events WHERE capture_id = $1",
            )
            .bind(record.capture_id)
            .fetch_one(pool)
            .await?;
            ledger.seed(record.capture_id, u64::try_from(stored).unwrap_or(0));
        }
    }

    let mut vehicle_ids = Vec::with_capacity(records.len());
    let mut capture_ids = Vec::with_capacity(records.len());
    let mut recorded_ats = Vec::with_capacity(records.len());
    let mut kinds = Vec::with_capacity(records.len());
    let mut fields = Vec::with_capacity(records.len());
    for record in records {
        let (kind, value) = match ledger.admit(record.capture_id, MAX_EVENTS_PER_CAPTURE) {
            Admit::Store => (record.kind, record.fields),
            Admit::Truncate => (
                Kind::Truncated,
                serde_json::json!({ "max_events": MAX_EVENTS_PER_CAPTURE }),
            ),
            Admit::Drop => continue,
        };
        vehicle_ids.push(record.vehicle_id);
        capture_ids.push(record.capture_id);
        recorded_ats.push(record.recorded_at);
        kinds.push(kind.as_str().to_owned());
        fields.push(value);
    }
    if vehicle_ids.is_empty() {
        return Ok(());
    }
    // Joining the current capture row drops records that were queued for a
    // capture that has since been replaced.
    sqlx::query(
        "INSERT INTO panda.vehicle_ingestion_capture_events \
             (vehicle_id, capture_id, recorded_at, kind, fields) \
         SELECT r.vehicle_id, r.capture_id, r.recorded_at, r.kind, r.fields \
         FROM unnest($1::uuid[], $2::uuid[], $3::timestamptz[], $4::text[], $5::jsonb[]) \
              AS r(vehicle_id, capture_id, recorded_at, kind, fields) \
         JOIN panda.vehicle_ingestion_diagnostics d \
           ON d.vehicle_id = r.vehicle_id AND d.capture_id = r.capture_id",
    )
    .bind(vehicle_ids)
    .bind(capture_ids)
    .bind(recorded_ats)
    .bind(kinds)
    .bind(fields)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn run_expiry(pool: PgPool) {
    let mut interval = tokio::time::interval(EXPIRY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let now = Utc::now();
        let expired: Vec<Uuid> = CAPTURE
            .get()
            .and_then(|capture| capture.windows.read().ok())
            .map(|windows| {
                windows
                    .iter()
                    .filter(|(_, window)| window.ends_at <= now)
                    .map(|(vehicle_id, _)| *vehicle_id)
                    .collect()
            })
            .unwrap_or_default();
        for vehicle_id in expired {
            if let Err(error) = stop(&pool, vehicle_id, StopReason::Expired).await {
                tracing::warn!(vehicle_id = %vehicle_id, err = %error, "ingestion capture expiry failed");
            }
        }
    }
}

pub(super) async fn run_purge(pool: PgPool) {
    let mut interval = tokio::time::interval(PURGE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let result = sqlx::query(
            "WITH purged AS ( \
                 DELETE FROM panda.vehicle_ingestion_diagnostics \
                 WHERE stopped_at IS NOT NULL AND stopped_at < now() - $1::interval \
                 RETURNING vehicle_id) \
             DELETE FROM panda.vehicle_ingestion_capture_events e \
             USING purged WHERE e.vehicle_id = purged.vehicle_id",
        )
        .bind(CAPTURE_RETENTION)
        .execute(&pool)
        .await;
        match result {
            Ok(done) if done.rows_affected() > 0 => {
                tracing::info!(
                    removed = done.rows_affected(),
                    "expired ingestion capture events purged"
                )
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(err = %error, "ingestion capture purge failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_admits_up_to_the_limit_then_one_truncation_marker() {
        let mut ledger = CapLedger::default();
        let capture = Uuid::new_v4();
        ledger.seed(capture, 1);
        assert!(matches!(ledger.admit(capture, 3), Admit::Store));
        assert!(matches!(ledger.admit(capture, 3), Admit::Store));
        assert!(matches!(ledger.admit(capture, 3), Admit::Truncate));
        assert!(matches!(ledger.admit(capture, 3), Admit::Drop));
        assert!(matches!(ledger.admit(capture, 3), Admit::Drop));
        // Another capture has its own budget.
        assert!(matches!(ledger.admit(Uuid::new_v4(), 3), Admit::Store));
    }
}

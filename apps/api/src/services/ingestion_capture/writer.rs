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
        if let std::collections::hash_map::Entry::Vacant(entry) = self.counts.entry(capture_id) {
            entry.insert((stored, false));
        }
    }

    fn admit(&mut self, capture_id: Uuid, cap: u64) -> Admit {
        let budget = self.counts.entry(capture_id).or_default();
        match (budget.0 < cap, budget.1) {
            (true, _) => {
                budget.0 += 1;
                Admit::Store
            }
            (false, false) => {
                budget.1 = true;
                Admit::Truncate
            }
            (false, true) => Admit::Drop,
        }
    }
}

pub(super) async fn run_writer(pool: PgPool, mut rx: mpsc::Receiver<Record>) {
    let mut ledger = CapLedger::default();
    let mut flush = tokio::time::interval(FLUSH_INTERVAL);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let mut records = Vec::with_capacity(BATCH_ROWS);
        while records.len() < BATCH_ROWS {
            tokio::select! {
                record = rx.recv() => match record {
                    Some(record) => records.push(record),
                    None => return,
                },
                _ = flush.tick() => if !records.is_empty() { break; },
            }
        }
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
    let unknown: Vec<_> = records
        .iter()
        .map(|record| record.capture_id)
        .filter(|capture| !ledger.is_known(*capture))
        .collect();
    if !unknown.is_empty() {
        let counts: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT capture_id,count(*) FROM panda.vehicle_ingestion_capture_events WHERE capture_id=ANY($1) GROUP BY capture_id"
        ).bind(&unknown).fetch_all(pool).await?;
        let counts: HashMap<_, _> = counts.into_iter().collect();
        for capture in unknown {
            ledger.seed(
                capture,
                counts
                    .get(&capture)
                    .and_then(|count| u64::try_from(*count).ok())
                    .unwrap_or(0),
            );
        }
    }
    let accepted: Vec<_> = records
        .into_iter()
        .filter_map(
            |mut record| match ledger.admit(record.capture_id, MAX_EVENTS_PER_CAPTURE) {
                Admit::Store => Some(record),
                Admit::Truncate => {
                    record.kind = Kind::Truncated;
                    record.fields = serde_json::json!({"max_events":MAX_EVENTS_PER_CAPTURE});
                    Some(record)
                }
                Admit::Drop => None,
            },
        )
        .collect();
    if accepted.is_empty() {
        return Ok(());
    }
    let vehicles: Vec<_> = accepted.iter().map(|record| record.vehicle_id).collect();
    let captures: Vec<_> = accepted.iter().map(|record| record.capture_id).collect();
    let times: Vec<_> = accepted.iter().map(|record| record.recorded_at).collect();
    let kinds: Vec<_> = accepted.iter().map(|record| record.kind.as_str()).collect();
    let fields: Vec<_> = accepted.into_iter().map(|record| record.fields).collect();
    sqlx::query(
        "INSERT INTO panda.vehicle_ingestion_capture_events(vehicle_id,capture_id,recorded_at,kind,fields)
         SELECT batch.vehicle,batch.capture,batch.at,batch.kind,batch.fields
         FROM unnest($1::uuid[],$2::uuid[],$3::timestamptz[],$4::text[],$5::jsonb[]) AS batch(vehicle,capture,at,kind,fields)
         WHERE EXISTS(SELECT 1 FROM panda.vehicle_ingestion_diagnostics current
                      WHERE current.vehicle_id=batch.vehicle AND current.capture_id=batch.capture)"
    ).bind(vehicles).bind(captures).bind(times).bind(kinds).bind(fields).execute(pool).await?;
    Ok(())
}

pub(super) async fn run_expiry(pool: PgPool) {
    let mut interval = tokio::time::interval(EXPIRY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let Some(capture) = CAPTURE.get() else {
            continue;
        };
        let expired = {
            let Ok(windows) = capture.windows.read() else {
                continue;
            };
            let now = Utc::now();
            windows
                .iter()
                .filter_map(|(id, window)| (window.ends_at <= now).then_some(*id))
                .collect::<Vec<_>>()
        };
        for vehicle_id in expired {
            match stop(&pool, vehicle_id, StopReason::Expired).await {
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(vehicle_id = %vehicle_id, err = %error, "ingestion capture expiry failed")
                }
            }
        }
    }
}

pub(super) async fn run_purge(pool: PgPool) {
    let mut interval = tokio::time::interval(PURGE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let purged = async {
            let mut tx = pool.begin().await?;
            let vehicles: Vec<Uuid> = sqlx::query_scalar(
                "DELETE FROM panda.vehicle_ingestion_diagnostics
                 WHERE stopped_at IS NOT NULL AND stopped_at<now()-$1::interval RETURNING vehicle_id"
            ).bind(CAPTURE_RETENTION).fetch_all(&mut *tx).await?;
            let removed = sqlx::query("DELETE FROM panda.vehicle_ingestion_capture_events WHERE vehicle_id=ANY($1)")
                .bind(vehicles).execute(&mut *tx).await?.rows_affected();
            tx.commit().await?;
            Ok::<_, sqlx::Error>(removed)
        }.await;
        match purged {
            Ok(removed) if removed > 0 => {
                tracing::info!(removed, "expired ingestion capture events purged")
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(err = %error, "ingestion capture purge failed"),
        }
    }
}

#[cfg(test)]
mod domain_replacement_capture_contracts {
    use super::*;
    #[tokio::test]
    #[ignore = "requires dedicated JOBS_DATABASE_URL"]
    async fn cap_and_stale_capture_rows_preserve_export_and_status() {
        use crate::services::weather_enrichment::job_fixture;
        let fixture = job_fixture::Fixture::new().await;
        let pool = &fixture.state.pool;
        let vehicle = job_fixture::vehicle(pool, 701).await;
        start(pool, vehicle, job_fixture::id(10701)).await.unwrap();
        let capture: Uuid = sqlx::query_scalar(
            "SELECT capture_id FROM panda.vehicle_ingestion_diagnostics WHERE vehicle_id=$1",
        )
        .bind(vehicle)
        .fetch_one(pool)
        .await
        .unwrap();
        let mut ledger = CapLedger::default();
        ledger.seed(capture, MAX_EVENTS_PER_CAPTURE - 1);
        let record = |capture_id| Record {
            vehicle_id: vehicle,
            capture_id,
            recorded_at: Utc::now(),
            kind: Kind::Ingestion,
            fields: serde_json::json!({"safe":true}),
        };
        write_batch(
            pool,
            &mut ledger,
            vec![
                record(capture),
                record(capture),
                record(capture),
                record(Uuid::new_v4()),
            ],
        )
        .await
        .unwrap();
        let state = status(pool, vehicle).await.unwrap();
        assert_eq!(state.state, "capturing");
        assert_eq!(state.event_count, 1);
        assert!(state.truncated);
        let rows = export_rows(pool, capture, 0, 100).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].1["kind"], "ingestion");
        assert_eq!(rows[2].1["kind"], "truncated");
        assert_eq!(rows[2].1["fields"]["max_events"], MAX_EVENTS_PER_CAPTURE);
        assert!(stop(pool, vehicle, StopReason::User).await.unwrap());
        assert!(!stop(pool, vehicle, StopReason::User).await.unwrap());
        let state = status(pool, vehicle).await.unwrap();
        assert_eq!(state.state, "stopped");
        assert_eq!(state.stop_reason.as_deref(), Some("user"));
        let header = export(pool, vehicle).await.unwrap().unwrap();
        assert_eq!(header.header["event_count"], 1);
        assert_eq!(header.header["truncated"], true);
        fixture.cleanup().await;
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

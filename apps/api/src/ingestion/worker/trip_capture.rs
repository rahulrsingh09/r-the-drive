//! Durable detector snapshots and a transactional completion outbox.

use crate::ingestion::{
    AcquisitionMerge as MergeWrite, AcquisitionValue as Stored, AcquisitionWrite,
};
use anyhow::{ensure, Context};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use sqlx::Connection;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::ingestion::trip_detector::{
    compute_distance_odometer_or_gps, CompletedTripData, TripDetectorState, TripEvent,
};
use crate::models::telemetry::TelemetryEvent;

const RECOVERY_GAP_SECONDS: i64 = 300;

#[derive(Clone, Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    detector: TripDetectorState,
    last_event_at: Option<DateTime<Utc>>,
}

pub(super) struct TripCapture {
    checkpoint: Checkpoint,
    replay_cutoff: Option<DateTime<Utc>>,
    recovering: bool,
    pending: Vec<CompletedTripData>,
    persisted: Option<serde_json::Value>,
}

impl TripCapture {
    pub(super) async fn load(pool: &PgPool, vehicle_id: Uuid) -> anyhow::Result<Self> {
        let persisted: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT snapshot FROM panda.active_trip_checkpoints WHERE vehicle_id=$1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?;
        let mut checkpoint = persisted
            .as_ref()
            .map(|value| {
                Checkpoint::deserialize(value)
                    .context("invalid trip checkpoint; refusing to overwrite recovery state")
            })
            .transpose()?
            .unwrap_or_else(|| Checkpoint {
                version: 1,
                detector: TripDetectorState::new(vehicle_id),
                last_event_at: None,
            });
        ensure!(
            checkpoint.version == 1,
            "unsupported trip checkpoint version"
        );
        ensure!(
            checkpoint.detector.vehicle_id() == vehicle_id,
            "trip checkpoint vehicle mismatch"
        );
        let replay_cutoff = checkpoint.last_event_at;
        checkpoint.detector.resume_after_restart();
        Ok(Self {
            checkpoint,
            replay_cutoff,
            recovering: true,
            pending: Vec::new(),
            persisted,
        })
    }

    pub(super) fn detector(&self) -> &TripDetectorState {
        &self.checkpoint.detector
    }

    pub(super) fn is_replay(&self, at: DateTime<Utc>) -> bool {
        self.replay_cutoff.is_some_and(|cutoff| at <= cutoff)
    }

    /// The caller must retry save before processing another event.
    pub(super) fn process(&mut self, event: &TelemetryEvent) -> TripEvent {
        match self.replay_cutoff {
            Some(cutoff) if event.ts <= cutoff => TripEvent::NoChange,
            _ => {
                self.recover_stale(event.ts);
                self.recovering = false;
                self.checkpoint.last_event_at = Some(
                    self.checkpoint
                        .last_event_at
                        .into_iter()
                        .chain(std::iter::once(event.ts))
                        .max()
                        .expect("incoming observation"),
                );
                let transition = self.checkpoint.detector.process(event);
                self.pending.extend(match &transition {
                    TripEvent::TripEnded { trip } => Some(trip.clone()),
                    _ => None,
                });
                transition
            }
        }
    }

    /// A stale fragment ends at its last observation, never at restart time.
    pub(super) fn recover_stale(&mut self, now: DateTime<Utc>) {
        let expired_at = self.checkpoint.last_event_at.filter(|last| {
            self.recovering && now - *last > Duration::seconds(RECOVERY_GAP_SECONDS)
        });
        if let Some(last) = expired_at {
            self.recovering = false;
            self.pending.extend(
                match self.checkpoint.detector.close_interrupted_trip(last) {
                    TripEvent::TripEnded { trip } => Some(trip),
                    _ => None,
                },
            );
        }
    }

    #[cfg(test)]
    pub(super) async fn save(&mut self, conn: &mut PgConnection) -> anyhow::Result<()> {
        let mut tx = conn.begin().await?;
        let snapshot = self.write(&mut tx).await?;
        tx.commit().await?;
        self.acknowledge(snapshot);
        Ok(())
    }

    pub(super) async fn write(&self, conn: &mut PgConnection) -> anyhow::Result<serde_json::Value> {
        let snapshot = serde_json::to_value(&self.checkpoint)?;
        for trip in &self.pending {
            let saved: Option<Uuid> = sqlx::query_scalar(
                "INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip) \
                 VALUES($1,$2,$3) ON CONFLICT(trip_id) DO UPDATE SET trip=EXCLUDED.trip \
                 WHERE pending_trip_completions.vehicle_id=EXCLUDED.vehicle_id \
                   AND pending_trip_completions.trip=EXCLUDED.trip RETURNING trip_id",
            )
            .bind(trip.trip_id)
            .bind(trip.vehicle_id)
            .bind(serde_json::to_value(trip)?)
            .fetch_optional(&mut *conn)
            .await?;
            ensure!(
                saved == Some(trip.trip_id),
                "conflicting pending trip; retaining checkpoint"
            );
        }
        let saved: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO panda.active_trip_checkpoints(vehicle_id,snapshot) VALUES($1,$2) \
             ON CONFLICT(vehicle_id) DO UPDATE SET snapshot=$2,updated_at=now() \
             WHERE active_trip_checkpoints.snapshot=$3 OR active_trip_checkpoints.snapshot=$2 \
             RETURNING vehicle_id",
        )
        .bind(self.checkpoint.detector.vehicle_id())
        .bind(&snapshot)
        .bind(&self.persisted)
        .fetch_optional(&mut *conn)
        .await?;
        ensure!(
            saved == Some(self.checkpoint.detector.vehicle_id()),
            "trip checkpoint ownership changed"
        );
        Ok(snapshot)
    }

    pub(super) fn acknowledge(&mut self, snapshot: serde_json::Value) {
        self.persisted = Some(snapshot);
        self.pending.clear();
    }

    pub(super) async fn verify_reacquired_lock(
        &self,
        conn: &mut PgConnection,
    ) -> anyhow::Result<()> {
        let stored: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT snapshot FROM panda.active_trip_checkpoints WHERE vehicle_id=$1",
        )
        .bind(self.checkpoint.detector.vehicle_id())
        .fetch_optional(conn)
        .await?;
        if stored == self.persisted {
            return Ok(());
        }
        match stored {
            Some(value) if value == serde_json::to_value(&self.checkpoint)? => Ok(()),
            _ => anyhow::bail!("checkpoint advanced under another collector; refusing stale write"),
        }
    }
}

pub(super) async fn drain_one(pool: &PgPool, vehicle_id: Uuid) -> anyhow::Result<bool> {
    let pending: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT trip_id,trip FROM panda.pending_trip_completions WHERE vehicle_id=$1 ORDER BY created_at,trip_id LIMIT 1")
        .bind(vehicle_id).fetch_optional(pool).await?;
    let completion = pending
        .map(|(id, value)| -> anyhow::Result<CompletedTripData> {
            let trip: CompletedTripData = serde_json::from_value(value)?;
            ensure!(
                trip.vehicle_id == vehicle_id && trip.trip_id == id,
                "completion identity mismatch"
            );
            Ok(trip)
        })
        .transpose()?;
    match completion {
        None => Ok(false),
        Some(trip) => {
            let distance = compute_distance_odometer_or_gps(
                trip.start_odometer_mi,
                trip.end_odometer_mi,
                &trip.points,
            );
            super::persist_trip(pool, &trip, distance).await?;
            Ok(true)
        }
    }
}

pub(super) async fn record_health(pool: &PgPool, vehicle_id: Uuid, error: Option<&str>) {
    let result = AcquisitionWrite::new("panda.vehicle_runtime_state")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field(
            "collector_heartbeat_at",
            Stored::Sql("now()"),
            MergeWrite::Replace,
        )
        .field(
            "trip_persistence_error",
            Stored::Text(error),
            MergeWrite::Replace,
        )
        .upsert(pool, "(vehicle_id)", "")
        .await;
    if let Err(error) = result {
        tracing::warn!(%vehicle_id, %error, "trip capture health update failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::telemetry::PowerState;
    use sqlx::{postgres::PgConnectOptions, Executor};

    struct TestDb {
        pool: PgPool,
        admin: PgPool,
        name: String,
        vehicle_id: Uuid,
    }

    impl TestDb {
        async fn new() -> Self {
            let url = std::env::var("TRIP_DURABILITY_TEST_DATABASE_URL")
                .expect("set an explicit isolated synthetic database URL");
            let options: PgConnectOptions = url.parse().unwrap();
            let admin = PgPool::connect_with(options.clone().database("postgres"))
                .await
                .unwrap();
            let name = format!("panda_trip_durability_{}", Uuid::new_v4().simple());
            admin
                .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
                .await
                .unwrap();
            let pool = PgPool::connect_with(options.database(&name)).await.unwrap();
            crate::db::migrations::MIGRATOR.run(&pool).await.unwrap();
            let user_id = Uuid::new_v4();
            sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,'synthetic@example.invalid','unused')")
                .bind(user_id).execute(&pool).await.unwrap();
            let vehicle_id = Uuid::new_v4();
            sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,'demo-trip-test','R1T')")
                .bind(vehicle_id).bind(user_id).execute(&pool).await.unwrap();
            Self {
                pool,
                admin,
                name,
                vehicle_id,
            }
        }

        async fn finish(self) {
            self.pool.close().await;
            self.admin
                .execute(sqlx::AssertSqlSafe(format!(
                    "ALTER DATABASE \"{}\" ALLOW_CONNECTIONS false",
                    self.name
                )))
                .await
                .unwrap();
            self.admin
                .execute(sqlx::AssertSqlSafe(format!(
                    "DROP DATABASE \"{}\" WITH (FORCE)",
                    self.name
                )))
                .await
                .unwrap();
            self.admin.close().await;
        }

        async fn counts(&self) -> (i64, i64, i64) {
            sqlx::query_as(
                "SELECT (SELECT count(*) FROM panda.trips), \
                 (SELECT count(*) FROM panda.pending_trip_completions), \
                 (SELECT count(*) FROM panda.weather_enrichment_jobs)",
            )
            .fetch_one(&self.pool)
            .await
            .unwrap()
        }
    }

    fn event(id: Uuid, seconds: i64, odometer: f64, power: PowerState) -> TelemetryEvent {
        let at = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut event = TelemetryEvent::empty(id, at + Duration::seconds(seconds));
        event.speed_mph = Some(if power == PowerState::Sleep {
            0.0
        } else {
            30.0
        });
        event.power_state = Some(power);
        event.odometer_miles = Some(odometer);
        event.latitude = Some(30.0 + seconds as f64 / 100_000.0);
        event.longitude = Some(-97.0);
        event.battery_level = Some(80.0);
        event.regen_power_kw = Some(-2.0);
        event
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn mid_trip_restart_recovers_identity_points_and_avoids_gap_energy() {
        let db = TestDb::new().await;
        let mut conn = db.pool.acquire().await.unwrap();
        let mut capture = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        let first = event(db.vehicle_id, 0, 10.0, PowerState::Drive);
        capture.process(&first);
        let trip_id = capture.detector().active_trip_id().unwrap();
        capture.save(&mut conn).await.unwrap();
        drop(capture);

        let mut recovered = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        assert_eq!(recovered.detector().active_trip_id(), Some(trip_id));
        assert_eq!(recovered.process(&first), TripEvent::NoChange);
        recovered.process(&event(db.vehicle_id, 60, 11.0, PowerState::Drive));
        recovered.save(&mut conn).await.unwrap();
        let TripEvent::TripEnded { trip } =
            recovered.process(&event(db.vehicle_id, 90, 12.0, PowerState::Sleep))
        else {
            panic!("trip should end")
        };
        assert_eq!(trip.trip_id, trip_id);
        assert_eq!(trip.started_at, first.ts);
        assert_eq!(
            trip.points.len(),
            3,
            "replayed frame must not duplicate route points"
        );
        assert!((trip.regen_wh.unwrap() - 2_000.0 * 30.0 / 3_600.0).abs() < 0.001);
        recovered.save(&mut conn).await.unwrap();
        drop(recovered);
        let restarted = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        assert_eq!(restarted.detector().active_trip_id(), None);
        assert_eq!(db.counts().await, (0, 1, 0));
        assert!(drain_one(&db.pool, db.vehicle_id).await.unwrap());
        assert_eq!(db.counts().await, (1, 0, 1));
        assert!(!drain_one(&db.pool, db.vehicle_id).await.unwrap());
        let stored: (Uuid, f64) = sqlx::query_as("SELECT id,distance_miles FROM panda.trips")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(stored, (trip_id, 2.0));
        super::super::write_telemetry(&db.pool, &first, Some(trip_id), None)
            .await
            .unwrap();
        super::super::write_telemetry(&db.pool, &first, Some(Uuid::new_v4()), None)
            .await
            .unwrap();
        let association: Uuid = sqlx::query_scalar(
            "SELECT trip_id FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts=$2",
        )
        .bind(db.vehicle_id)
        .bind(first.ts)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            association, trip_id,
            "replay must not reassign historical telemetry"
        );
        drop(conn);
        db.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn checkpoint_and_completion_failures_retry_atomically_and_idempotently() {
        let db = TestDb::new().await;
        let mut conn = db.pool.acquire().await.unwrap();
        let mut capture = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        capture.process(&event(db.vehicle_id, 0, 10.0, PowerState::Drive));
        capture.save(&mut conn).await.unwrap();
        let TripEvent::TripEnded { trip } =
            capture.process(&event(db.vehicle_id, 60, 11.0, PowerState::Sleep))
        else {
            panic!("trip should end")
        };
        db.pool.execute("ALTER TABLE panda.active_trip_checkpoints ADD CONSTRAINT synthetic_failure CHECK(false) NOT VALID").await.unwrap();
        assert!(capture.save(&mut conn).await.is_err());
        assert_eq!(
            capture.pending.len(),
            1,
            "failed save must retain completion"
        );
        assert_eq!(
            db.counts().await,
            (0, 0, 0),
            "enqueue must roll back with failed checkpoint"
        );
        let prior = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        assert_eq!(prior.detector().active_trip_id(), Some(trip.trip_id));
        db.pool
            .execute("ALTER TABLE panda.active_trip_checkpoints DROP CONSTRAINT synthetic_failure")
            .await
            .unwrap();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
        shutdown_tx.send(()).unwrap();
        assert!(
            super::super::save_trip_checkpoint(
                &db.pool,
                db.vehicle_id,
                &mut conn,
                &mut capture,
                &mut shutdown_rx,
            )
            .await,
            "intentional stop must first attempt to commit the observed completion"
        );
        assert!(
            shutdown_rx.try_recv().is_ok(),
            "the worker must still observe the stop"
        );
        capture.save(&mut conn).await.unwrap();
        assert!(capture.pending.is_empty());
        assert_eq!(db.counts().await, (0, 1, 0));

        db.pool
            .execute(
                "ALTER TABLE panda.trips ADD CONSTRAINT synthetic_failure CHECK(false) NOT VALID",
            )
            .await
            .unwrap();
        assert!(drain_one(&db.pool, db.vehicle_id).await.is_err());
        assert_eq!(db.counts().await, (0, 1, 0));
        db.pool
            .execute("ALTER TABLE panda.trips DROP CONSTRAINT synthetic_failure")
            .await
            .unwrap();
        db.pool.execute("ALTER TABLE panda.weather_enrichment_jobs ADD CONSTRAINT synthetic_failure CHECK(false) NOT VALID").await.unwrap();
        assert!(drain_one(&db.pool, db.vehicle_id).await.is_err());
        assert_eq!(
            db.counts().await,
            (0, 1, 0),
            "trip insert and retry removal must roll back together"
        );
        db.pool
            .execute("ALTER TABLE panda.weather_enrichment_jobs DROP CONSTRAINT synthetic_failure")
            .await
            .unwrap();
        drop(capture);
        assert!(drain_one(&db.pool, db.vehicle_id).await.unwrap());
        assert_eq!(db.counts().await, (1, 0, 1));

        // Replay an uncertain acknowledgement after a user edited the saved trip.
        db.pool
            .execute("UPDATE panda.trips SET distance_miles=42")
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip) VALUES($1,$2,$3)",
        )
        .bind(trip.trip_id)
        .bind(db.vehicle_id)
        .bind(serde_json::to_value(&trip).unwrap())
        .execute(&db.pool)
        .await
        .unwrap();
        assert!(drain_one(&db.pool, db.vehicle_id).await.unwrap());
        assert_eq!(db.counts().await, (1, 0, 1));
        let distance: f64 = sqlx::query_scalar("SELECT distance_miles FROM panda.trips")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(distance, 42.0, "retry must not overwrite stored history");
        drop(conn);
        db.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn stale_restart_closes_only_observed_fragment_and_starts_fresh() {
        let db = TestDb::new().await;
        let mut conn = db.pool.acquire().await.unwrap();
        let mut capture = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        capture.process(&event(db.vehicle_id, 0, 10.0, PowerState::Drive));
        capture.process(&event(db.vehicle_id, 60, 11.0, PowerState::Drive));
        let old_id = capture.detector().active_trip_id().unwrap();
        capture.save(&mut conn).await.unwrap();
        let mut restarted = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        let later = event(db.vehicle_id, 3_600, 50.0, PowerState::Drive);
        restarted.recover_stale(later.ts);
        restarted.process(&later);
        assert_ne!(restarted.detector().active_trip_id(), Some(old_id));
        assert_eq!(restarted.pending.len(), 1);
        let fragment = &restarted.pending[0];
        assert_eq!(fragment.trip_id, old_id);
        assert_eq!(fragment.ended_at, later.ts - Duration::seconds(3_540));
        assert_eq!(fragment.end_odometer_mi, Some(11.0));
        assert_eq!(fragment.points.len(), 2);
        restarted.save(&mut conn).await.unwrap();
        assert!(drain_one(&db.pool, db.vehicle_id).await.unwrap());
        let TripEvent::TripEnded { trip } =
            restarted.process(&event(db.vehicle_id, 3_660, 51.0, PowerState::Sleep))
        else {
            panic!("new trip should end")
        };
        assert_eq!(trip.started_at, later.ts);
        assert_eq!(trip.start_odometer_mi, Some(50.0));
        restarted.save(&mut conn).await.unwrap();
        assert!(drain_one(&db.pool, db.vehicle_id).await.unwrap());
        assert_eq!(db.counts().await, (2, 0, 2));
        drop(conn);
        db.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn readiness_requires_heartbeat_checkpoint_and_clear_retry_queue() {
        use crate::routes::health::fetch_trip_capture_health;
        let db = TestDb::new().await;
        async fn ready(db: &TestDb) -> bool {
            serde_json::to_value(
                fetch_trip_capture_health(&db.pool, db.vehicle_id)
                    .await
                    .unwrap(),
            )
            .unwrap()["ready"]
                .as_bool()
                .unwrap()
        }
        assert!(!ready(&db).await);
        let mut conn = db.pool.acquire().await.unwrap();
        let mut capture = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        capture.save(&mut conn).await.unwrap();
        record_health(&db.pool, db.vehicle_id, None).await;
        sqlx::query("UPDATE panda.vehicle_runtime_state SET auth_state='authorized',worker_health='connected' WHERE vehicle_id=$1")
            .bind(db.vehicle_id).execute(&db.pool).await.unwrap();
        assert!(
            ready(&db).await,
            "parked vehicles need not emit fresh trip samples"
        );
        capture.process(&event(db.vehicle_id, 0, 10.0, PowerState::Drive));
        capture.process(&event(db.vehicle_id, 60, 11.0, PowerState::Sleep));
        capture.save(&mut conn).await.unwrap();
        assert!(!ready(&db).await);
        drain_one(&db.pool, db.vehicle_id).await.unwrap();
        record_health(&db.pool, db.vehicle_id, Some("checkpoint_retry_pending")).await;
        assert!(!ready(&db).await);
        record_health(&db.pool, db.vehicle_id, None).await;
        db.pool.execute("UPDATE panda.vehicle_runtime_state SET collector_heartbeat_at=now()-interval '61 seconds'").await.unwrap();
        assert!(!ready(&db).await);
        record_health(&db.pool, db.vehicle_id, None).await;
        db.pool
            .execute("UPDATE panda.vehicle_runtime_state SET worker_health='degraded'")
            .await
            .unwrap();
        assert!(!ready(&db).await);
        drop(conn);
        db.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn aborted_worker_releases_lock_and_recovery_rejects_changed_checkpoint() {
        let db = TestDb::new().await;
        let conn = super::super::acquire_collector_lock(&db.pool, db.vehicle_id)
            .await
            .unwrap()
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (child_tx, child_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _conn = conn;
            let _child = super::super::WorkerTask(tokio::spawn(async move {
                let _sender = child_tx;
                std::future::pending::<()>().await;
            }));
            started_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started_rx.await.unwrap();
        assert!(
            super::super::acquire_collector_lock(&db.pool, db.vehicle_id)
                .await
                .unwrap()
                .is_none()
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), child_rx)
                .await
                .unwrap()
                .is_err()
        );
        let mut conn = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(conn) = super::super::acquire_collector_lock(&db.pool, db.vehicle_id)
                    .await
                    .unwrap()
                {
                    break conn;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("aborted collector must release its session lock");

        let stale = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        let mut current = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        current.process(&event(db.vehicle_id, 0, 10.0, PowerState::Drive));
        current.save(&mut conn).await.unwrap();
        assert!(stale.verify_reacquired_lock(&mut conn).await.is_err());
        // A COMMIT may succeed before the connection loses its acknowledgement.
        current.persisted = None;
        current.verify_reacquired_lock(&mut conn).await.unwrap();
        db.pool.execute("UPDATE panda.active_trip_checkpoints SET snapshot=jsonb_set(snapshot,'{version}','999')").await.unwrap();
        assert!(TripCapture::load(&db.pool, db.vehicle_id).await.is_err());
        drop(conn);
        db.finish().await;
    }
    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn conflicting_outbox_payload_does_not_advance_checkpoint_or_drop_pending_trip() {
        let db = TestDb::new().await;
        let mut conn = db.pool.acquire().await.unwrap();
        let mut capture = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        capture.process(&event(db.vehicle_id, 0, 10.0, PowerState::Drive));
        capture.save(&mut conn).await.unwrap();
        let TripEvent::TripEnded { trip } =
            capture.process(&event(db.vehicle_id, 60, 11.0, PowerState::Sleep))
        else {
            panic!("expected completion")
        };
        let mut conflicting = serde_json::to_value(&trip).unwrap();
        conflicting["soc_end"] = serde_json::json!(1.0);
        sqlx::query(
            "INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip) VALUES($1,$2,$3)",
        )
        .bind(trip.trip_id)
        .bind(db.vehicle_id)
        .bind(conflicting)
        .execute(&db.pool)
        .await
        .unwrap();
        let error = capture.save(&mut conn).await.unwrap_err();
        assert!(error.to_string().contains("conflicting pending trip"));
        assert_eq!(capture.pending.len(), 1);
        conn.ping().await.unwrap();
        let recovered = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        assert_eq!(recovered.detector().active_trip_id(), Some(trip.trip_id));
        sqlx::query("DELETE FROM panda.pending_trip_completions WHERE trip_id=$1")
            .bind(trip.trip_id)
            .execute(&db.pool)
            .await
            .unwrap();
        capture.save(&mut conn).await.unwrap();
        assert!(capture.pending.is_empty());
        assert_eq!(db.counts().await, (0, 1, 0));
        drop(conn);
        db.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn recovery_rejects_other_vehicle_checkpoint_without_overwriting_it() {
        let db = TestDb::new().await;
        let mut conn = db.pool.acquire().await.unwrap();
        let mut capture = TripCapture::load(&db.pool, db.vehicle_id).await.unwrap();
        capture.save(&mut conn).await.unwrap();
        let mut wrong = serde_json::to_value(&capture.checkpoint).unwrap();
        wrong["detector"]["vehicle_id"] = serde_json::json!(Uuid::new_v4());
        sqlx::query("UPDATE panda.active_trip_checkpoints SET snapshot=$2 WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .bind(&wrong)
            .execute(&db.pool)
            .await
            .unwrap();
        let result = TripCapture::load(&db.pool, db.vehicle_id).await;
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("trip checkpoint vehicle mismatch"));
        let stored: serde_json::Value = sqlx::query_scalar(
            "SELECT snapshot FROM panda.active_trip_checkpoints WHERE vehicle_id=$1",
        )
        .bind(db.vehicle_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(stored, wrong);
        drop(conn);
        db.finish().await;
    }

    /// Legacy-only readings of a drive while Parallax is quiet: odometer, GPS
    /// and battery every minute, no power state and no speed.
    async fn insert_quiet_drive(db: &TestDb, start: DateTime<Utc>, steps: i64) {
        sqlx::query(
            "INSERT INTO timeseries.telemetry(ts,vehicle_id,odometer_miles,latitude,longitude,battery_level) \
             VALUES($1,$2,100.0,30.0,-97.0,70.0)",
        )
        .bind(start - Duration::hours(1))
        .bind(db.vehicle_id)
        .execute(&db.pool)
        .await
        .unwrap();
        for step in 0..steps {
            sqlx::query(
                "INSERT INTO timeseries.telemetry(ts,vehicle_id,odometer_miles,latitude,longitude,battery_level) \
                 VALUES($1,$2,$3,30.0,$4,$5)",
            )
            .bind(start + Duration::minutes(step))
            .bind(db.vehicle_id)
            .bind(100.62 + 0.62 * step as f64)
            .bind(-97.0 + 0.01 * step as f64)
            .bind(70.0 - 0.3 * step as f64)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        let parked = 100.62 + 0.62 * (steps - 1) as f64;
        for minute in [steps + 3, steps + 8] {
            sqlx::query(
                "INSERT INTO timeseries.telemetry(ts,vehicle_id,odometer_miles,latitude,longitude,battery_level) \
                 VALUES($1,$2,$3,30.0,$4,64.0)",
            )
            .bind(start + Duration::minutes(minute))
            .bind(db.vehicle_id)
            .bind(parked)
            .bind(-97.0 + 0.01 * (steps - 1) as f64)
            .execute(&db.pool)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn missed_drive_is_listed_recovered_once_and_linked() {
        use crate::services::trip_gaps::{find_trip_gaps, recover_trip_gap, RecoverError};
        use chrono::DurationRound;
        let db = TestDb::new().await;
        let start = (Utc::now() - Duration::days(2))
            .duration_trunc(Duration::minutes(1))
            .unwrap();
        insert_quiet_drive(&db, start, 20).await;

        let gaps = find_trip_gaps(&db.pool, db.vehicle_id).await.unwrap();
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        let gap = &gaps[0];
        assert_eq!(gap.started_at, start);
        assert!(
            (gap.distance_miles - 12.4).abs() < 1e-6,
            "{}",
            gap.distance_miles
        );

        let trip_id = recover_trip_gap(&db.pool, db.vehicle_id, &gap.id)
            .await
            .unwrap();
        let (linked, unlinked): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE trip_id = $1), count(*) FILTER (WHERE trip_id IS NULL) \
             FROM timeseries.telemetry WHERE vehicle_id = $2",
        )
        .bind(trip_id)
        .bind(db.vehicle_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(linked >= 20, "{linked}");
        assert!(
            unlinked >= 1,
            "parked readings outside the trip stay unlinked"
        );
        assert_eq!(db.counts().await, (1, 0, 1));
        assert!(matches!(
            recover_trip_gap(&db.pool, db.vehicle_id, &gap.id).await,
            Err(RecoverError::NotFound)
        ));
        assert!(find_trip_gaps(&db.pool, db.vehicle_id)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(db.counts().await, (1, 0, 1));
        db.finish().await;
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn recovered_trip_is_refused_while_a_drive_is_captured() {
        let db = TestDb::new().await;
        let mut trip = drain_fixture_trip(db.vehicle_id);
        let margin = Duration::minutes(2);
        // A live completion is pending: recovery waits for it.
        sqlx::query("INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip) VALUES($1,$2,'{}')")
            .bind(Uuid::new_v4())
            .bind(db.vehicle_id)
            .execute(&db.pool)
            .await
            .unwrap();
        assert_eq!(
            super::super::persist_recovered_trip(&db.pool, &trip, 5.0, margin)
                .await
                .unwrap(),
            super::super::RecoveredSave::Pending
        );
        assert_eq!(db.counts().await, (0, 1, 0));
        db.pool
            .execute("DELETE FROM panda.pending_trip_completions")
            .await
            .unwrap();

        assert_eq!(
            super::super::persist_recovered_trip(&db.pool, &trip, 5.0, margin)
                .await
                .unwrap(),
            super::super::RecoveredSave::Saved
        );
        // The same drive again, with new boundaries inside the margin.
        trip.trip_id = Uuid::new_v4();
        trip.started_at += Duration::seconds(90);
        assert_eq!(
            super::super::persist_recovered_trip(&db.pool, &trip, 5.0, margin)
                .await
                .unwrap(),
            super::super::RecoveredSave::Overlap
        );
        assert_eq!(db.counts().await, (1, 0, 1));
        db.finish().await;
    }

    fn drain_fixture_trip(vehicle_id: Uuid) -> CompletedTripData {
        let at = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut detector = TripDetectorState::new(vehicle_id);
        detector.process(&event(vehicle_id, 0, 10.0, PowerState::Drive));
        detector.process(&event(vehicle_id, 60, 12.0, PowerState::Drive));
        match detector.process(&event(vehicle_id, 120, 15.0, PowerState::Sleep)) {
            TripEvent::TripEnded { trip } => {
                assert_eq!(trip.started_at, at);
                trip
            }
            other => panic!("expected a completed trip, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore = "requires an explicit isolated synthetic TimescaleDB"]
    async fn saved_session_takes_its_peak_and_type_from_the_recorded_charging_graph() {
        let db = TestDb::new().await;
        let session_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(3);
        // R2 shape: the detector saw no power and classified from its average.
        let session = crate::ingestion::charge_detector::CompletedChargeSession {
            session_id,
            vehicle_id: db.vehicle_id,
            started_at,
            ended_at: started_at + Duration::minutes(16),
            location_lat: None,
            location_lng: None,
            soc_start: Some(28.0),
            soc_end: Some(72.0),
            charge_limit: Some(80.0),
            battery_capacity_wh: Some(91_500.0),
            energy_used_wh: None,
            energy_added_wh: Some(40_260.0),
            peak_charge_kw: None,
            avg_charge_rate_kw: Some(15.0),
            charger_type: Some("ac".to_string()),
        };
        sqlx::query(
            "INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,source) VALUES($1,$2,$3,'telemetry')",
        )
        .bind(session_id)
        .bind(db.vehicle_id)
        .bind(started_at)
        .execute(&db.pool)
        .await
        .unwrap();
        for (index, (seconds, kw)) in [(2_i64, 26.2_f64), (245, 189.5), (910, 116.5), (930, 950.0)]
            .into_iter()
            .enumerate()
        {
            sqlx::query("INSERT INTO timeseries.parallax_charge_curve_points(vehicle_id,source_at,segment_index,charge_session_id,power_kw) VALUES($1,$2,$3,$4,$5)")
                .bind(db.vehicle_id)
                .bind(started_at + Duration::seconds(seconds))
                .bind(index as i32)
                .bind(session_id)
                .bind(kw)
                .execute(&db.pool)
                .await
                .unwrap();
        }

        super::super::persist_charge_session(&db.pool, &session)
            .await
            .unwrap();

        let (charger_type, peak, voltage): (Option<String>, Option<f64>, Option<f64>) =
            sqlx::query_as("SELECT charger_type, max_charge_rate_kw, peak_voltage FROM panda.charge_sessions WHERE id=$1")
                .bind(session_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(charger_type.as_deref(), Some("dc"));
        // The implausible 950 kW reading is ignored.
        assert_eq!(peak, Some(189.5));
        assert_eq!(voltage, None, "no kW in the volts column");
        db.finish().await;
    }
}

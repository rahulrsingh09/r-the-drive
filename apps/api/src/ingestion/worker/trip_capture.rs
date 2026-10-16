//! Durable detector snapshots and a transactional completion outbox.

use anyhow::{ensure, Context};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, PgConnection, PgPool};
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
        let snapshot: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT snapshot FROM panda.active_trip_checkpoints WHERE vehicle_id=$1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?;
        let mut checkpoint = match snapshot.clone() {
            Some(value) => serde_json::from_value::<Checkpoint>(value)
                .context("invalid trip checkpoint; refusing to overwrite recovery state")?,
            None => Checkpoint {
                version: 1,
                detector: TripDetectorState::new(vehicle_id),
                last_event_at: None,
            },
        };
        ensure!(
            checkpoint.version == 1,
            "unsupported trip checkpoint version"
        );
        ensure!(
            checkpoint.detector.vehicle_id() == vehicle_id,
            "trip checkpoint vehicle mismatch"
        );
        checkpoint.detector.resume_after_restart();
        Ok(Self {
            replay_cutoff: checkpoint.last_event_at,
            checkpoint,
            recovering: true,
            pending: Vec::new(),
            persisted: snapshot,
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
        if self.is_replay(event.ts) {
            return TripEvent::NoChange;
        }
        self.recover_stale(event.ts);
        self.recovering = false;
        let transition = self.checkpoint.detector.process(event);
        if let TripEvent::TripEnded { trip } = &transition {
            self.pending.push(trip.clone());
        }
        self.checkpoint.last_event_at = Some(
            self.checkpoint
                .last_event_at
                .map_or(event.ts, |at| at.max(event.ts)),
        );
        transition
    }

    /// A stale fragment ends at its last observation, never at restart time.
    pub(super) fn recover_stale(&mut self, now: DateTime<Utc>) {
        if self.recovering {
            if let Some(last) = self.checkpoint.last_event_at {
                if now - last > Duration::seconds(RECOVERY_GAP_SECONDS) {
                    if let TripEvent::TripEnded { trip } =
                        self.checkpoint.detector.close_interrupted_trip(last)
                    {
                        self.pending.push(trip);
                    }
                    self.recovering = false;
                }
            }
        }
    }

    pub(super) async fn save(&mut self, conn: &mut PgConnection) -> anyhow::Result<()> {
        let snapshot = serde_json::to_value(&self.checkpoint)?;
        let mut tx = conn.begin().await?;
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
            .fetch_optional(&mut *tx)
            .await?;
            ensure!(
                saved == Some(trip.trip_id),
                "conflicting pending trip; retaining checkpoint"
            );
        }
        sqlx::query(
            "INSERT INTO panda.active_trip_checkpoints(vehicle_id,snapshot) VALUES($1,$2) \
             ON CONFLICT(vehicle_id) DO UPDATE SET snapshot=$2,updated_at=now()",
        )
        .bind(self.checkpoint.detector.vehicle_id())
        .bind(&snapshot)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.persisted = Some(snapshot);
        self.pending.clear();
        Ok(())
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
        // The candidate also covers a COMMIT whose acknowledgement was lost.
        ensure!(
            stored == self.persisted || stored == Some(serde_json::to_value(&self.checkpoint)?),
            "checkpoint advanced under another collector; refusing stale write"
        );
        Ok(())
    }
}

pub(super) async fn drain_one(pool: &PgPool, vehicle_id: Uuid) -> anyhow::Result<bool> {
    let pending: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT trip_id,trip FROM panda.pending_trip_completions \
         WHERE vehicle_id=$1 ORDER BY created_at,trip_id LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;
    let Some((trip_id, value)) = pending else {
        return Ok(false);
    };
    let trip: CompletedTripData = serde_json::from_value(value)?;
    ensure!(
        trip.vehicle_id == vehicle_id && trip.trip_id == trip_id,
        "completion identity mismatch"
    );
    let distance = compute_distance_odometer_or_gps(
        trip.start_odometer_mi,
        trip.end_odometer_mi,
        &trip.points,
    );
    super::persist_trip(pool, &trip, distance).await?;
    Ok(true)
}

pub(super) async fn record_health(pool: &PgPool, vehicle_id: Uuid, error: Option<&str>) {
    if let Err(error) = sqlx::query(
        "INSERT INTO panda.vehicle_runtime_state \
         (vehicle_id,collector_heartbeat_at,trip_persistence_error) VALUES($1,now(),$2) \
         ON CONFLICT(vehicle_id) DO UPDATE \
         SET collector_heartbeat_at=now(),trip_persistence_error=$2",
    )
    .bind(vehicle_id)
    .bind(error)
    .execute(pool)
    .await
    {
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
}

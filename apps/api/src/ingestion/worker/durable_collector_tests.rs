//! Synthetic contracts for the worker's durable boundary; never uses a configured app database.
use super::*;
use sqlx::{postgres::PgConnectOptions, Executor};

struct CollectorFixture {
    pool: PgPool,
    admin: PgPool,
    name: String,
    vehicle_id: Uuid,
    at: DateTime<Utc>,
}

impl CollectorFixture {
    async fn new() -> Self {
        Self::with_migrator(&crate::db::migrations::MIGRATOR).await
    }

    async fn with_migrator(migrator: &sqlx::migrate::Migrator) -> Self {
        let url = std::env::var("TRIP_DURABILITY_TEST_DATABASE_URL")
            .expect("explicit isolated synthetic TimescaleDB required");
        let options: PgConnectOptions = url.parse().unwrap();
        let admin = PgPool::connect_with(options.clone().database("postgres"))
            .await
            .unwrap();
        let name = format!("panda_durable_collector_{}", Uuid::new_v4().simple());
        admin
            .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .await
            .unwrap();
        let pool = PgPool::connect_with(options.database(&name)).await.unwrap();
        migrator.run(&pool).await.unwrap();
        let owner = Uuid::new_v4();
        let vehicle_id = Uuid::new_v4();
        sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,'collector@example.invalid','unused')")
            .bind(owner).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,'demo-durable-collector','R1T')")
            .bind(vehicle_id).bind(owner).execute(&pool).await.unwrap();
        let at =
            sqlx::query_scalar("SELECT date_trunc('milliseconds',now()) - interval '10 minutes'")
                .fetch_one(&pool)
                .await
                .unwrap();
        Self {
            pool,
            admin,
            name,
            vehicle_id,
            at,
        }
    }

    fn event(&self, seconds: i64) -> TelemetryEvent {
        TelemetryEvent::empty(self.vehicle_id, self.at + Duration::seconds(seconds))
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
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn raw_partial_storage_preserves_values_and_association_precedence() {
    let db = CollectorFixture::new().await;
    let trip_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let mut original = db.event(0);
    original.battery_level = Some(72.0);
    original.door_front_left_closed = Some(false);
    original.power_state = Some(PowerState::Go);
    original.power_state_ts = Some(original.ts);
    original.odometer_miles = Some(100.0);
    upsert_latest_status(&db.pool, &original).await.unwrap();
    write_telemetry(&db.pool, &original, Some(trip_id), Some(session_id))
        .await
        .unwrap();
    let mut fusion = TripSignalFusion::default();
    fusion.fuse_from(&original, true);
    let mut patch = db.event(60);
    patch.odometer_miles = Some(101.0);
    patch.door_front_left_closed = Some(true);
    let (joined, _) = fusion.fuse_from(&patch, true);
    assert!(joined.power_state.is_some());
    assert!(patch.power_state.is_none());
    write_telemetry(&db.pool, &patch, Some(trip_id), Some(session_id))
        .await
        .unwrap();
    upsert_latest_status(&db.pool, &patch).await.unwrap();
    let stored: (Option<String>, Option<f64>, bool) = sqlx::query_as(
        "SELECT power_state,battery_level,door_front_left_closed FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts=$2")
        .bind(db.vehicle_id).bind(patch.ts).fetch_one(&db.pool).await.unwrap();
    assert_eq!(stored, (None, None, true));
    let mut same_time = db.event(0);
    same_time.tire_fl_psi = Some(40.0);
    let replacement_session = Uuid::new_v4();
    write_telemetry(
        &db.pool,
        &same_time,
        Some(Uuid::new_v4()),
        Some(replacement_session),
    )
    .await
    .unwrap();
    let stored: (f64, bool, f64, Uuid, Uuid) = sqlx::query_as(
        "SELECT battery_level,door_front_left_closed,tire_fl_psi,trip_id,charge_session_id FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts=$2")
        .bind(db.vehicle_id).bind(original.ts).fetch_one(&db.pool).await.unwrap();
    assert_eq!(stored, (72.0, false, 40.0, trip_id, replacement_session));
    let latest: (f64, bool) = sqlx::query_as("SELECT battery_level,door_front_left_closed FROM panda.vehicle_latest_status WHERE vehicle_id=$1")
        .bind(db.vehicle_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(latest, (72.0, true));
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn durable_state_period_restart_and_late_frames_keep_one_open_period() {
    let db = CollectorFixture::new().await;
    let first = transition_state_period(&db.pool, db.vehicle_id, None, VehicleState::Drive, db.at)
        .await
        .unwrap()
        .unwrap();
    let recovered = load_open_state_period(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.id, recovered.id);
    let late = transition_state_period(
        &db.pool,
        db.vehicle_id,
        None,
        VehicleState::Sleep,
        db.at - Duration::seconds(1),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.id, late.id);
    let next_at = db.at + Duration::seconds(60);
    let next = transition_state_period(
        &db.pool,
        db.vehicle_id,
        Some(recovered),
        VehicleState::Sleep,
        next_at,
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(next.id, first.id);
    let repeated =
        transition_state_period(&db.pool, db.vehicle_id, None, VehicleState::Sleep, next_at)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(repeated.id, next.id);
    let periods: Vec<(String, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as("SELECT state,started_at,ended_at FROM panda.vehicle_state_periods WHERE vehicle_id=$1 ORDER BY started_at")
        .bind(db.vehicle_id).fetch_all(&db.pool).await.unwrap();
    assert_eq!(
        periods,
        vec![
            ("drive".into(), db.at, Some(next_at)),
            ("sleep".into(), next_at, None)
        ]
    );
    let mut partial = db.event(120);
    partial.door_front_left_closed = Some(false);
    write_telemetry(&db.pool, &partial, None, None)
        .await
        .unwrap();
    assert_eq!(
        reconcile_state_period_tail(&db.pool, db.vehicle_id, None)
            .await
            .unwrap()
            .unwrap()
            .id,
        next.id
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn charge_restart_sql_characterization() {
    let db = CollectorFixture::new().await;
    for (name, powers) in [
        ("changing", vec![Some(12.0), Some(24.0), Some(6.0)]),
        ("opening_only", vec![Some(12.0)]),
        ("no_power", vec![None, None]),
        ("negative", vec![Some(-12.0), Some(-24.0), Some(-6.0)]),
        (
            "exact_spike_and_ceiling",
            vec![Some(300.0), Some(10.0), Some(50.0), Some(300.0)],
        ),
        (
            "nonfinite",
            vec![None, Some(f64::NAN), Some(f64::INFINITY), Some(12.0)],
        ),
        ("missing", vec![Some(12.0), None, Some(24.0)]),
        ("zero", vec![Some(12.0), Some(0.0), Some(24.0)]),
        ("spike", vec![Some(0.0), Some(10.0), Some(60.0), Some(10.0)]),
        ("ceiling", vec![Some(12.0), Some(400.0), Some(12.0)]),
    ] {
        let mut detector = ChargeDetectorState::new(db.vehicle_id);
        for (i, power) in powers.iter().enumerate() {
            let mut event = db.event(i as i64 * 60);
            event.charger_state = Some(ChargerState::Charging);
            event.power_kw = *power;
            event.battery_level = Some(20.0 + i as f64);
            event.battery_capacity_wh = Some(135_000.0);
            event.battery_limit = Some(80.0);
            if i > 0 {
                event.latitude = Some(30.0);
                event.longitude = Some(-97.0);
            }
            detector.process(&event);
            let id = detector.active_session_id().unwrap();
            ensure_active_charge_session(&db.pool, db.vehicle_id, id, &event)
                .await
                .unwrap();
            write_telemetry(&db.pool, &event, None, Some(id))
                .await
                .unwrap();
        }
        let snapshot = load_active_charge_snapshot(&db.pool, db.vehicle_id)
            .await
            .unwrap();
        let mut restarted = ChargeDetectorState::from_snapshot(db.vehicle_id, snapshot.clone());
        let mut end = db.event(powers.len() as i64 * 60);
        end.charger_state = Some(ChargerState::Done);
        let ChargeEvent::SessionEnded(live) = detector.process(&end) else {
            panic!("live end")
        };
        let ChargeEvent::SessionEnded(restart) = restarted.process(&end) else {
            panic!("restart end")
        };
        assert_eq!(live.session_id, restart.session_id);
        println!("CHARGE_SQL {name}: live_wh={:?} restart_wh={:?} live_peak={:?} restart_peak={:?} last_power={:?}",live.energy_used_wh,restart.energy_used_wh,live.peak_charge_kw,restart.peak_charge_kw,snapshot.last_power_ts);
        assert_eq!(
            restart.energy_used_wh, live.energy_used_wh,
            "{name}: SQL restart integral must match live detector"
        );
        assert_eq!(
            restart.peak_charge_kw, live.peak_charge_kw,
            "{name}: SQL restart peak must match accepted power"
        );
        assert_eq!(
            format!("{restart:?}"),
            format!("{live:?}"),
            "{name}: completed session parity"
        );
        sqlx::query("DELETE FROM timeseries.telemetry WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM panda.charge_sessions WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    db.finish().await;
}

struct PipelineHarness {
    pipeline: durable_processing::CollectorPipeline,
    lock: PgConnection,
    shutdown: broadcast::Sender<()>,
    stop: broadcast::Receiver<()>,
    power: watch::Receiver<Option<PowerState>>,
    charging: watch::Receiver<bool>,
    context: watch::Receiver<ActiveSessionContext>,
    watchdog: tokio::time::Instant,
}

impl PipelineHarness {
    async fn new(db: &CollectorFixture, suppress: bool) -> Self {
        let config: Config = serde_json::from_value(serde_json::json!({
            "database_url": "synthetic-unused", "redis_url": "synthetic-unused",
            "rivian_suppress_duplicate_telemetry": suppress,
        }))
        .unwrap();
        let redis = redis::Client::open(
            std::env::var("DURABLE_COLLECTOR_TEST_REDIS_URL")
                .expect("explicit isolated Redis required"),
        )
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
        let lock = acquire_collector_lock(&db.pool, db.vehicle_id)
            .await
            .unwrap()
            .unwrap();
        let capture = trip_capture::TripCapture::load(&db.pool, db.vehicle_id)
            .await
            .unwrap();
        let (power_tx, power) = watch::channel(None);
        let (charge_tx, charging) = watch::channel(false);
        let (session_tx, context) = watch::channel(ActiveSessionContext::default());
        let (shutdown, stop) = broadcast::channel(1);
        let charge_capture = match charge_capture::ChargeCapture::load(&db.pool, db.vehicle_id)
            .await
            .unwrap()
        {
            Some(capture) => capture,
            None => {
                let snapshot = load_active_charge_snapshot(&db.pool, db.vehicle_id).await;
                let detector = snapshot.map_or_else(
                    || ChargeDetectorState::new(db.vehicle_id),
                    |s| ChargeDetectorState::from_snapshot(db.vehicle_id, s),
                );
                charge_capture::ChargeCapture::from_detector(detector)
            }
        };
        let period = load_open_state_period(&db.pool, db.vehicle_id)
            .await
            .unwrap();
        let pipeline = durable_processing::CollectorPipeline::new(
            durable_processing::CollectorIo {
                vehicle_id: db.vehicle_id,
                pool: db.pool.clone(),
                config,
                redis,
                power_state: power_tx,
                charging: charge_tx,
                active_session: session_tx,
            },
            capture,
            charge_capture,
            period,
        );
        Self {
            pipeline,
            lock,
            shutdown,
            stop,
            power,
            charging,
            context,
            watchdog: tokio::time::Instant::now() - std::time::Duration::from_secs(300),
        }
    }

    async fn send(&mut self, inbound: WsInboundEvent) -> durable_processing::ProcessOutcome {
        self.pipeline
            .process(inbound, &mut self.lock, &mut self.stop, &mut self.watchdog)
            .await
    }

    async fn telemetry(
        &mut self,
        event: TelemetryEvent,
        source: &str,
    ) -> durable_processing::ProcessOutcome {
        let inbound = WsInboundEvent {
            kind: WsInboundKind::Telemetry,
            received_at: event.ts,
            raw: "{\"synthetic\":true}".into(),
            message_type: Some(source.into()),
            telemetry: Some(event),
            charging_session: None,
            battery_cell_type: None,
        };
        self.send(inbound).await
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn stages_route_control_heartbeat_baseline_and_partial_payloads() {
    use durable_processing::ProcessOutcome;
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, false).await;
    let mut heartbeat = db.event(0);
    heartbeat.is_online = Some(false);
    heartbeat.battery_level = Some(99.0);
    let inbound = WsInboundEvent {
        kind: WsInboundKind::Heartbeat,
        received_at: heartbeat.ts,
        raw: "{}".into(),
        message_type: Some("ping".into()),
        telemetry: Some(heartbeat),
        charging_session: None,
        battery_cell_type: Some("synthetic-cell".into()),
    };
    assert!(matches!(
        harness.send(inbound).await,
        ProcessOutcome::Continue
    ));
    let counts:(i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM timeseries.telemetry),(SELECT count(*) FROM panda.active_trip_checkpoints)")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (0, 0));
    let cell: String =
        sqlx::query_scalar("SELECT battery_cell_type FROM panda.vehicles WHERE id=$1")
            .bind(db.vehicle_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(cell, "synthetic-cell");
    let before = harness.watchdog;
    let mut baseline = db.event(60);
    baseline.battery_level = Some(70.0);
    assert!(matches!(
        harness.telemetry(baseline, "baseline").await,
        ProcessOutcome::Continue
    ));
    assert_eq!(
        harness.watchdog, before,
        "baseline must not reset socket watchdog"
    );
    let mut partial = db.event(120);
    partial.door_front_left_closed = Some(false);
    harness.telemetry(partial, "parallax:doors").await;
    assert_eq!(
        harness.watchdog, before,
        "Parallax must not reset socket watchdog"
    );
    assert!(!harness.power.has_changed().unwrap());
    assert!(!harness.charging.has_changed().unwrap());
    let latest:(f64,bool)=sqlx::query_as("SELECT battery_level,door_front_left_closed FROM panda.vehicle_latest_status WHERE vehicle_id=$1")
        .bind(db.vehicle_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(latest, (70.0, false));
    harness
        .send(WsInboundEvent {
            kind: WsInboundKind::Control,
            received_at: db.at,
            raw: "{}".into(),
            message_type: Some("reconnect".into()),
            telemetry: None,
            charging_session: None,
            battery_cell_type: None,
        })
        .await;
    harness.pipeline.flush().await;
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM timeseries.telemetry),(SELECT count(*) FROM panda.active_trip_checkpoints),(SELECT count(*) FROM panda.rivian_ws_raw_events)")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (2, 1, 2));
    let health: String = sqlx::query_scalar(
        "SELECT worker_health FROM panda.vehicle_runtime_state WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(health, "degraded");
    drop(harness);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn checkpoint_failure_stops_all_downstream_stages_but_retains_raw_intake() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, false).await;
    db.pool.execute("ALTER TABLE panda.active_trip_checkpoints ADD CONSTRAINT synthetic_failure CHECK(false) NOT VALID").await.unwrap();
    let mut event = db.event(0);
    event.charger_state = Some(ChargerState::Charging);
    event.power_kw = Some(12.0);
    harness.shutdown.send(()).unwrap();
    assert!(matches!(
        harness.telemetry(event, "next").await,
        durable_processing::ProcessOutcome::CheckpointUnavailable
    ));
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM panda.rivian_ws_raw_events),(SELECT count(*) FROM panda.charge_sessions),(SELECT count(*) FROM timeseries.telemetry),(SELECT count(*) FROM panda.vehicle_latest_status)")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (1, 0, 0, 0));
    assert!(!harness.context.has_changed().unwrap());
    assert!(!harness.charging.has_changed().unwrap());
    drop(harness);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn checkpoint_retry_commits_once_before_telemetry_and_restart_keeps_trip_id() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, false).await;
    db.pool.execute("ALTER TABLE panda.active_trip_checkpoints ADD CONSTRAINT synthetic_failure CHECK(false) NOT VALID").await.unwrap();
    let mut first = db.event(0);
    first.power_state = Some(PowerState::Drive);
    first.speed_mph = Some(30.0);
    first.odometer_miles = Some(100.0);
    let pool = db.pool.clone();
    let delayed = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM timeseries.telemetry")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
        pool.execute("ALTER TABLE panda.active_trip_checkpoints DROP CONSTRAINT synthetic_failure")
            .await
            .unwrap();
    });
    assert!(matches!(
        harness.telemetry(first.clone(), "next").await,
        durable_processing::ProcessOutcome::Continue
    ));
    delayed.await.unwrap();
    let id: Uuid =
        sqlx::query_scalar("SELECT trip_id FROM timeseries.telemetry WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    release_collector_lock(&mut harness.lock, db.vehicle_id)
        .await
        .unwrap();
    drop(harness);
    let mut restarted = PipelineHarness::new(&db, false).await;
    first.power_state = Some(PowerState::Sleep);
    restarted.telemetry(first, "parallax:power").await;
    let mut later = db.event(60);
    later.odometer_miles = Some(101.0);
    restarted.telemetry(later, "parallax:odometer").await;
    let capture = trip_capture::TripCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    assert_eq!(
        capture.detector().active_trip_id(),
        Some(id),
        "replay must not prime fusion with stale sleep"
    );
    let mut end = db.event(90);
    end.power_state = Some(PowerState::Sleep);
    end.odometer_miles = Some(102.0);
    restarted.telemetry(end, "next").await;
    let completion: serde_json::Value =
        sqlx::query_scalar("SELECT trip FROM panda.pending_trip_completions WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(completion["trip_id"], id.to_string());
    restarted.pipeline.retry_completion().await;
    let trips: Vec<(Uuid, f64)> =
        sqlx::query_as("SELECT id,distance_miles FROM panda.trips WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(trips, vec![(id, 2.0)]);
    drop(restarted);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn canonical_charge_context_and_parallax_bypass_survive_partial_updates() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, false).await;
    let mut first = db.event(0);
    first.charger_state = Some(ChargerState::Charging);
    first.power_kw = Some(12.0);
    harness.telemetry(first, "next").await;
    let session_id = harness.context.borrow().session_id.unwrap();
    assert_eq!(harness.context.borrow().started_at, Some(db.at));
    assert!(*harness.charging.borrow_and_update());
    let mut partial = db.event(1900);
    partial.power_state = Some(PowerState::Sleep);
    partial.power_kw = Some(100.0);
    harness.telemetry(partial, "parallax:power").await;
    assert_eq!(harness.context.borrow().session_id, Some(session_id));
    assert!(!harness.charging.has_changed().unwrap());
    let state: String = sqlx::query_scalar(
        "SELECT state FROM panda.vehicle_state_periods WHERE vehicle_id=$1 AND ended_at IS NULL",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(state, "charging");
    let rebuilt = load_active_charge_snapshot(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    assert!((rebuilt.energy_used_wh - 100_000.0 * 1900.0 / 3600.0).abs() < 0.000001);
    println!("RESTART_LIMIT parallax_stamp: live_wh=0 restart_wh={}; stamped telemetry has no source discriminator", rebuilt.energy_used_wh);
    let mut end = db.event(1960);
    end.charger_state = Some(ChargerState::Done);
    let durable_processing::ProcessOutcome::ChargeEnded(session) =
        harness.telemetry(end, "next").await
    else {
        panic!("canonical charge must close")
    };
    assert_eq!(session.session_id, session_id);
    assert_eq!(
        session.energy_used_wh, None,
        "Parallax power cannot enter canonical integral"
    );
    assert_eq!(harness.context.borrow().session_id, None);
    assert_eq!(
        harness.context.borrow().ended_at,
        Some(db.at + Duration::seconds(1960))
    );
    let associations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM timeseries.telemetry WHERE vehicle_id=$1 AND charge_session_id=$2",
    )
    .bind(db.vehicle_id)
    .bind(session_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        associations, 3,
        "start, partial and end retain canonical association"
    );
    drop(harness);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn legacy_projection_materializes_with_start_preserves_partial_and_clears_on_end() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, false).await;
    let redis_client =
        redis::Client::open(std::env::var("DURABLE_COLLECTOR_TEST_REDIS_URL").unwrap()).unwrap();
    let mut redis = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let key = rivian_poll::live_session_redis_key(db.vehicle_id);
    let meaningful = r#"{"type":"next","payload":{"data":{"chargingSession":{"liveData":{"powerKW":9.5,"totalChargedEnergy":12.5},"chartData":[]}}}}"#;
    let partial = r#"{"type":"next","payload":{"data":{"chargingSession":{"liveData":null,"chartData":[]}}}}"#;
    let explicit_null = r#"{"type":"next","payload":{"data":{"chargingSession":null}}}"#;
    let envelope = |raw: &str, event: Option<TelemetryEvent>| WsInboundEvent {
        kind: WsInboundKind::ChargingSession,
        received_at: db.at,
        raw: raw.into(),
        message_type: Some("next".into()),
        telemetry: event,
        charging_session: parser::parse_charging_session_message(raw).unwrap(),
        battery_cell_type: None,
    };
    let mut start = db.event(0);
    start.charger_state = Some(ChargerState::Charging);
    harness.send(envelope(meaningful, Some(start))).await;
    let id = harness.context.borrow().session_id.unwrap();
    let stored: Option<String> = redis.get(&key).await.unwrap();
    assert!(stored.is_some());
    let energy: Option<f64> =
        sqlx::query_scalar("SELECT live_total_charged_kwh FROM panda.charge_sessions WHERE id=$1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        energy,
        Some(12.5),
        "canonical row must exist before projection enrichment"
    );
    harness.send(envelope(partial, None)).await;
    assert_eq!(redis.get::<_, Option<String>>(&key).await.unwrap(), stored);
    harness.send(envelope(explicit_null, None)).await;
    assert!(!redis.exists::<_, bool>(&key).await.unwrap());
    harness.send(envelope(meaningful, None)).await;
    assert!(redis.exists::<_, bool>(&key).await.unwrap());
    let mut end = db.event(60);
    end.charger_state = Some(ChargerState::Done);
    assert!(matches!(
        harness.send(envelope(meaningful, Some(end))).await,
        durable_processing::ProcessOutcome::ChargeEnded(_)
    ));
    assert!(!redis.exists::<_, bool>(&key).await.unwrap());
    harness.send(envelope(meaningful, None)).await;
    assert!(
        !redis.exists::<_, bool>(&key).await.unwrap(),
        "late projection cannot resurrect closed session"
    );
    harness.pipeline.flush().await;
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT legacy_null_count,legacy_all_null_count,legacy_meaningful_count FROM panda.parallax_collector_state WHERE vehicle_id=$1")
        .bind(db.vehicle_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        counts,
        (1, 0, 5),
        "chartData:[] is a non-null diagnostic field"
    );
    drop(harness);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn suppression_keeps_live_latest_checkpoint_and_software_stages_running() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, true).await;
    let mut first = db.event(0);
    first.battery_level = Some(72.0);
    first.ota_current_version = Some("synthetic.1".into());
    harness.telemetry(first.clone(), "next").await;
    first.ts += Duration::seconds(30);
    harness.telemetry(first, "next").await;
    let mut threshold = db.event(60);
    threshold.battery_level = Some(71.8);
    threshold.battery_level_ts = Some(threshold.ts);
    harness.telemetry(threshold, "next").await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM timeseries.telemetry")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let latest: f64 = sqlx::query_scalar(
        "SELECT battery_level FROM panda.vehicle_latest_status WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(latest, 71.8);
    let mut upgrade = db.event(90);
    upgrade.ota_current_version = Some("synthetic.2".into());
    harness.telemetry(upgrade, "next").await;
    let software:Vec<(String,Option<DateTime<Utc>>)>=sqlx::query_as("SELECT version,observed_until FROM panda.software_versions WHERE vehicle_id=$1 ORDER BY installed_at")
        .bind(db.vehicle_id).fetch_all(&db.pool).await.unwrap();
    assert_eq!(
        software,
        vec![
            ("synthetic.1".into(), Some(db.at + Duration::seconds(90))),
            ("synthetic.2".into(), None)
        ]
    );
    let snapshot: serde_json::Value = sqlx::query_scalar(
        "SELECT snapshot FROM panda.active_trip_checkpoints WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let last: DateTime<Utc> = serde_json::from_value(snapshot["last_event_at"].clone()).unwrap();
    assert_eq!(last, db.at + Duration::seconds(90));
    drop(harness);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn startup_drains_at_most_sixteen_completions_before_credentials_or_redis() {
    let db = CollectorFixture::new().await;
    let mut capture = trip_capture::TripCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    for i in 0..17 {
        let mut start = db.event(i * 20);
        start.power_state = Some(PowerState::Drive);
        start.speed_mph = Some(30.0);
        start.odometer_miles = Some(i as f64 * 2.0);
        capture.process(&start);
        let mut end = db.event(i * 20 + 10);
        end.power_state = Some(PowerState::Sleep);
        end.odometer_miles = Some(i as f64 * 2.0 + 1.0);
        assert!(matches!(capture.process(&end), TripEvent::TripEnded { .. }));
    }
    let mut conn = db.pool.acquire().await.unwrap();
    capture.save(&mut conn).await.unwrap();
    drop(conn);
    let config: Config =
        serde_json::from_value(serde_json::json!({"database_url":"unused","redis_url":"unused"}))
            .unwrap();
    let (_stop, shutdown) = broadcast::channel(1);
    run_vehicle_worker(
        db.vehicle_id,
        db.pool.clone(),
        redis::Client::open("redis://127.0.0.1:1").unwrap(),
        "invalid-synthetic-key".into(),
        config,
        shutdown,
    )
    .await;
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM panda.trips),(SELECT count(*) FROM panda.pending_trip_completions)").fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (16, 1));
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn database_trace_preserves_intake_checkpoint_charge_projection_and_storage_order() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, false).await;
    db.pool
        .execute("CREATE TABLE collector_stage_trace (sequence bigserial, target text)")
        .await
        .unwrap();
    db.pool.execute("CREATE FUNCTION collector_trace_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN INSERT INTO public.collector_stage_trace(target) VALUES(TG_ARGV[0]); RETURN NEW; END $$").await.unwrap();
    for table in [
        "panda.rivian_ws_raw_events",
        "panda.active_trip_checkpoints",
        "panda.charge_sessions",
        "panda.vehicle_runtime_state",
        "panda.vehicle_latest_status",
        "timeseries.telemetry",
        "panda.vehicle_state_periods",
    ] {
        db.pool.execute(sqlx::AssertSqlSafe(format!("CREATE TRIGGER collector_trace AFTER INSERT OR UPDATE ON {table} FOR EACH ROW EXECUTE FUNCTION collector_trace_write('{}')", table.split('.').nth(1).unwrap()))).await.unwrap();
    }
    let mut event = db.event(0);
    event.charger_state = Some(ChargerState::Charging);
    event.power_kw = Some(12.0);
    harness.telemetry(event, "next").await;
    let order: Vec<String> =
        sqlx::query_scalar("SELECT target FROM collector_stage_trace ORDER BY sequence")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        order,
        vec![
            "rivian_ws_raw_events",
            "active_trip_checkpoints",
            "charge_sessions",
            "vehicle_runtime_state",
            "vehicle_latest_status",
            "telemetry",
            "vehicle_runtime_state",
            "vehicle_state_periods"
        ]
    );
    drop(harness);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn charge_restart_continuation_uses_last_accepted_power_clock() {
    let db = CollectorFixture::new().await;
    let mut live = ChargeDetectorState::new(db.vehicle_id);
    for (seconds, kw) in [
        (0, Some(12.0)),
        (60, Some(10.0)),
        (120, Some(60.0)),
        (180, None),
        (240, Some(0.0)),
    ] {
        let mut event = db.event(seconds);
        event.charger_state = Some(ChargerState::Charging);
        event.power_kw = kw;
        live.process(&event);
        let id = live.active_session_id().unwrap();
        ensure_active_charge_session(&db.pool, db.vehicle_id, id, &event)
            .await
            .unwrap();
        write_telemetry(&db.pool, &event, None, Some(id))
            .await
            .unwrap();
    }
    let snapshot = load_active_charge_snapshot(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    assert_eq!(snapshot.last_power_ts, Some(db.at + Duration::seconds(60)));
    let mut resumed = ChargeDetectorState::from_snapshot(db.vehicle_id, snapshot);
    let mut end = db.event(300);
    end.charger_state = Some(ChargerState::Done);
    end.power_kw = Some(10.0);
    let ChargeEvent::SessionEnded(expected) = live.process(&end) else {
        panic!("live end")
    };
    let ChargeEvent::SessionEnded(actual) = resumed.process(&end) else {
        panic!("resumed end")
    };
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    println!(
        "CHARGE_SQL continuation: live_wh={:?} restart_wh={:?}",
        expected.energy_used_wh, actual.energy_used_wh
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn charge_restart_documents_unpersisted_sample_information_loss() {
    let db = CollectorFixture::new().await;
    let mut live = ChargeDetectorState::new(db.vehicle_id);
    for (seconds, kw) in [(0, 12.0), (60, 24.0), (120, 6.0)] {
        let mut event = db.event(seconds);
        event.charger_state = Some(ChargerState::Charging);
        event.power_kw = Some(kw);
        live.process(&event);
        let id = live.active_session_id().unwrap();
        ensure_active_charge_session(&db.pool, db.vehicle_id, id, &event)
            .await
            .unwrap();
        if seconds != 60 {
            write_telemetry(&db.pool, &event, None, Some(id))
                .await
                .unwrap();
        }
    }
    let snapshot = load_active_charge_snapshot(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    let mut resumed = ChargeDetectorState::from_snapshot(db.vehicle_id, snapshot);
    let mut end = db.event(180);
    end.charger_state = Some(ChargerState::Done);
    let ChargeEvent::SessionEnded(expected) = live.process(&end) else {
        panic!("live end")
    };
    let ChargeEvent::SessionEnded(actual) = resumed.process(&end) else {
        panic!("resumed end")
    };
    assert_eq!(expected.energy_used_wh, Some(500.0));
    assert_eq!(actual.energy_used_wh, Some(200.0));
    println!("RESTART_LIMIT unpersisted_sample: live_wh={:?} restart_wh={:?}; ordered storage cannot recover an omitted accepted reading",expected.energy_used_wh,actual.energy_used_wh);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires an explicit isolated synthetic TimescaleDB"]
async fn charge_restart_documents_lost_arrival_order() {
    let db = CollectorFixture::new().await;
    let mut live = ChargeDetectorState::new(db.vehicle_id);
    for (seconds, kw) in [(0, 12.0), (60, 12.0), (30, 24.0), (120, 12.0)] {
        let mut event = db.event(seconds);
        event.charger_state = Some(ChargerState::Charging);
        event.power_kw = Some(kw);
        live.process(&event);
        let id = live.active_session_id().unwrap();
        ensure_active_charge_session(&db.pool, db.vehicle_id, id, &event)
            .await
            .unwrap();
        write_telemetry(&db.pool, &event, None, Some(id))
            .await
            .unwrap();
    }
    let snapshot = load_active_charge_snapshot(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    let mut resumed = ChargeDetectorState::from_snapshot(db.vehicle_id, snapshot);
    let mut end = db.event(180);
    end.charger_state = Some(ChargerState::Done);
    let ChargeEvent::SessionEnded(expected) = live.process(&end) else {
        panic!("live end")
    };
    let ChargeEvent::SessionEnded(actual) = resumed.process(&end) else {
        panic!("resumed end")
    };
    assert_eq!(expected.energy_used_wh, Some(400.0));
    assert_eq!(actual.energy_used_wh, Some(500.0));
    assert_eq!(expected.peak_charge_kw, Some(12.0));
    assert_eq!(actual.peak_charge_kw, Some(24.0));
    println!("RESTART_LIMIT arrival_order: live_wh={:?} restart_wh={:?}, live_peak={:?} restart_peak={:?}; SQL timestamp order cannot recover original arrival order",expected.energy_used_wh,actual.energy_used_wh,expected.peak_charge_kw,actual.peak_charge_kw);
    db.finish().await;
}

mod charge_durability;

mod telemetry_workflows;

mod residual_workflows;

mod legacy_charge_reference;

mod reliability_regressions;

mod residual_boundaries;

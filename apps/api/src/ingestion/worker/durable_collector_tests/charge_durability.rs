use super::*;
use crate::ingestion::worker::{
    charge_capture::{self, ChargeCapture},
    state_decisions::SampleSource,
};

async fn captures(
    db: &CollectorFixture,
) -> (trip_capture::TripCapture, ChargeCapture, PgConnection) {
    let mut trips = trip_capture::TripCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    let mut conn = acquire_collector_lock(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    let charges = charge_capture::initialize(&mut conn, &mut trips, db.vehicle_id)
        .await
        .unwrap();
    (trips, charges, conn)
}

async fn commit_event(
    trips: &mut trip_capture::TripCapture,
    charges: &mut ChargeCapture,
    conn: &mut PgConnection,
    event: &TelemetryEvent,
    source: SampleSource,
) -> ChargeEvent {
    trips.process(event);
    let result = charges.process(event, source, event.ts).unwrap();
    charge_capture::save_joint(conn, trips, charges)
        .await
        .unwrap();
    result
}

fn charging(db: &CollectorFixture, seconds: i64, kw: f64) -> TelemetryEvent {
    let mut event = db.event(seconds);
    event.charger_state = Some(ChargerState::Charging);
    event.power_kw = Some(kw);
    event
}

fn ended(transition: ChargeEvent) -> crate::ingestion::charge_detector::CompletedChargeSession {
    let ChargeEvent::SessionEnded(session) = transition else {
        panic!("expected completed charge");
    };
    session
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn durable_charge_restart_preserves_omissions_arrival_order_and_parallax_bypass() {
    for (name, frames, expected_wh, expected_peak) in [
        (
            "failed_telemetry",
            vec![(60, 24.0, false), (120, 6.0, false)],
            Some(500.0),
            Some(24.0),
        ),
        (
            "arrival_order",
            vec![(60, 12.0, false), (30, 24.0, false), (120, 12.0, false)],
            Some(400.0),
            Some(12.0),
        ),
        ("parallax_bypass", vec![(1900, 100.0, true)], None, None),
    ] {
        let db = CollectorFixture::new().await;
        let (mut trips, mut charges, mut conn) = captures(&db).await;
        let first = charging(&db, 0, 12.0);
        commit_event(
            &mut trips,
            &mut charges,
            &mut conn,
            &first,
            SampleSource::Socket,
        )
        .await;
        let mut reference = charges.detector().clone();
        let id = reference.active_session_id();
        write_telemetry(&db.pool, &first, None, id).await.unwrap();
        if name == "failed_telemetry" {
            db.pool.execute("ALTER TABLE timeseries.telemetry ADD CONSTRAINT synthetic_charge_write CHECK(power_kw <> 24) NOT VALID").await.unwrap();
        }
        for (seconds, kw, parallax) in frames {
            let event = charging(&db, seconds, kw);
            let source = if parallax {
                SampleSource::Parallax
            } else {
                SampleSource::Socket
            };
            if !parallax {
                reference.process(&event);
            }
            commit_event(&mut trips, &mut charges, &mut conn, &event, source).await;
            let stored = write_telemetry(&db.pool, &event, None, id).await;
            assert_eq!(stored.is_err(), name == "failed_telemetry" && seconds == 60);
        }
        let mut resumed = ChargeCapture::load(&db.pool, db.vehicle_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(resumed.detector()).unwrap(),
            serde_json::to_value(&reference).unwrap(),
            "{name}"
        );
        let mut end = db.event(if name == "parallax_bypass" { 1960 } else { 180 });
        end.charger_state = Some(ChargerState::Done);
        let expected = ended(reference.process(&end));
        let actual = ended(
            commit_event(
                &mut trips,
                &mut resumed,
                &mut conn,
                &end,
                SampleSource::Socket,
            )
            .await,
        );
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap(),
            "{name}"
        );
        assert_eq!(actual.energy_used_wh, expected_wh, "{name}");
        assert_eq!(actual.peak_charge_kw, expected_peak, "{name}");
        assert!(charge_capture::drain_one(&db.pool, db.vehicle_id)
            .await
            .unwrap());
        let stored: (Option<f64>, Option<f64>) = sqlx::query_as(
            "SELECT energy_used_wh,max_charge_rate_kw FROM panda.charge_sessions WHERE id=$1",
        )
        .bind(actual.session_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(stored, (expected_wh, expected_peak));
        println!("DURABLE_CHARGE {name}: {stored:?}; all completed fields match");
        drop(conn);
        db.finish().await;
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn durable_charge_survives_actual_suppression_and_worker_restart() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, true).await;
    for (seconds, kw) in [(0, 12.0), (60, 12.0), (120, 6.0)] {
        harness.telemetry(charging(&db, seconds, kw), "next").await;
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM timeseries.telemetry WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        count, 2,
        "accepted duplicate power is deliberately suppressed"
    );
    drop(harness);
    let mut resumed = PipelineHarness::new(&db, true).await;
    let mut end = db.event(180);
    end.charger_state = Some(ChargerState::Done);
    let durable_processing::ProcessOutcome::ChargeEnded(actual) =
        resumed.telemetry(end, "next").await
    else {
        panic!("completion");
    };
    assert_eq!(actual.energy_used_wh, Some(300.0));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.pending_charge_completions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    drop(resumed);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn charge_checkpoint_and_trip_outboxes_rollback_and_retry_as_one_candidate() {
    let db = CollectorFixture::new().await;
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    let first = charging(&db, 0, 12.0);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &first,
        SampleSource::Socket,
    )
    .await;
    let before: (serde_json::Value,serde_json::Value) = sqlx::query_as("SELECT t.snapshot,c.snapshot FROM panda.active_trip_checkpoints t JOIN panda.active_charge_checkpoints c USING(vehicle_id)").fetch_one(&db.pool).await.unwrap();
    let mut end = db.event(60);
    end.charger_state = Some(ChargerState::Done);
    trips.process(&end);
    charges.process(&end, SampleSource::Socket, end.ts).unwrap();
    for target in [
        "panda.active_trip_checkpoints",
        "panda.active_charge_checkpoints",
        "panda.pending_charge_completions",
    ] {
        db.pool
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {target} ADD CONSTRAINT synthetic_charge_fault CHECK(false) NOT VALID"
            )))
            .await
            .unwrap();
        assert!(
            charge_capture::save_joint(&mut conn, &mut trips, &mut charges)
                .await
                .is_err(),
            "{target}"
        );
        conn.ping().await.unwrap();
        let after: (serde_json::Value,serde_json::Value) = sqlx::query_as("SELECT t.snapshot,c.snapshot FROM panda.active_trip_checkpoints t JOIN panda.active_charge_checkpoints c USING(vehicle_id)").fetch_one(&db.pool).await.unwrap();
        assert_eq!(before, after, "{target}");
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM panda.pending_charge_completions")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(pending, 0, "{target}");
        db.pool
            .execute(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {target} DROP CONSTRAINT synthetic_charge_fault"
            )))
            .await
            .unwrap();
    }
    charge_capture::save_joint(&mut conn, &mut trips, &mut charges)
        .await
        .unwrap();
    assert!(ChargeCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap()
        .detector()
        .active_session_id()
        .is_none());
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.pending_charge_completions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(pending, 1);
    drop(conn);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn completion_write_and_ack_failure_survive_restart_without_resurrecting_idle_state() {
    let db = CollectorFixture::new().await;
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    for (seconds, kw) in [(0, 12.0), (60, 24.0), (120, 6.0)] {
        let event = charging(&db, seconds, kw);
        commit_event(
            &mut trips,
            &mut charges,
            &mut conn,
            &event,
            SampleSource::Socket,
        )
        .await;
        write_telemetry(
            &db.pool,
            &event,
            None,
            charges.detector().active_session_id(),
        )
        .await
        .unwrap();
    }
    let id = charges.detector().active_session_id().unwrap();
    let mut end = db.event(180);
    end.charger_state = Some(ChargerState::Done);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &end,
        SampleSource::Socket,
    )
    .await;
    db.pool.execute("ALTER TABLE panda.charge_sessions ADD CONSTRAINT synthetic_completion_fault CHECK(ended_at IS NULL) NOT VALID").await.unwrap();
    assert!(charge_capture::drain_one(&db.pool, db.vehicle_id)
        .await
        .is_err());
    db.pool
        .execute("ALTER TABLE panda.charge_sessions DROP CONSTRAINT synthetic_completion_fault")
        .await
        .unwrap();
    db.pool.execute("CREATE FUNCTION synthetic_ack_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic ack failure'; END $$; CREATE TRIGGER synthetic_ack_failure BEFORE DELETE ON panda.pending_charge_completions FOR EACH ROW EXECUTE FUNCTION synthetic_ack_failure()").await.unwrap();
    assert!(charge_capture::drain_one(&db.pool, db.vehicle_id)
        .await
        .is_err());
    let row:(Option<DateTime<Utc>>,i64)=sqlx::query_as("SELECT ended_at,(SELECT count(*) FROM panda.pending_charge_completions) FROM panda.charge_sessions WHERE id=$1").bind(id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        row,
        (None, 1),
        "canonical row and outbox acknowledgment must roll back together"
    );
    db.pool
        .execute("DROP TRIGGER synthetic_ack_failure ON panda.pending_charge_completions")
        .await
        .unwrap();
    drop(conn);
    let (mut trips, mut recovered, mut conn) = captures(&db).await;
    assert!(
        recovered.detector().active_session_id().is_none(),
        "Idle checkpoint outranks stamped legacy telemetry"
    );
    assert!(charge_capture::drain_one(&db.pool, db.vehicle_id)
        .await
        .unwrap());
    assert!(!charge_capture::drain_one(&db.pool, db.vehicle_id)
        .await
        .unwrap());
    let row:(f64,i64)=sqlx::query_as("SELECT energy_used_wh,(SELECT count(*) FROM panda.pending_charge_completions) FROM panda.charge_sessions WHERE id=$1").bind(id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(row, (500.0, 0));
    commit_event(
        &mut trips,
        &mut recovered,
        &mut conn,
        &db.event(240),
        SampleSource::Socket,
    )
    .await;
    assert!(recovered.detector().active_session_id().is_none());
    drop(conn);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn lost_commit_ack_retries_idempotently_and_fences_a_stale_collector() {
    let db = CollectorFixture::new().await;
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    let mut stale_trips = trip_capture::TripCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap();
    let mut stale_charges = ChargeCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    let event = charging(&db, 0, 12.0);
    trips.process(&event);
    charges
        .process(&event, SampleSource::Socket, event.ts)
        .unwrap();
    let mut tx = conn.begin().await.unwrap();
    crate::services::charge_sessions::lock_charge_history(&mut tx, db.vehicle_id)
        .await
        .unwrap();
    trips.write(&mut tx).await.unwrap();
    charges.write(&mut tx).await.unwrap();
    tx.commit().await.unwrap();
    trips.verify_reacquired_lock(&mut conn).await.unwrap();
    charges.verify_reacquired_lock(&mut conn).await.unwrap();
    charge_capture::save_joint(&mut conn, &mut trips, &mut charges)
        .await
        .unwrap();
    let saved: serde_json::Value = sqlx::query_scalar(
        "SELECT snapshot FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    stale_trips.process(&db.event(60));
    stale_charges
        .process(&db.event(60), SampleSource::Socket, db.at)
        .unwrap();
    assert!(
        charge_capture::save_joint(&mut conn, &mut stale_trips, &mut stale_charges)
            .await
            .is_err()
    );
    conn.ping().await.unwrap();
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT snapshot FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(saved, after);
    drop(conn);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn snapshot_jsonb_round_trips_float_bits_and_rejects_corrupt_identity_or_version() {
    let db = CollectorFixture::new().await;
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    let mut metadata = db.event(0);
    metadata.battery_level = Some(f64::from_bits(0x7ff8_0000_0000_0042));
    metadata.battery_limit = Some(f64::INFINITY);
    metadata.battery_capacity_wh = Some(f64::NEG_INFINITY);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &metadata,
        SampleSource::Socket,
    )
    .await;
    let loaded = ChargeCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(loaded.detector()).unwrap(),
        serde_json::to_value(charges.detector()).unwrap()
    );
    let saved: serde_json::Value = sqlx::query_scalar(
        "SELECT snapshot FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for (key, value) in [
        ("{version}", serde_json::json!(999)),
        ("{detector,vehicle_id}", serde_json::json!(Uuid::new_v4())),
    ] {
        sqlx::query("UPDATE panda.active_charge_checkpoints SET snapshot=jsonb_set(snapshot,$2::text[],$3) WHERE vehicle_id=$1")
            .bind(db.vehicle_id).bind(key).bind(value).execute(&db.pool).await.unwrap();
        assert!(ChargeCapture::load(&db.pool, db.vehicle_id).await.is_err());
        sqlx::query("UPDATE panda.active_charge_checkpoints SET snapshot=$2 WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .bind(&saved)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    drop(conn);
    db.finish().await;
}

async fn duplicate(db: &CollectorFixture, active: Uuid) -> Uuid {
    let duplicate = Uuid::new_v4();
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,source,rivian_session_id) VALUES($1,$2,$3,$4,'telemetry+rivian_api','synthetic-durable-charge')")
        .bind(duplicate).bind(db.vehicle_id).bind(db.at + Duration::seconds(60)).bind(db.at+Duration::seconds(180)).execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,transaction_id_grouping_key) VALUES($1,'synthetic-charge-a','txn','synthetic-durable-group'),($2,'synthetic-charge-b','txn','synthetic-durable-group')")
        .bind(active).bind(duplicate).execute(&db.pool).await.unwrap();
    crate::services::charge_sessions::backfill_charge_session_aliases_from_rows(
        &db.pool,
        Some(db.vehicle_id),
    )
    .await
    .unwrap();
    duplicate
}

async fn references(db: &CollectorFixture) -> (Vec<Uuid>, Vec<Uuid>, Vec<Uuid>) {
    (
        sqlx::query_scalar("SELECT id FROM panda.charge_sessions ORDER BY id").fetch_all(&db.pool).await.unwrap(),
        sqlx::query_scalar("SELECT charge_session_id FROM timeseries.telemetry WHERE charge_session_id IS NOT NULL ORDER BY ts").fetch_all(&db.pool).await.unwrap(),
        sqlx::query_scalar("SELECT charge_session_id FROM panda.charge_session_external_aliases ORDER BY external_id").fetch_all(&db.pool).await.unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn canonicalization_defers_active_and_pending_ids_then_merges_atomically_after_drain() {
    let db = CollectorFixture::new().await;
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    let event = charging(&db, 0, 12.0);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &event,
        SampleSource::Socket,
    )
    .await;
    let id = charges.detector().active_session_id().unwrap();
    write_telemetry(&db.pool, &event, None, Some(id))
        .await
        .unwrap();
    let target = duplicate(&db, id).await;
    let before = references(&db).await;
    let stats = crate::services::charge_sessions::canonicalize_charge_sessions(
        &db.pool,
        Some(db.vehicle_id),
    )
    .await
    .unwrap();
    assert_eq!(stats.clusters_merged, 0);
    assert_eq!(references(&db).await, before);
    let mut end = db.event(180);
    end.charger_state = Some(ChargerState::Done);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &end,
        SampleSource::Socket,
    )
    .await;
    let stats = crate::services::charge_sessions::canonicalize_charge_sessions(
        &db.pool,
        Some(db.vehicle_id),
    )
    .await
    .unwrap();
    assert_eq!(
        stats.clusters_merged, 0,
        "pending completion pins its ID after detector becomes Idle"
    );
    assert_eq!(references(&db).await, before);
    assert!(charge_capture::drain_one(&db.pool, db.vehicle_id)
        .await
        .unwrap());
    db.pool.execute("CREATE FUNCTION synthetic_canonical_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic canonical failure'; END $$; CREATE TRIGGER synthetic_canonical_failure BEFORE DELETE ON panda.charge_sessions FOR EACH ROW EXECUTE FUNCTION synthetic_canonical_failure()").await.unwrap();
    assert!(
        crate::services::charge_sessions::canonicalize_charge_sessions(
            &db.pool,
            Some(db.vehicle_id)
        )
        .await
        .is_err()
    );
    assert_eq!(
        references(&db).await,
        before,
        "restamps and aliases roll back with failed deletion"
    );
    db.pool
        .execute("DROP TRIGGER synthetic_canonical_failure ON panda.charge_sessions")
        .await
        .unwrap();
    let stats = crate::services::charge_sessions::canonicalize_charge_sessions(
        &db.pool,
        Some(db.vehicle_id),
    )
    .await
    .unwrap();
    assert_eq!(stats.clusters_merged, 1);
    let (ids, telemetry, aliases) = references(&db).await;
    assert_eq!(ids, vec![target]);
    assert_eq!(telemetry, vec![target]);
    assert!(aliases.iter().all(|id| *id == target));
    assert!(ChargeCapture::load(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap()
        .detector()
        .active_session_id()
        .is_none());
    drop(conn);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn canonicalization_waits_for_the_pin_transaction_and_preserves_the_committed_id() {
    let db = CollectorFixture::new().await;
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    let event = charging(&db, 0, 12.0);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &event,
        SampleSource::Socket,
    )
    .await;
    let id = charges.detector().active_session_id().unwrap();
    write_telemetry(&db.pool, &event, None, Some(id))
        .await
        .unwrap();
    duplicate(&db, id).await;
    let mut tx = conn.begin().await.unwrap();
    crate::services::charge_sessions::lock_charge_history(&mut tx, db.vehicle_id)
        .await
        .unwrap();
    let before = references(&db).await;
    let pool = db.pool.clone();
    let vehicle = db.vehicle_id;
    let mut canonical = tokio::spawn(async move {
        crate::services::charge_sessions::canonicalize_charge_sessions(&pool, Some(vehicle)).await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut canonical)
            .await
            .is_err()
    );
    let sample = charging(&db, 60, 24.0);
    trips.process(&sample);
    charges
        .process(&sample, SampleSource::Socket, sample.ts)
        .unwrap();
    let trip_value = trips.write(&mut tx).await.unwrap();
    let charge_value = charges.write(&mut tx).await.unwrap();
    tx.commit().await.unwrap();
    trips.acknowledge(trip_value);
    charges.acknowledge(charge_value);
    assert_eq!(canonical.await.unwrap().unwrap().clusters_merged, 0);
    assert_eq!(references(&db).await, before);
    drop(conn);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn legacy_initialization_waits_for_canonicalization_without_resurrecting_merged_ids() {
    let db = CollectorFixture::new().await;
    let first = charging(&db, 0, 12.0);
    let old = Uuid::new_v4();
    ensure_active_charge_session(&db.pool, db.vehicle_id, old, &first)
        .await
        .unwrap();
    write_telemetry(&db.pool, &first, None, Some(old))
        .await
        .unwrap();
    let target = duplicate(&db, old).await;
    sqlx::query("UPDATE panda.charge_sessions SET ended_at=NULL WHERE id=$1")
        .bind(target)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut gate = db.pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(212341,7)")
        .execute(&mut *gate)
        .await
        .unwrap();
    db.pool.execute("CREATE FUNCTION synthetic_canonical_gate() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(212341,7); RETURN OLD; END $$; CREATE TRIGGER synthetic_canonical_gate BEFORE DELETE ON panda.charge_sessions FOR EACH ROW EXECUTE FUNCTION synthetic_canonical_gate()").await.unwrap();
    let pool = db.pool.clone();
    let vehicle = db.vehicle_id;
    let canonical = tokio::spawn(async move {
        crate::services::charge_sessions::canonicalize_charge_sessions(&pool, Some(vehicle)).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_locks WHERE locktype='advisory' AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND classid=212341 AND objid=7 AND NOT granted)").fetch_one(&db.pool).await.unwrap();
            if blocked {break;} tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let pool = db.pool.clone();
    let mut initialization = tokio::spawn(async move {
        let mut trips = trip_capture::TripCapture::load(&pool, vehicle)
            .await
            .unwrap();
        let mut lock = acquire_collector_lock(&pool, vehicle)
            .await
            .unwrap()
            .unwrap();
        charge_capture::initialize(&mut lock, &mut trips, vehicle)
            .await
            .unwrap()
            .detector()
            .active_session_id()
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut initialization)
            .await
            .is_err()
    );
    sqlx::query("SELECT pg_advisory_unlock(212341,7)")
        .execute(&mut *gate)
        .await
        .unwrap();
    assert_eq!(canonical.await.unwrap().unwrap().clusters_merged, 1);
    assert_eq!(initialization.await.unwrap(), None, "legacy canonicalization closes its merged row; initialization must observe that committed result");
    let (ids, telemetry, aliases) = references(&db).await;
    assert_eq!(ids, vec![target]);
    assert_eq!(telemetry, vec![target]);
    assert!(aliases.iter().all(|id| *id == target));
    drop(gate);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn migrated_restricted_application_role_records_and_completes_without_history_deletion() {
    let base = std::env::var("TRIP_DURABILITY_TEST_DATABASE_URL").unwrap();
    let admin = PgPool::connect(&base).await.unwrap();
    let name = format!("panda_charge_restricted_{}", Uuid::new_v4().simple());
    admin.execute(sqlx::AssertSqlSafe(format!("CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic-charge-role-only'"))).await.unwrap();
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {name} OWNER {name}"
        )))
        .await
        .unwrap();
    let mut admin_url = url::Url::parse(&base).unwrap();
    admin_url.set_path(&format!("/{name}"));
    let bootstrap = PgPool::connect(admin_url.as_str()).await.unwrap();
    bootstrap.execute(sqlx::AssertSqlSafe(format!("CREATE SCHEMA panda AUTHORIZATION {name}; CREATE SCHEMA timeseries AUTHORIZATION {name}; REVOKE CREATE ON SCHEMA public FROM PUBLIC; REVOKE ALL ON SCHEMA panda,timeseries FROM PUBLIC; CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda; CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda"))).await.unwrap();
    let mut app_url = admin_url.clone();
    app_url.set_username(&name).unwrap();
    app_url
        .set_password(Some("synthetic-charge-role-only"))
        .unwrap();
    let migration = PgPool::connect(app_url.as_str()).await.unwrap();
    crate::db::migrations::run_current_migrations(&migration)
        .await
        .unwrap();
    migration.close().await;
    let pool = crate::db::pool::create_pool(app_url.as_str())
        .await
        .unwrap();
    let config: Config = serde_json::from_value(
        serde_json::json!({"database_url":"unused","redis_url":"unused","panda_env":"production"}),
    )
    .unwrap();
    crate::private_deployment::history::protect_history(&pool, &config)
        .await
        .unwrap();
    let flags: (bool, bool, bool) = sqlx::query_as(
        "SELECT rolsuper,rolcreatedb,rolbypassrls FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(flags, (false, false, false));
    let user = Uuid::new_v4();
    let vehicle = Uuid::new_v4();
    sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,'restricted@example.invalid','unused')").bind(user).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,'synthetic-restricted-charge','R1T')").bind(vehicle).bind(user).execute(&pool).await.unwrap();
    let at = sqlx::query_scalar("SELECT date_trunc('milliseconds',now())-interval '10 minutes'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let db = CollectorFixture {
        pool,
        admin,
        name: name.clone(),
        vehicle_id: vehicle,
        at,
    };
    let (mut trips, mut charges, mut conn) = captures(&db).await;
    for (seconds, kw) in [(0, 12.0), (60, 24.0), (120, 6.0)] {
        let event = charging(&db, seconds, kw);
        commit_event(
            &mut trips,
            &mut charges,
            &mut conn,
            &event,
            SampleSource::Socket,
        )
        .await;
        write_telemetry(
            &db.pool,
            &event,
            None,
            charges.detector().active_session_id(),
        )
        .await
        .unwrap();
    }
    let mut end = db.event(180);
    end.charger_state = Some(ChargerState::Done);
    commit_event(
        &mut trips,
        &mut charges,
        &mut conn,
        &end,
        SampleSource::Socket,
    )
    .await;
    assert!(charge_capture::drain_one(&db.pool, db.vehicle_id)
        .await
        .unwrap());
    for table in [
        "panda.vehicles",
        "panda.trips",
        "timeseries.telemetry",
        "panda.vehicle_state_periods",
        "panda.software_versions",
        "panda.battery_capacity_snapshots",
    ] {
        for verb in ["DELETE FROM", "TRUNCATE"] {
            let error = db
                .pool
                .execute(sqlx::AssertSqlSafe(format!("{verb} {table}")))
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("42501"),
                "{verb} {table}"
            );
        }
    }
    let error = db
        .pool
        .execute("TRUNCATE panda.charge_sessions")
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    let energy: f64 =
        sqlx::query_scalar("SELECT energy_used_wh FROM panda.charge_sessions WHERE vehicle_id=$1")
            .bind(vehicle)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(energy, 500.0);
    drop(conn);
    bootstrap.close().await;
    db.finish().await;
    let admin = PgPool::connect(&base).await.unwrap();
    admin
        .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {name}")))
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn charge_migration_appends_to_existing_ledger_without_changing_data_or_identities() {
    let entries: Vec<_> = crate::db::migrations::MIGRATOR
        .iter()
        .filter(|m| m.version < 1_000_031)
        .cloned()
        .collect();
    let previous = sqlx::migrate::Migrator::with_migrations(entries);
    let db = CollectorFixture::with_migrator(&previous).await;
    let before: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT version,description,checksum FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(before.last().unwrap().0, 1_000_030);
    let result = crate::db::migrations::run_current_migrations(&db.pool)
        .await
        .unwrap();
    assert_eq!(result.latest_version, 1_000_031);
    let after: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT version,description,checksum FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(after.len(), before.len() + 1);
    let vehicle: Uuid = sqlx::query_scalar("SELECT id FROM panda.vehicles WHERE id=$1")
        .bind(db.vehicle_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(vehicle, db.vehicle_id);
    let (_, charges, conn) = captures(&db).await;
    assert!(charges.detector().active_session_id().is_none());
    drop(conn);
    db.finish().await;
}

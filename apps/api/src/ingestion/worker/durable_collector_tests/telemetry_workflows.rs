use super::super::telemetry_contract_tests::{all_fields, golden};
use super::*;
use serde_json::{json, Value};

async fn rows(db: &CollectorFixture, event: &TelemetryEvent) -> Value {
    let raw: Value = sqlx::query_scalar("SELECT to_jsonb(t) - 'vehicle_id' FROM timeseries.telemetry t WHERE vehicle_id=$1 AND ts=$2")
        .bind(db.vehicle_id).bind(event.ts).fetch_one(&db.pool).await.unwrap();
    let latest: Value = sqlx::query_scalar("SELECT to_jsonb(t) - 'vehicle_id' - 'updated_at' FROM panda.vehicle_latest_status t WHERE vehicle_id=$1")
        .bind(db.vehicle_id).fetch_one(&db.pool).await.unwrap();
    json!({"raw":raw,"latest":latest})
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn all_column_storage_and_latest_recency_golden() {
    let db = CollectorFixture::new().await;
    let mut event = all_fields();
    event.vehicle_id = db.vehicle_id;
    let trip = Uuid::from_u128(31);
    let charge = Uuid::from_u128(41);
    write_telemetry(&db.pool, &event, Some(trip), Some(charge))
        .await
        .unwrap();
    upsert_latest_status(&db.pool, &event).await.unwrap();
    let first = rows(&db, &event).await;
    let mut patch = blank_event(db.vehicle_id, event.ts);
    patch.drive_mode = Some(crate::models::telemetry::DriveMode::Unknown);
    patch.battery_level = Some(90.0);
    patch.latitude = Some(1.0);
    patch.longitude = Some(2.0);
    patch.door_front_left_closed = Some(false);
    patch.tire_rr_psi = Some(35.0);
    write_telemetry(
        &db.pool,
        &patch,
        Some(Uuid::from_u128(32)),
        Some(Uuid::from_u128(42)),
    )
    .await
    .unwrap();
    upsert_latest_status(&db.pool, &patch).await.unwrap();
    let timestampless = rows(&db, &event).await;
    patch.ts -= Duration::days(1);
    patch.battery_level_ts = Some(event.ts - Duration::days(1));
    patch.location_ts = Some(event.ts - Duration::days(1));
    patch.battery_level = Some(91.0);
    patch.tire_rr_psi = Some(36.0);
    upsert_latest_status(&db.pool, &patch).await.unwrap();
    let late = rows(&db, &event).await;
    patch.battery_level_ts = event.battery_level_ts;
    patch.location_ts = event.location_ts;
    upsert_latest_status(&db.pool, &patch).await.unwrap();
    let equal_sensor_timestamp = rows(&db, &event).await;
    golden(
        "database",
        json!({"first":first,"timestampless":timestampless,"late":late,"equal_sensor_timestamp":equal_sensor_timestamp}),
        include_str!("../telemetry_contract_fixtures/database.json"),
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn database_nonfinite_values_and_failed_write_leave_existing_rows_intact() {
    let db = CollectorFixture::new().await;
    let mut event = db.event(0);
    event.battery_level = Some(f64::NAN);
    event.power_kw = Some(f64::INFINITY);
    event.speed_mph = Some(f64::NEG_INFINITY);
    write_telemetry(&db.pool, &event, None, None).await.unwrap();
    upsert_latest_status(&db.pool, &event).await.unwrap();
    let stored: (f64, f64, f64) = sqlx::query_as(
        "SELECT battery_level,power_kw,speed_mph FROM timeseries.telemetry WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(stored.0.is_nan());
    assert_eq!(stored.1, f64::INFINITY);
    assert_eq!(stored.2, f64::NEG_INFINITY);
    sqlx::query("ALTER TABLE timeseries.telemetry ADD CONSTRAINT worker_reject_test CHECK (battery_level <> 99)").execute(&db.pool).await.unwrap();
    event.battery_level = Some(99.0);
    assert!(write_telemetry(&db.pool, &event, None, None).await.is_err());
    let retained: f64 =
        sqlx::query_scalar("SELECT battery_level FROM timeseries.telemetry WHERE vehicle_id=$1")
            .bind(db.vehicle_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(retained.is_nan());
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn state_rotation_failure_rolls_back_close_and_retry_ignores_stale_cache() {
    let db = CollectorFixture::new().await;
    let first = transition_state_period(&db.pool, db.vehicle_id, None, VehicleState::Drive, db.at)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("ALTER TABLE panda.vehicle_state_periods ADD CONSTRAINT worker_reject_state CHECK (state <> 'sleep')").execute(&db.pool).await.unwrap();
    assert!(transition_state_period(
        &db.pool,
        db.vehicle_id,
        Some(first.clone()),
        VehicleState::Sleep,
        db.at + Duration::seconds(10)
    )
    .await
    .is_err());
    let current = load_open_state_period(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.id, first.id);
    sqlx::query("ALTER TABLE panda.vehicle_state_periods DROP CONSTRAINT worker_reject_state")
        .execute(&db.pool)
        .await
        .unwrap();
    let next = transition_state_period(
        &db.pool,
        db.vehicle_id,
        None,
        VehicleState::Sleep,
        db.at + Duration::seconds(10),
    )
    .await
    .unwrap()
    .unwrap();
    let same = transition_state_period(
        &db.pool,
        db.vehicle_id,
        Some(first.clone()),
        VehicleState::Sleep,
        db.at - Duration::seconds(1),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(same.id, next.id);
    let late = transition_state_period(
        &db.pool,
        db.vehicle_id,
        Some(first),
        VehicleState::Drive,
        db.at,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(late.id, next.id);
    let zero = transition_state_period(
        &db.pool,
        db.vehicle_id,
        None,
        VehicleState::Ready,
        next.started_at,
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(zero.id, next.id);
    let periods: Vec<(String,DateTime<Utc>,Option<DateTime<Utc>>)> = sqlx::query_as("SELECT state,started_at,ended_at FROM panda.vehicle_state_periods WHERE vehicle_id=$1 ORDER BY id")
        .bind(db.vehicle_id).fetch_all(&db.pool).await.unwrap();
    assert_eq!(
        periods,
        vec![
            ("drive".into(), db.at, Some(next.started_at)),
            ("sleep".into(), next.started_at, Some(next.started_at)),
            ("ready".into(), next.started_at, None)
        ]
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn reconciliation_reads_one_latest_meaningful_row_without_filling_sparse_state() {
    let db = CollectorFixture::new().await;
    let first = transition_state_period(&db.pool, db.vehicle_id, None, VehicleState::Drive, db.at)
        .await
        .unwrap()
        .unwrap();
    let mut sleep = db.event(10);
    sleep.power_state = Some(PowerState::Sleep);
    write_telemetry(&db.pool, &sleep, None, None).await.unwrap();
    let mut partial = db.event(20);
    partial.ota_status = Some("idle".into());
    write_telemetry(&db.pool, &partial, None, None)
        .await
        .unwrap();
    let unchanged = reconcile_state_period_tail(&db.pool, db.vehicle_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.id, first.id);
    partial.ts += Duration::seconds(10);
    partial.is_online = Some(false);
    write_telemetry(&db.pool, &partial, None, None)
        .await
        .unwrap();
    let offline = reconcile_state_period_tail(&db.pool, db.vehicle_id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(offline.state, VehicleState::Offline);
    assert_eq!(offline.started_at, partial.ts);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn failed_telemetry_write_keeps_gate_retryable_without_blocking_durable_or_live_state() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, true).await;
    db.pool.execute("ALTER TABLE timeseries.telemetry ADD CONSTRAINT worker_reject_write CHECK(false) NOT VALID").await.unwrap();
    let mut event = db.event(0);
    event.power_state = Some(PowerState::Ready);
    event.power_state_ts = Some(event.ts);
    event.battery_level = Some(70.0);
    harness.telemetry(event.clone(), "baseline").await;
    let counts: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM timeseries.telemetry),(SELECT count(*) FROM panda.vehicle_latest_status),(SELECT count(*) FROM panda.active_trip_checkpoints),(SELECT count(*) FROM panda.active_charge_checkpoints)")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (0, 1, 1, 1));
    assert_eq!(*harness.power.borrow(), Some(PowerState::Ready));
    assert_eq!(
        load_open_state_period(&db.pool, db.vehicle_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        VehicleState::Ready
    );
    db.pool
        .execute("ALTER TABLE timeseries.telemetry DROP CONSTRAINT worker_reject_write")
        .await
        .unwrap();
    harness.telemetry(event.clone(), "baseline").await;
    harness.pipeline.flush().await;
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM timeseries.telemetry),telemetry_writes_persisted,telemetry_writes_suppressed FROM panda.rivian_stewardship_counters WHERE vehicle_id=$1")
        .bind(db.vehicle_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (1, 1, 0));
    event.ts += Duration::seconds(30);
    harness.telemetry(event, "parallax:battery").await;
    harness.pipeline.flush().await;
    let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM timeseries.telemetry),telemetry_writes_suppressed,telemetry_suppressed_duplicate FROM panda.rivian_stewardship_counters WHERE vehicle_id=$1")
        .bind(db.vehicle_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
    let checkpoint: Value = sqlx::query_scalar(
        "SELECT snapshot FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
    )
    .bind(db.vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(checkpoint["sequence"], 3);
    assert_eq!(checkpoint["last_observation"]["source"], "parallax");
    drop(harness);
    db.finish().await;
}

use super::*;
use crate::ingestion::charge_detector::CompletedChargeSession;
use crate::ingestion::trip_detector::{CompletedTripData, TrackPoint};
use serde_json::{json, Value};

fn trip(db: &CollectorFixture, id: u128) -> CompletedTripData {
    CompletedTripData {
        trip_id: Uuid::from_u128(id),
        vehicle_id: db.vehicle_id,
        started_at: db.at,
        ended_at: db.at + Duration::seconds(120),
        points: vec![
            TrackPoint {
                ts: db.at,
                lat: 30.0,
                lng: -97.0,
                speed_mph: 12.0,
                altitude_m: Some(100.0),
            },
            TrackPoint {
                ts: db.at + Duration::seconds(120),
                lat: 30.01,
                lng: -97.01,
                speed_mph: 30.0,
                altitude_m: Some(102.0),
            },
        ],
        soc_start: Some(70.0),
        soc_end: Some(69.0),
        battery_capacity_wh: Some(100_000.0),
        start_odometer_mi: Some(100.0),
        end_odometer_mi: Some(102.0),
        range_start_mi: Some(200.0),
        range_end_mi: Some(198.0),
        power_max_kw: Some(20.0),
        power_min_kw: Some(-5.0),
        elevation_gain_m: Some(3.0),
        elevation_loss_m: Some(1.0),
        inside_temp_avg_c: Some(21.0),
        outside_temp_avg_c: Some(10.0),
        regen_wh: Some(100.0),
        dominant_drive_mode: Some("all_purpose".into()),
    }
}

fn charge(db: &CollectorFixture, id: u128) -> CompletedChargeSession {
    CompletedChargeSession {
        session_id: Uuid::from_u128(id),
        vehicle_id: db.vehicle_id,
        started_at: db.at,
        ended_at: db.at + Duration::seconds(125),
        location_lat: None,
        location_lng: None,
        soc_start: Some(40.0),
        soc_end: Some(45.0),
        charge_limit: Some(80.0),
        battery_capacity_wh: Some(100_000.0),
        energy_used_wh: Some(5500.0),
        energy_added_wh: Some(5000.0),
        peak_charge_kw: Some(150.0),
        avg_charge_rate_kw: Some(100.0),
        charger_type: Some("dc".into()),
    }
}

fn normalize(value: &mut Value, db: &CollectorFixture) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "created_at" | "updated_at" | "computed_at" | "next_attempt_at"
                ) && !value.is_null()
                {
                    *value = json!("<clock>");
                } else {
                    normalize(value, db);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize(value, db);
            }
        }
        Value::String(text) if text == &db.vehicle_id.to_string() => *value = json!("<vehicle>"),
        Value::String(text) => {
            if let Ok(at) = DateTime::parse_from_rfc3339(text) {
                *value = json!(format!(
                    "@{}ms",
                    at.signed_duration_since(db.at).num_milliseconds()
                ));
            }
        }
        _ => {}
    }
}

fn golden(name: &str, mut actual: Value, expected: &str, db: &CollectorFixture) {
    normalize(&mut actual, db);
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(expected).unwrap(),
        "{name}"
    );
}

async fn trip_rows(db: &CollectorFixture) -> Value {
    let trips: Value = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]') FROM panda.trips t",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let jobs: Value = sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY trip_id),'[]') FROM panda.weather_enrichment_jobs t")
        .fetch_one(&db.pool).await.unwrap();
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM panda.pending_trip_completions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    json!({"trips":trips,"jobs":jobs,"pending":pending})
}

async fn enqueue(db: &CollectorFixture, trip: &CompletedTripData) {
    sqlx::query("INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip) VALUES($1,$2,$3) ON CONFLICT(trip_id) DO UPDATE SET trip=EXCLUDED.trip")
        .bind(trip.trip_id).bind(db.vehicle_id).bind(serde_json::to_value(trip).unwrap())
        .execute(&db.pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_trip_projection_complete_rows_fallback_and_admission() {
    let db = CollectorFixture::new().await;
    let full = trip(&db, 101);
    enqueue(&db, &full).await;
    persist_trip(&db.pool, &full, 2.0).await.unwrap();
    let mut before = db.event(-60);
    before.battery_level = Some(81.0);
    write_telemetry(&db.pool, &before, None, None)
        .await
        .unwrap();
    let mut end = db.event(100);
    end.battery_level = Some(80.0);
    write_telemetry(&db.pool, &end, None, None).await.unwrap();
    let mut fallback = trip(&db, 102);
    fallback.soc_start = None;
    fallback.soc_end = None;
    fallback.range_start_mi = None;
    fallback.range_end_mi = None;
    enqueue(&db, &fallback).await;
    persist_trip(&db.pool, &fallback, 2.0).await.unwrap();
    let mut short = trip(&db, 103);
    short.points.clear();
    enqueue(&db, &short).await;
    persist_trip(&db.pool, &short, 0.099).await.unwrap();
    let mut zero = trip(&db, 104);
    zero.ended_at = zero.started_at;
    zero.soc_start = None;
    zero.soc_end = None;
    zero.battery_capacity_wh = None;
    zero.points.clear();
    enqueue(&db, &zero).await;
    persist_trip(&db.pool, &zero, 0.1).await.unwrap();
    persist_trip(&db.pool, &full, 2.0).await.unwrap();
    let scheduled_at_creation: bool = sqlx::query_scalar(
        "SELECT bool_and(next_attempt_at=created_at) FROM panda.weather_enrichment_jobs",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(scheduled_at_creation);
    golden(
        "trip-completions",
        trip_rows(&db).await,
        include_str!("../residual_contract_fixtures/trip-completions.json"),
        &db,
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_trip_completion_failure_keeps_outbox_and_identity_collision() {
    let db = CollectorFixture::new().await;
    let mut data = trip(&db, 105);
    enqueue(&db, &data).await;
    db.pool.execute("ALTER TABLE panda.weather_enrichment_jobs ADD CONSTRAINT residual_reject_job CHECK(false) NOT VALID").await.unwrap();
    assert!(persist_trip(&db.pool, &data, 2.0).await.is_err());
    assert_eq!(
        trip_rows(&db).await,
        json!({"trips":[],"jobs":[],"pending":1})
    );
    db.pool
        .execute("ALTER TABLE panda.weather_enrichment_jobs DROP CONSTRAINT residual_reject_job")
        .await
        .unwrap();
    persist_trip(&db.pool, &data, 2.0).await.unwrap();
    data.ended_at += Duration::seconds(1);
    enqueue(&db, &data).await;
    assert!(persist_trip(&db.pool, &data, 2.0)
        .await
        .unwrap_err()
        .to_string()
        .contains("identity collision"));
    assert_eq!(trip_rows(&db).await["pending"], 1);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_charge_materialization_and_completed_merge_rows() {
    let db = CollectorFixture::new().await;
    let id = Uuid::from_u128(201);
    let mut observed = db.event(-120);
    observed.battery_level = Some(38.0);
    observed.battery_limit = Some(75.0);
    write_telemetry(&db.pool, &observed, None, Some(id))
        .await
        .unwrap();
    let mut active = db.event(0);
    active.battery_level = Some(40.0);
    active.battery_limit = Some(80.0);
    active.latitude = Some(0.0);
    active.longitude = Some(0.0);
    ensure_active_charge_session(&db.pool, db.vehicle_id, id, &active)
        .await
        .unwrap();
    active.latitude = Some(30.0);
    active.longitude = Some(-97.0);
    ensure_active_charge_session(&db.pool, db.vehicle_id, id, &active)
        .await
        .unwrap();
    sqlx::query("UPDATE panda.charge_sessions SET source='api',data_confidence='api',kwh_added=7.0,cost_usd=2.0 WHERE id=$1")
        .bind(id).execute(&db.pool).await.unwrap();
    let mut complete = charge(&db, 201);
    complete.energy_added_wh = None;
    let mut conn = db.pool.acquire().await.unwrap();
    persist_charge_session_record(&mut conn, &complete, None, None, &MatchedLocation::none())
        .await
        .unwrap();
    drop(conn);
    let coordinates = charge_session_coordinates(&db.pool, &complete).await;
    let rows: Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM panda.charge_sessions t",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    golden(
        "charge-completions",
        json!({"rows":rows,"coordinates":coordinates}),
        include_str!("../residual_contract_fixtures/charge-completions.json"),
        &db,
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn residual_software_failure_retries_before_and_after_restart() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, true).await;
    let mut event = db.event(0);
    event.ota_current_version = Some("synthetic-v1".into());
    harness.telemetry(event.clone(), "baseline").await;
    db.pool.execute("ALTER TABLE panda.software_versions ADD CONSTRAINT residual_reject_version CHECK(version <> 'synthetic-v2')").await.unwrap();
    event.ts += Duration::seconds(60);
    event.ota_current_version = Some("synthetic-v2".into());
    harness.telemetry(event.clone(), "baseline").await;
    db.pool
        .execute("ALTER TABLE panda.software_versions DROP CONSTRAINT residual_reject_version")
        .await
        .unwrap();
    event.ts += Duration::seconds(60);
    harness.telemetry(event.clone(), "baseline").await;
    let before: Vec<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT version,observed_until FROM panda.software_versions ORDER BY installed_at",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        before,
        vec![
            ("synthetic-v1".into(), Some(db.at + Duration::seconds(120))),
            ("synthetic-v2".into(), None)
        ]
    );
    drop(harness);
    let mut restarted = PipelineHarness::new(&db, true).await;
    restarted.telemetry(event, "baseline").await;
    let versions: Vec<String> =
        sqlx::query_scalar("SELECT version FROM panda.software_versions ORDER BY installed_at")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(versions, vec!["synthetic-v1", "synthetic-v2"]);
    drop(restarted);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_counter_and_legacy_flush_failures_retain_batches() {
    let db = CollectorFixture::new().await;
    let mut counters = CounterBatch::new(db.vehicle_id);
    counters.increment("telemetry_writes_persisted");
    counters.increment("untrusted_column");
    db.pool.execute("ALTER TABLE panda.rivian_stewardship_counters ADD CONSTRAINT residual_reject_count CHECK(false) NOT VALID").await.unwrap();
    counters.flush(&db.pool).await;
    assert_eq!(counters.counts.get("telemetry_writes_persisted"), Some(&1));
    db.pool
        .execute(
            "ALTER TABLE panda.rivian_stewardship_counters DROP CONSTRAINT residual_reject_count",
        )
        .await
        .unwrap();
    counters.flush(&db.pool).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM panda.rivian_stewardship_counters")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        1
    );
    let mut counts = [1, 2, 3, 4, 5];
    db.pool.execute("ALTER TABLE panda.parallax_collector_state ADD CONSTRAINT residual_reject_legacy CHECK(false) NOT VALID").await.unwrap();
    flush_legacy_charging_diagnostics(
        &db.pool,
        db.vehicle_id,
        &mut counts,
        Some(LegacyChargingSessionClass::Meaningful),
        Some(db.at),
        Some(db.at),
    )
    .await;
    assert_eq!(counts, [1, 2, 3, 4, 5]);
    db.pool
        .execute(
            "ALTER TABLE panda.parallax_collector_state DROP CONSTRAINT residual_reject_legacy",
        )
        .await
        .unwrap();
    flush_legacy_charging_diagnostics(
        &db.pool,
        db.vehicle_id,
        &mut counts,
        Some(LegacyChargingSessionClass::Meaningful),
        Some(db.at),
        Some(db.at),
    )
    .await;
    assert_eq!(counts, [0; 5]);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_raw_accounting_and_successful_counter_flush() {
    let db = CollectorFixture::new().await;
    let mut config: Config = serde_json::from_value(
        json!({"database_url":"synthetic-unused","redis_url":"synthetic-unused"}),
    )
    .unwrap();
    config.rivian_persist_raw_events = true;
    let mut batch = CounterBatch::new(db.vehicle_id);
    for (kind, message_type, raw) in [
        (WsInboundKind::Control, "reconnect", "not-json"),
        (WsInboundKind::Heartbeat, "ka", "{}"),
        (WsInboundKind::Telemetry, "next", "null"),
        (WsInboundKind::Control, "subscribe", "[]"),
    ] {
        handle_inbound_accounting(
            &db.pool,
            db.vehicle_id,
            &config,
            &WsInboundEvent {
                kind,
                received_at: db.at,
                raw: raw.into(),
                message_type: Some(message_type.into()),
                telemetry: None,
                charging_session: None,
                battery_cell_type: None,
            },
            &mut batch,
        )
        .await;
    }
    assert_eq!(batch.counts.get("raw_events_persisted"), Some(&4));
    let raw: Vec<(String, Option<Value>)> = sqlx::query_as(
        "SELECT payload_text,payload_json FROM panda.rivian_ws_raw_events ORDER BY payload_text",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        raw,
        vec![
            ("[]".into(), Some(json!([]))),
            ("not-json".into(), None),
            ("null".into(), Some(Value::Null)),
            ("{}".into(), Some(json!({})))
        ]
    );
    batch.flush(&db.pool).await;
    let persisted: Vec<(i64,i64,i64,i64,i64)> = sqlx::query_as("SELECT ws_messages_received,ws_heartbeats_received,ws_payload_messages_received,outbound_messages_sent,raw_events_persisted FROM panda.rivian_stewardship_counters")
        .fetch_all(&db.pool).await.unwrap();
    assert_eq!(persisted, vec![(2, 1, 1, 1, 4)]);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_retention_floor_partial_failure_and_session_lock() {
    let db = CollectorFixture::new().await;
    for days in [0, 2] {
        sqlx::query("INSERT INTO panda.rivian_ws_raw_events(vehicle_id,received_at,event_type,payload_text) VALUES($1,now()-($2 * interval '1 day'),'telemetry','{}')")
            .bind(db.vehicle_id).bind(days).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO panda.rivian_parallax_events(vehicle_id,received_at,rvm,payload_b64) VALUES($1,now()-($2 * interval '1 day'),'synthetic','e30=')")
            .bind(db.vehicle_id).bind(days).execute(&db.pool).await.unwrap();
    }
    let options = db.pool.connect_options();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .min_connections(2)
        .connect_with((*options).clone())
        .await
        .unwrap();
    let first = pool.acquire().await.unwrap();
    let second = pool.acquire().await.unwrap();
    drop(first);
    drop(second);
    cleanup_raw_events(&pool, 0).await;
    let count: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM panda.rivian_ws_raw_events),(SELECT count(*) FROM panda.rivian_parallax_events)").fetch_one(&db.pool).await.unwrap();
    assert_eq!(count, (1, 1));
    let available: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(0x726d5241::bigint)")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(
        available,
        "retention lock must be released despite pool rotation"
    );
    pool.close().await;
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn residual_live_cache_survives_database_failure_and_expires() {
    let db = CollectorFixture::new().await;
    let redis_client =
        redis::Client::open(std::env::var("DURABLE_COLLECTOR_TEST_REDIS_URL").unwrap()).unwrap();
    let mut redis = redis_client
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let key = rivian_poll::live_session_redis_key(db.vehicle_id);
    let id = Uuid::from_u128(202);
    ensure_active_charge_session(&db.pool, db.vehicle_id, id, &db.event(0))
        .await
        .unwrap();
    let raw = r#"{"type":"next","payload":{"data":{"chargingSession":{"liveData":{"powerKW":9.5,"totalChargedEnergy":12.5},"chartData":[]}}}}"#;
    let event = parser::parse_charging_session_message(raw)
        .unwrap()
        .unwrap();
    db.pool.execute("ALTER TABLE panda.charge_sessions ADD CONSTRAINT residual_reject_live CHECK(live_total_charged_kwh IS NULL)").await.unwrap();
    apply_charging_session_update(&db.pool, &mut redis, db.vehicle_id, &event, db.at, Some(id))
        .await;
    assert!(redis.exists::<_, bool>(&key).await.unwrap());
    let ttl: i64 = redis.ttl(&key).await.unwrap();
    assert!(ttl > 0 && ttl <= rivian_poll::LIVE_SESSION_TTL_SECONDS as i64);
    let persisted: Option<f64> =
        sqlx::query_scalar("SELECT live_total_charged_kwh FROM panda.charge_sessions WHERE id=$1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(persisted, None);
    db.pool
        .execute("ALTER TABLE panda.charge_sessions DROP CONSTRAINT residual_reject_live")
        .await
        .unwrap();
    apply_charging_session_update(&db.pool, &mut redis, db.vehicle_id, &event, db.at, Some(id))
        .await;
    let persisted: Option<f64> =
        sqlx::query_scalar("SELECT live_total_charged_kwh FROM panda.charge_sessions WHERE id=$1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(persisted, Some(12.5));
    redis.pexpire::<_, ()>(&key, 1).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(!redis.exists::<_, bool>(&key).await.unwrap());
    db.finish().await;
}

#[tokio::test(start_paused = true)]
async fn residual_acquisition_tasks_abort_and_release_owned_resources() {
    let (released, mut release_rx) = mpsc::unbounded_channel();
    struct Release(mpsc::UnboundedSender<()>);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let (ready, started) = tokio::sync::oneshot::channel();
    let task = WorkerTask(tokio::spawn(async move {
        let _resource = Release(released);
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
    }));
    started.await.unwrap();
    drop(task);
    assert_eq!(release_rx.recv().await, Some(()));
    let mut task = WorkerTask(tokio::spawn(std::future::pending::<()>()));
    task.abort();
    assert!((&mut task).await.unwrap_err().is_cancelled());
}

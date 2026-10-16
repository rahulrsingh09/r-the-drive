use super::*;
use chrono::{DateTime, Duration as ChronoDuration};
use panda_api::{
    ingestion::{
        charge_detector::{
            ActiveChargeSessionSnapshot, ChargeDetectorState, ChargeEvent, CompletedChargeSession,
        },
        trip_detector::{TripDetectorState, TripEvent},
    },
    models::telemetry::{ChargerState, PowerState, TelemetryEvent},
};

fn at() -> DateTime<Utc> {
    "2020-01-01T10:00:00Z".parse().unwrap()
}

fn trip_state(vehicle: Uuid, trip: Uuid) -> TripDetectorState {
    let mut detector = TripDetectorState::new(vehicle);
    for minute in 0..2 {
        let mut event = TelemetryEvent::empty(vehicle, at() + ChronoDuration::minutes(minute));
        event.power_state = Some(PowerState::Drive);
        event.speed_mph = Some(12.5);
        event.latitude = Some(47.625);
        event.longitude = Some(-122.25 + minute as f64 / 256.0);
        event.power_kw = Some(13.125);
        event.battery_level = Some(75.25 - minute as f64 / 4.0);
        event.battery_capacity_wh = Some(135_000.0);
        event.odometer_miles = Some(100.125 + minute as f64);
        event.distance_to_empty_mi = Some(256.125 - minute as f64);
        detector.process(&event);
    }
    let mut snapshot = serde_json::to_value(detector).unwrap();
    snapshot["active_trip_id"] = json!(trip);
    serde_json::from_value(snapshot).unwrap()
}

async fn seed(pool: &PgPool) {
    for offset in [0_u128, 10] {
        let user = Uuid::from_u128(7000 + offset);
        let vehicle = Uuid::from_u128(7001 + offset);
        let trip = Uuid::from_u128(7002 + offset);
        let session = Uuid::from_u128(7004 + offset);
        sqlx::query("INSERT INTO panda.users(id,email,password_hash,created_at,updated_at) VALUES($1,$2,'synthetic-only',$3,$3)")
            .bind(user).bind(format!("durable-{offset}@example.invalid")).bind(at()).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES($1,$2,$3,'R1T',$4,$4)")
            .bind(vehicle).bind(user).bind(format!("synthetic-durable-{offset}")).bind(at()).execute(pool).await.unwrap();
        let mut trip_detector = trip_state(vehicle, trip);
        if offset != 0 {
            let mut stop = TelemetryEvent::empty(vehicle, at() + ChronoDuration::minutes(2));
            stop.power_state = Some(PowerState::Sleep);
            let TripEvent::TripEnded { trip: completion } = trip_detector.process(&stop) else {
                panic!("synthetic completed trip");
            };
            sqlx::query("INSERT INTO panda.pending_trip_completions(trip_id,vehicle_id,trip,created_at) VALUES($1,$2,$3,$4)")
                .bind(trip).bind(vehicle).bind(serde_json::to_value(completion).unwrap()).bind(at()).execute(pool).await.unwrap();
        }
        sqlx::query("INSERT INTO panda.active_trip_checkpoints(vehicle_id,snapshot,updated_at) VALUES($1,$2,$3)")
            .bind(vehicle).bind(json!({"version":1,"detector":trip_detector,"last_event_at":at()+ChronoDuration::minutes(1)})).bind(at()).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,soc_start,source,created_at) VALUES($1,$2,$3,30,'telemetry',$3)")
            .bind(session).bind(vehicle).bind(at()).execute(pool).await.unwrap();
        let charge = if offset == 0 {
            ChargeDetectorState::from_snapshot(
                vehicle,
                ActiveChargeSessionSnapshot {
                    session_id: session,
                    started_at: at(),
                    location_lat: None,
                    location_lng: None,
                    soc_start: Some(30.0),
                    last_soc: Some(f64::from_bits(0x7ff8_0000_0000_0042)),
                    charge_limit: Some(-0.0),
                    battery_capacity_wh: Some(135_000.0),
                    last_charge_ts: at() + ChronoDuration::minutes(1),
                    last_power_ts: Some(at() + ChronoDuration::minutes(1)),
                    energy_used_wh: f64::from_bits(0x409f_4000_0000_0001),
                    peak_charge_kw: f64::from_bits(0x4028_3f35_ba78_19af),
                },
            )
        } else {
            let completion = CompletedChargeSession {
                session_id: session,
                vehicle_id: vehicle,
                started_at: at(),
                ended_at: at() + ChronoDuration::minutes(2),
                location_lat: None,
                location_lng: None,
                soc_start: Some(30.0),
                soc_end: Some(31.25),
                charge_limit: Some(-0.0),
                battery_capacity_wh: Some(135_000.0),
                energy_used_wh: Some(f64::from_bits(0x409f_4000_0000_0001)),
                energy_added_wh: Some(1687.5),
                peak_charge_kw: Some(f64::from_bits(0x4028_3f35_ba78_19af)),
                avg_charge_rate_kw: Some(30.000000000000007),
                charger_type: Some("ac".into()),
            };
            sqlx::query("INSERT INTO panda.pending_charge_completions(session_id,vehicle_id,session,created_at) VALUES($1,$2,$3,$4)")
                .bind(session).bind(vehicle).bind(json!({"version":1,"session":completion})).bind(at()).execute(pool).await.unwrap();
            ChargeDetectorState::new(vehicle)
        };
        sqlx::query("INSERT INTO panda.active_charge_checkpoints(vehicle_id,active_session_id,snapshot,updated_at) VALUES($1,$2,$3,$4)")
            .bind(vehicle).bind(charge.active_session_id()).bind(json!({"version":1,"sequence":7,"detector":charge,"last_observation":{"at":at(),"received_at":at(),"source":"socket"}})).bind(at()).execute(pool).await.unwrap();
    }
}

async fn complete_state(pool: &PgPool) -> Value {
    let mut state = serde_json::Map::new();
    for (table, order) in [
        ("panda.users", "id"),
        ("panda.vehicles", "id"),
        ("panda.trips", "id"),
        ("panda.charge_sessions", "id"),
        ("panda.active_trip_checkpoints", "vehicle_id"),
        ("panda.pending_trip_completions", "trip_id"),
        ("panda.active_charge_checkpoints", "vehicle_id"),
        ("panda.pending_charge_completions", "session_id"),
        ("timeseries.telemetry", "vehicle_id,ts"),
        ("panda.weather_enrichment_jobs", "trip_id"),
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM {table} t ORDER BY {order}"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        state.insert(table.into(), json!(rows));
    }
    let bits: Vec<(Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id,encode(float8send(energy_used_wh),'hex'),encode(float8send(max_charge_rate_kw),'hex') FROM panda.charge_sessions ORDER BY id"
    ).fetch_all(pool).await.unwrap();
    state.insert("charge_float_bits".into(), json!(bits));
    json!(state)
}

fn normalize_clocks(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if (key.ends_with("_at") || key == "last_seen")
                    && value.as_str().is_some_and(|s| {
                        !s.starts_with("2020-") && s.parse::<DateTime<Utc>>().is_ok()
                    })
                {
                    *value = json!("<clock>");
                } else {
                    normalize_clocks(value);
                }
            }
        }
        Value::Array(rows) => rows.iter_mut().for_each(normalize_clocks),
        _ => {}
    }
}

async fn assert_pins(pool: &PgPool) {
    for session in [7004_u128, 7014] {
        let error = sqlx::query("DELETE FROM panda.charge_sessions WHERE id=$1")
            .bind(Uuid::from_u128(session))
            .execute(pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("23001")
        );
    }
}

#[tokio::test]
#[ignore = "requires explicitly isolated synthetic restore server and PostgreSQL tools"]
async fn nonempty_durable_state_survives_rollback_restore_and_worker_restart() {
    let fixture = DatabaseFixture::new().await;
    seed(&fixture.pool).await;
    assert_pins(&fixture.pool).await;
    let before = complete_state(&fixture.pool).await;
    for table in [
        "panda.active_trip_checkpoints",
        "panda.pending_trip_completions",
        "panda.active_charge_checkpoints",
        "panda.pending_charge_completions",
    ] {
        assert!(
            !before[table].as_array().unwrap().is_empty(),
            "nonempty {table}"
        );
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture.config.clone();
    config.port = listener.local_addr().unwrap().port();
    let health = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/health", get(|| async { StatusCode::OK }))
                .route(
                    "/v1/auth/setup",
                    get(|| async { Json(json!({"setup_required":false})) }),
                ),
        )
        .await
        .unwrap();
    });
    fs::create_dir_all(&config.vehicle_image_cache_dir)
        .await
        .unwrap();
    let backup = backups::run_backup_now(
        &fixture.pool,
        &config,
        None,
        backups::BackupRunTrigger::Manual,
    )
    .await
    .unwrap();
    let package: String =
        sqlx::query_scalar("SELECT storage_path FROM panda.backup_artifacts WHERE run_id=$1")
            .bind(backup.run_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let validated =
        backups::validate_recovery_package_with_limits(Path::new(&package), &config.recovery)
            .await
            .unwrap();
    let inspection = restore_compatibility::inspect_recovery_dump(
        Path::new(&package),
        Path::new(&config.backup_artifact_dir),
        &config.recovery,
    )
    .await
    .unwrap();
    let plan = restore_compatibility::plan_restore(
        &validated.manifest,
        &validated.checksum_sha256,
        &fixture.pool,
        Some(&inspection),
    )
    .await
    .unwrap();
    assert!(plan.compatible);
    fixture.pool.close().await;
    let mut stages = vec![json!({"stage":"before","state":before})];
    for (number, fault) in [(8000_u128, Some("database_swapped")), (8001, None)] {
        let mut job = fixed_job();
        job.id = Uuid::from_u128(number);
        job.artifact_path = package.clone();
        job.plan = Some(plan.clone());
        restore_jobs::write(&config, &job).await.unwrap();
        if let Some(fault) = fault {
            std::env::set_var("PANDA_RESTORE_FAULT_PHASE", fault);
        }
        let result = perform_restore_inner(&config, job.id).await;
        std::env::remove_var("PANDA_RESTORE_FAULT_PHASE");
        if fault.is_some() {
            assert_eq!(result.unwrap_err().to_string(),"restored candidate activation failed and was rolled back: restore fault injection triggered at database_swapped");
        } else {
            result.unwrap();
        }
        let pool = PgPool::connect(&config.database_url).await.unwrap();
        let restored = complete_state(&pool).await;
        assert_eq!(before, restored, "complete durable state after {fault:?}");
        assert_pins(&pool).await;
        stages.push(
            json!({"stage":if fault.is_some() {"rollback"} else {"restored"},"state":restored}),
        );
        pool.close().await;
        cleanup_candidate(&config, &format!("panda_restore_{}", job.id.simple())).await;
    }
    let admin = PgPool::connect(&config.database_url).await.unwrap();
    admin.execute(sqlx::AssertSqlSafe(format!(
        "GRANT CONNECT ON DATABASE panda TO {role};
         GRANT USAGE ON SCHEMA panda,timeseries TO {role};
         GRANT SELECT,INSERT,UPDATE ON ALL TABLES IN SCHEMA panda,timeseries TO {role};
         GRANT USAGE,SELECT,UPDATE ON ALL SEQUENCES IN SCHEMA panda,timeseries TO {role};
         GRANT DELETE ON panda.pending_trip_completions,panda.pending_charge_completions TO {role};",
        role=fixture.role
    ))).await.unwrap();
    admin.close().await;
    let mut url = url::Url::parse(&config.database_url).unwrap();
    url.set_username(&fixture.role).unwrap();
    url.set_password(Some("synthetic_app_only")).unwrap();
    let app = panda_api::db::pool::application_pool_options()
        .max_connections(4)
        .connect(url.as_str())
        .await
        .unwrap();
    panda_api::private_deployment::history::protect_history(&app, &config)
        .await
        .unwrap();
    let mut previous = None;
    for replay in 0..2 {
        for vehicle in [7001_u128, 7011] {
            let (_sender, shutdown) = tokio::sync::broadcast::channel(1);
            tokio::time::timeout(
                Duration::from_secs(15),
                panda_api::ingestion::worker::run_vehicle_worker(
                    Uuid::from_u128(vehicle),
                    app.clone(),
                    redis::Client::open("redis://127.0.0.1:1").unwrap(),
                    config.age_encryption_key.clone().unwrap(),
                    config.clone(),
                    shutdown,
                ),
            )
            .await
            .expect("credential-free worker must finish before any provider connection");
        }
        let mut state = complete_state(&app).await;
        evidence(&format!("durable-restart-{replay}-raw"), &state);
        assert_eq!(state["panda.pending_trip_completions"], json!([]));
        assert_eq!(state["panda.pending_charge_completions"], json!([]));
        assert_eq!(state["panda.trips"].as_array().unwrap().len(), 2);
        assert_eq!(state["charge_float_bits"][1][1], json!("409f400000000001"));
        normalize_clocks(&mut state);
        if let Some(previous) = previous {
            assert_eq!(previous, state, "idempotent actual completion drain");
        }
        previous = Some(state.clone());
        stages.push(json!({"stage":format!("restart-{replay}"),"state":state}));
    }
    let original_detector: ChargeDetectorState = serde_json::from_value(
        before["panda.active_charge_checkpoints"][0]["snapshot"]["detector"].clone(),
    )
    .unwrap();
    let saved: Value = sqlx::query_scalar(
        "SELECT snapshot FROM panda.active_charge_checkpoints WHERE vehicle_id=$1",
    )
    .bind(Uuid::from_u128(7001))
    .fetch_one(&app)
    .await
    .unwrap();
    let restored_detector: ChargeDetectorState =
        serde_json::from_value(saved["detector"].clone()).unwrap();
    let mut completions = Vec::new();
    for mut detector in [original_detector, restored_detector] {
        let mut end =
            TelemetryEvent::empty(Uuid::from_u128(7001), at() + ChronoDuration::minutes(2));
        end.charger_state = Some(ChargerState::Done);
        end.battery_level = Some(32.0);
        let ChargeEvent::SessionEnded(completion) = detector.process(&end) else {
            panic!("continued active charge")
        };
        completions.push(serde_json::to_value(completion).unwrap());
    }
    assert_eq!(
        completions[0], completions[1],
        "continued charge exact float-bit payload"
    );
    stages.push(json!({"stage":"charge-continuity","completions":completions}));
    let actual = json!(stages);
    evidence("durable", &actual);
    app.close().await;
    health.abort();
    let _ = health.await;
    fixture.cleanup().await;
    let mut expected =
        serde_json::from_str::<Value>(include_str!("restore_execution/durable.json")).unwrap();
    for (index, stage) in [(3, "restart-0"), (4, "restart-1")] {
        assert_eq!(expected[index]["stage"], stage);
        let charge = &mut expected[index]["state"]["panda.charge_sessions"][1];
        assert_eq!(charge["id"], Uuid::from_u128(7014).to_string());
        assert_eq!(charge["peak_voltage"], json!(12.12345679012364));
        assert_eq!(charge["peak_voltage"], charge["max_charge_rate_kw"]);
        // The frozen baseline misfiled detector power as voltage.
        charge["peak_voltage"] = Value::Null;
    }
    assert_eq!(actual, expected);
}

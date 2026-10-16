//! Uses only an explicitly selected disposable database with real migrations.
use super::*;
use sqlx::{postgres::PgConnectOptions, Executor};

fn at() -> DateTime<Utc> {
    Utc.timestamp_millis_opt(1_800_000_000_000).unwrap()
}

struct Fixture {
    admin: PgPool,
    pool: PgPool,
    name: String,
    role: String,
}
impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("PARALLAX_TEST_DATABASE_URL")
            .expect("explicit disposable Parallax database required");
        let options: PgConnectOptions = url.parse().unwrap();
        let admin = PgPool::connect_with(options.clone().database("postgres"))
            .await
            .unwrap();
        let name = format!("parallax_test_{}", Uuid::new_v4().simple());
        let role = format!("{name}_app");
        admin
            .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .await
            .unwrap();
        let migration = PgPool::connect_with(options.clone().database(&name))
            .await
            .unwrap();
        crate::db::migrations::run_current_migrations(&migration)
            .await
            .unwrap();
        migration.execute(sqlx::AssertSqlSafe(format!("CREATE ROLE \"{role}\" LOGIN PASSWORD 'synthetic_only' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"))).await.unwrap();
        migration.execute(sqlx::AssertSqlSafe(format!("GRANT USAGE ON SCHEMA panda,timeseries TO \"{role}\"; GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA panda,timeseries TO \"{role}\"; GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA panda,timeseries TO \"{role}\""))).await.unwrap();
        migration.close().await;
        let pool = crate::db::pool::application_pool_options()
            .connect_with(
                options
                    .database(&name)
                    .username(&role)
                    .password("synthetic_only"),
            )
            .await
            .unwrap();
        let privileges:(bool,bool)=sqlx::query_as("SELECT has_table_privilege(current_user,'panda.charge_sessions','DELETE'),has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE')").fetch_one(&pool).await.unwrap();
        assert_eq!(privileges, (false, false));
        sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,'parallax@example.invalid','synthetic-only')").bind(Uuid::from_u128(1)).execute(&pool).await.unwrap();
        Self {
            admin,
            pool,
            name,
            role,
        }
    }
    async fn seed(&self, index: u128) -> (Uuid, Uuid) {
        let vehicle = Uuid::from_u128(1000 + index);
        let session = Uuid::from_u128(3000 + index);
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,$3,'synthetic')").bind(vehicle).bind(Uuid::from_u128(1)).bind(format!("synthetic-{index}")).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at) VALUES($1,$2,$3)")
            .bind(session)
            .bind(vehicle)
            .bind(at() - chrono::Duration::minutes(1))
            .execute(&self.pool)
            .await
            .unwrap();
        set_collector_state(&self.pool, vehicle, "connected", None)
            .await
            .unwrap();
        (vehicle, session)
    }
    async fn close(self) {
        self.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE \"{}\" WITH (FORCE)",
                self.name
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE \"{}\"", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
    }
}

fn cases() -> Vec<(String, String, Vec<u8>)> {
    let mut cases = Vec::new();
    let mut add = |name: String, topic: &str, wire: Vec<u8>| cases.push((name, topic.into(), wire));
    for topic in VEHICLE_STATE_TOPICS {
        add(format!("{topic}/empty"), topic, vec![]);
        add(format!("{topic}/malformed"), topic, vec![0x0a, 0xff]);
    }
    // Unknown protobuf identity tags must not become durable values.
    #[derive(Clone, PartialEq, ProstMessage)]
    struct PrivateWifi {
        #[prost(int32, optional, tag = "1")]
        status: Option<i32>,
        #[prost(string, tag = "3")]
        ssid: String,
        #[prost(int32, optional, tag = "8")]
        rssi: Option<i32>,
    }
    #[derive(Clone, PartialEq, ProstMessage)]
    struct PrivateNetwork {
        #[prost(message, optional, tag = "4")]
        wifi: Option<PrivateWifi>,
    }
    add(
        "network/private-identifiers".into(),
        "vehicle.network.state",
        PrivateNetwork {
            wifi: Some(PrivateWifi {
                status: Some(2),
                ssid: "synthetic-private-network".into(),
                rssi: Some(-55),
            }),
        }
        .encode_to_vec(),
    );
    for signal in [-255, -150, 0, 1] {
        add(
            format!("network/signal/{signal}"),
            "vehicle.network.state",
            NetworkState {
                overall_state: Some(1),
                active_transport: Some(4),
                wifi: Some(WifiState {
                    status: Some(2),
                    rssi_dbm: Some(signal),
                    link_speed_mbps: Some(54),
                    frequency_mhz: Some(2412),
                    channel_width_mhz: Some(20),
                    ..Default::default()
                }),
                cellular: Some(CellularState {
                    access_technology: Some("technology-too-long".into()),
                    signal_dbm: Some(signal),
                }),
            }
            .encode_to_vec(),
        );
    }
    add(
        "efficiency/duplicate-and-missing-modes".into(),
        "dynamics.vehicle.efficiency",
        EfficiencyState {
            reference_wh_per_km: Some(207),
            learned_wh_per_km: Some(248),
            mode_ranges: vec![
                ModeRange {
                    mode: Some(2),
                    full_charge_range_km: Some(531),
                },
                ModeRange {
                    mode: Some(1),
                    full_charge_range_km: Some(400),
                },
                ModeRange {
                    mode: Some(2),
                    full_charge_range_km: Some(500),
                },
                ModeRange {
                    mode: None,
                    full_charge_range_km: Some(300),
                },
            ],
        }
        .encode_to_vec(),
    );
    for mass in [999, 1000, 3160, 10000, 10001] {
        add(
            format!("mass/{mass}"),
            "dynamics.vehicle.mass_estimate",
            MassEstimate {
                estimated_mass_kg: Some(mass),
            }
            .encode_to_vec(),
        );
    }
    add(
        "parked/all-windows-and-nonfinite".into(),
        "energy_edge_compute.graphs.parked_energy_distributions",
        ParkedEnergyDistributions {
            hours_24: Some(ParkedEnergyWindow {
                total_kwh: Some(1.5),
                vehicle_systems_kwh: Some(0.25),
                outlets_kwh: Some(0.125),
                climate_kwh: Some(0.5),
                gear_guard_kwh: Some(0.625),
                total_range_impact_km: Some(6.0),
                vehicle_systems_range_impact_km: Some(1.0),
                outlets_range_impact_km: Some(0.5),
                climate_range_impact_km: Some(2.0),
                gear_guard_range_impact_km: Some(2.5),
                duration_minutes: Some(1440),
            }),
            hours_8: Some(ParkedEnergyWindow {
                total_kwh: Some(f32::NAN),
                duration_minutes: Some(-1),
                ..Default::default()
            }),
            since_parked: Some(ParkedEnergyWindow::default()),
        }
        .encode_to_vec(),
    );
    for (name, total, power, fallback, state) in [
        ("valid", Some(2.5), Some(70.5), Some(90), Some(1)),
        ("fallback", None, None, Some(80), None),
        (
            "invalid-primary",
            Some(f32::NAN),
            Some(f32::NAN),
            Some(80),
            None,
        ),
        ("status-only", None, None, None, Some(99)),
        ("zero", Some(0.0), Some(0.0), None, None),
    ] {
        add(
            format!("breakdown/{name}"),
            "energy_edge_compute.graphs.charge_session_breakdown",
            ChargeBreakdown {
                total_kwh: total,
                current_power_kw: power,
                fallback_power_kw: fallback,
                charging_state: state,
            }
            .encode_to_vec(),
        );
    }
    for pack in [f64::NAN, -0.1, 0.0, 82.5, 500.0, 500.1] {
        add(
            format!("pack/{pack}"),
            "energy.high_voltage.battery_state",
            HvBatteryState {
                charge_state: Some(HvChargeState {
                    soc: Some(50.0),
                    pack_energy_kwh: Some(pack),
                    range_km: Some(200.0),
                }),
            }
            .encode_to_vec(),
        );
    }
    let segment = |seconds, soc, power| ChargingGraphSegment {
        soc,
        power_kw: power,
        start_unix_ms: Some((at() + chrono::Duration::seconds(seconds)).timestamp_millis()),
        end_unix_ms: None,
        state: None,
    };
    add(
        "curve/order-tie-skip-association".into(),
        "energy_edge_compute.graphs.charging_graph_global",
        ChargingGraphGlobal {
            segments: vec![
                segment(-100, Some(20), Some(50.0)),
                segment(10, Some(30), Some(60.0)),
                segment(0, Some(25), Some(55.0)),
                segment(10, None, Some(65.0)),
                segment(20, Some(101), Some(f32::NAN)),
                ChargingGraphSegment {
                    soc: Some(40),
                    power_kw: Some(20.0),
                    start_unix_ms: None,
                    ..Default::default()
                },
            ],
        }
        .encode_to_vec(),
    );
    for seconds in [-1, 0, 1, 59, 60, 61, 172800, 172801] {
        add(
            format!("estimate/{seconds}"),
            "charging.session.time_estimation",
            ChargingTimeEstimation {
                remaining_seconds: Some(seconds),
            }
            .encode_to_vec(),
        );
    }
    add(
        "status/sparse".into(),
        "charging.session.status",
        ChargingStatus {
            plug_connection_status: Some(2),
            display_status: None,
            evse_type: Some(99),
        }
        .encode_to_vec(),
    );
    add(
        "cold/nonfinite".into(),
        "energy_edge_compute.graphs.cold_weather_soc",
        ColdWeatherSoc {
            available_soc_pct: Some(80),
            cold_limited_soc_pct: Some(5),
            cold_range_impact_km: Some(f32::INFINITY),
        }
        .encode_to_vec(),
    );
    add(
        "power/valid".into(),
        "vehicle.power.state",
        PowerStateMessage { state: Some(4) }.encode_to_vec(),
    );
    add(
        "gear/unknown".into(),
        "dynamics.vehicle.gear",
        GearState { gear: Some(99) }.encode_to_vec(),
    );
    cases
}

fn assert_database_clock(value: &Value) {
    let actual = DateTime::parse_from_rfc3339(value.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    assert!(
        (Utc::now() - actual).num_seconds().abs() < 60,
        "unexpected database clock {value}"
    );
}

async fn rows(pool: &PgPool, vehicle: Uuid) -> Value {
    let mut result = serde_json::Map::new();
    for table in [
        "panda.parallax_collector_state",
        "panda.charge_sessions",
        "timeseries.parallax_network_samples",
        "timeseries.parallax_efficiency_samples",
        "timeseries.parallax_mass_samples",
        "timeseries.parallax_parked_energy_samples",
        "timeseries.parallax_charge_breakdown_samples",
        "timeseries.parallax_charge_curve_points",
        "timeseries.parallax_cold_weather_samples",
    ] {
        let mut rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM {table} t WHERE vehicle_id=$1 ORDER BY to_jsonb(t)::text"
        )))
        .bind(vehicle)
        .fetch_all(pool)
        .await
        .unwrap();
        for row in &mut rows {
            for (key, value) in row.as_object_mut().unwrap() {
                if matches!(key.as_str(), "created_at" | "updated_at" | "connected_at")
                    && !value.is_null()
                {
                    assert_database_clock(value);
                    *value = json!("database-clock");
                }
                if matches!(key.as_str(), "last_frame_at")
                    && value.as_str().is_some_and(|v| !v.starts_with("2027-01-15"))
                    && !value.is_null()
                {
                    assert_database_clock(value);
                    *value = json!("database-clock");
                }
                if key == "id" && table != "panda.charge_sessions" {
                    *value = json!("generated-row-id");
                }
            }
        }
        rows.sort_by_key(Value::to_string);
        result.insert(table.into(), json!(rows));
    }
    json!(result)
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB; run with --ignored"]
async fn full_persistence_and_forwarding_contract() {
    let db = Fixture::new().await;
    let mut output = vec![];
    for (index, (name, topic, payload)) in cases().into_iter().enumerate() {
        let (vehicle, session) = db.seed(index as u128).await;
        let context = crate::ingestion::worker::ActiveSessionContext {
            session_id: Some(session),
            started_at: Some(at() - chrono::Duration::minutes(1)),
            ..Default::default()
        };
        let (tx, mut rx) = mpsc::channel(4);
        let mut observations = vec![];
        for _ in 0..2 {
            let mut report = EnvelopeReport::new();
            let outcome = persist_allowlisted_envelope(
                &db.pool,
                vehicle,
                &topic,
                &payload,
                at(),
                at(),
                &context,
                Some(&tx),
                &mut report,
                true,
            )
            .await;
            let sent = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
            observations.push(json!({"outcome":outcome.map(|v|v.as_str()).map_err(|e|e.to_string()),"forward":report.forward_outcome.as_str(),"decoded":report.decoded,"notes":report.notes,"sent":sent,"rows":rows(&db.pool,vehicle).await}));
        }
        output.push(json!({"name":name,"topic":topic,"payload":hex::encode(payload),"observations":observations}));
    }
    let (vehicle, session) = db.seed(1000).await;
    let context = crate::ingestion::worker::ActiveSessionContext {
        session_id: Some(session),
        started_at: Some(at()),
        ..Default::default()
    };
    let payload = PowerStateMessage { state: Some(3) }.encode_to_vec();
    for mode in [
        "not-attached",
        "enqueued",
        "full",
        "closed",
        "stale-old",
        "stale-future",
        "old-boundary",
        "future-boundary",
    ] {
        let (tx, mut rx) = mpsc::channel(1);
        if mode == "full" {
            tx.try_send(("occupied".into(), TelemetryEvent::empty(vehicle, at())))
                .unwrap();
        }
        if mode == "closed" {
            rx.close();
        }
        let offset = match mode {
            "stale-old" => -301,
            "stale-future" => 31,
            "old-boundary" => -300,
            "future-boundary" => 30,
            _ => 0,
        };
        let mut report = EnvelopeReport::new();
        let result = persist_allowlisted_envelope(
            &db.pool,
            vehicle,
            "vehicle.power.state",
            &payload,
            at(),
            at() + chrono::Duration::seconds(offset),
            &context,
            if mode == "not-attached" {
                None
            } else {
                Some(&tx)
            },
            &mut report,
            true,
        )
        .await;
        let sent = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        output.push(json!({"mode":mode,"outcome":result.map(|v|v.as_str()).map_err(|e|e.to_string()),"forward":report.forward_outcome.as_str(),"decoded":report.decoded,"notes":report.notes,"sent":sent,"rows":rows(&db.pool,vehicle).await}));
    }
    db.close().await;
    contract_tests::assert_snapshot(
        "persistence",
        &json!(output),
        include_str!("fixtures/persistence.json"),
    );
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB; run with --ignored"]
async fn lease_heartbeat_and_history_protection_contract() {
    let db = Fixture::new().await;
    let (vehicle, session) = db.seed(2000).await;
    let owner = Uuid::new_v4();
    assert!(!acquire_in_process_lease(&db.pool, vehicle, owner)
        .await
        .unwrap());
    set_collector_state(&db.pool, vehicle, "disconnected", None)
        .await
        .unwrap();
    assert!(acquire_in_process_lease(&db.pool, vehicle, owner)
        .await
        .unwrap());
    assert!(!acquire_in_process_lease(&db.pool, vehicle, Uuid::new_v4())
        .await
        .unwrap());
    set_collector_state(&db.pool, vehicle, "connected", None)
        .await
        .unwrap();
    touch_collector_heartbeat(&db.pool, vehicle).await.unwrap();
    release_in_process_lease(&db.pool, vehicle, Uuid::new_v4())
        .await
        .unwrap();
    touch_collector_heartbeat(&db.pool, vehicle).await.unwrap();
    release_in_process_lease(&db.pool, vehicle, owner)
        .await
        .unwrap();
    assert_eq!(
        touch_collector_heartbeat(&db.pool, vehicle)
            .await
            .unwrap_err()
            .to_string(),
        "Parallax collector heartbeat state is missing or disconnected"
    );
    assert_eq!(
        update_parallax_power(&db.pool, vehicle, session, 60.0, at())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        update_parallax_power(
            &db.pool,
            vehicle,
            session,
            50.0,
            at() - chrono::Duration::seconds(1)
        )
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        update_parallax_power(&db.pool, Uuid::new_v4(), session, 90.0, at())
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE panda.charge_sessions SET ended_at=$2 WHERE id=$1")
        .bind(session)
        .bind(at())
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        update_parallax_power(&db.pool, vehicle, session, 80.0, at())
            .await
            .unwrap(),
        0
    );
    assert!(sqlx::query("DELETE FROM panda.charge_sessions WHERE id=$1")
        .bind(session)
        .execute(&db.pool)
        .await
        .is_err());
    let power: Option<f64> =
        sqlx::query_scalar("SELECT parallax_live_power_kw FROM panda.charge_sessions WHERE id=$1")
            .bind(session)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(power, Some(60.0));
    db.close().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB; run with --ignored"]
async fn companion_shutdown_during_lease_and_token_retry() {
    let db = Fixture::new().await;
    let (vehicle, _) = db.seed(2001).await;
    let (tx, _rx) = mpsc::channel(1);
    let (_context_tx, context_rx) =
        watch::channel(crate::ingestion::worker::ActiveSessionContext::default());
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let task = spawn_in_process(
        db.pool.clone(),
        vehicle,
        "synthetic".into(),
        "unused-no-credentials".into(),
        tx.clone(),
        context_rx.clone(),
        shutdown_rx,
    );
    tokio::task::yield_now().await;
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM panda.parallax_collector_state WHERE vehicle_id=$1")
            .bind(vehicle)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(status, "connected");
    set_collector_state(&db.pool, vehicle, "disconnected", None)
        .await
        .unwrap();
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let task = spawn_in_process(
        db.pool.clone(),
        vehicle,
        "synthetic".into(),
        "unused-no-credentials".into(),
        tx,
        context_rx,
        shutdown_rx,
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status: String = sqlx::query_scalar(
                "SELECT status FROM panda.parallax_collector_state WHERE vehicle_id=$1",
            )
            .bind(vehicle)
            .fetch_one(&db.pool)
            .await
            .unwrap();
            if status == "error" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let state:(String,Option<Uuid>,Option<String>)=sqlx::query_as("SELECT status,owner_instance_id,last_error FROM panda.parallax_collector_state WHERE vehicle_id=$1").bind(vehicle).fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        state,
        ("disconnected".into(), None, Some("shutdown".into()))
    );
    db.close().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB; run with --ignored"]
async fn scripted_socket_persists_and_classifies_closes() {
    use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
    let db = Fixture::new().await;
    let (vehicle, _) = db.seed(2002).await;
    let session = CollectorSession {
        vehicle_id: vehicle,
        rivian_vehicle_id: "synthetic".into(),
        tokens: RivianTokenBundle {
            access_token: String::new(),
            refresh_token: String::new(),
            app_session_token: String::new(),
            user_session_token: String::new(),
            csrf_token: String::new(),
            created_at: at(),
        },
    };
    let (_context_tx, mut context) =
        watch::channel(crate::ingestion::worker::ActiveSessionContext::default());
    let envelope = |value: Value| {
        Ok(Message::Text(json!({"id":SUBSCRIPTION_ID,"type":"next","payload":{"data":{"parallaxMessages":value}}}).to_string().into()))
    };
    let mut socket=contract_tests::ScriptSocket{incoming:vec![
        Ok(Message::Ping(vec![1,2].into())),
        envelope(json!({"rvm":"unknown.synthetic","payload":"not-base64"})),
        envelope(json!({"rvm":"vehicle.power.state","payload":"!"})),
        envelope(json!({"rvm":"vehicle.network.state","payload":"","timestamp":at().timestamp_millis()})),
        envelope(json!({"rvm":"vehicle.power.state","payload":"Cv8="})),
        Ok(Message::Close(Some(CloseFrame{code:CloseCode::Library(4420),reason:"Connection TTL expired".into()}))),
    ].into(),outgoing:vec![],pending:false};
    drive_subscribed_socket(&db.pool, &session, &mut context, None, &mut socket)
        .await
        .unwrap();
    assert_eq!(socket.outgoing, vec![Message::Pong(vec![1, 2].into())]);
    let state:(String,i64,i64)=sqlx::query_as("SELECT status,decode_error_count::bigint,empty_frame_count::bigint FROM panda.parallax_collector_state WHERE vehicle_id=$1").bind(vehicle).fetch_one(&db.pool).await.unwrap();
    assert_eq!(state, ("disconnected".into(), 2, 0));
    let samples:i64=sqlx::query_scalar("SELECT count(*) FROM timeseries.parallax_network_samples WHERE vehicle_id=$1 AND source_at=$2 AND schema_version=1").bind(vehicle).bind(at()).fetch_one(&db.pool).await.unwrap();
    assert_eq!(samples, 1);
    set_collector_state(&db.pool, vehicle, "connected", None)
        .await
        .unwrap();
    let close = Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "synthetic close".into(),
    });
    let expected = format!("Parallax socket closed: {close:?}");
    let mut socket = contract_tests::ScriptSocket {
        incoming: vec![Ok(Message::Close(close))].into(),
        outgoing: vec![],
        pending: false,
    };
    assert_eq!(
        drive_subscribed_socket(&db.pool, &session, &mut context, None, &mut socket)
            .await
            .unwrap_err()
            .to_string(),
        expected
    );
    db.close().await;
}

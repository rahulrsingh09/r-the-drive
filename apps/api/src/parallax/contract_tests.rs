//! Fixed synthetic wire contracts captured before the companion replacement.
use super::*;
use futures::{Sink, Stream};
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio_tungstenite::tungstenite::Error as SocketError;

fn at() -> DateTime<Utc> {
    Utc.timestamp_millis_opt(1_800_000_000_000).unwrap()
}
fn vehicle() -> Uuid {
    Uuid::from_u128(100)
}

pub(super) fn assert_snapshot(name: &str, actual: &Value, expected: &str) {
    if let Ok(directory) = std::env::var("PARALLAX_BASELINE_OUTPUT") {
        std::fs::write(
            format!("{directory}/{name}.json"),
            serde_json::to_string_pretty(actual).unwrap(),
        )
        .unwrap();
    }
    assert!(serde_json::to_string_pretty(actual).unwrap() == expected.trim_end(),
        "full {name} contract changed; export actual with PARALLAX_BASELINE_OUTPUT to inspect the complete difference");
}

fn canonical_cases() -> Vec<(String, String, Vec<u8>)> {
    let mut cases = Vec::new();
    let mut add =
        |name: String, topic: &str, bytes: Vec<u8>| cases.push((name, topic.into(), bytes));
    for topic in VEHICLE_STATE_TOPICS
        .iter()
        .copied()
        .chain(["unknown.vehicle.topic"])
    {
        for (name, bytes) in [
            ("empty", vec![]),
            ("unknown-tag", vec![0xa0, 0x06, 1]),
            ("malformed", vec![0x0a, 0xff]),
            ("zero-tag", vec![0]),
        ] {
            add(format!("{topic}/{name}"), topic, bytes);
        }
    }
    for state in -1..=6 {
        add(
            format!("power/{state}"),
            "vehicle.power.state",
            PowerStateMessage { state: Some(state) }.encode_to_vec(),
        );
    }
    for gear in -1..=6 {
        add(
            format!("gear/{gear}"),
            "dynamics.vehicle.gear",
            GearState { gear: Some(gear) }.encode_to_vec(),
        );
    }
    for (name, lat, lon, alt) in [
        ("ordinary", Some(12.5), Some(-32.0), Some(100.0)),
        ("no-altitude", Some(0.0), Some(0.0), None),
        ("lat-min", Some(-90.0), Some(-180.0), Some(-1000.0)),
        ("max", Some(90.0), Some(180.0), Some(20000.0)),
        ("missing-lat", None, Some(1.0), None),
        ("missing-lon", Some(1.0), None, None),
        ("invalid-lat", Some(90.01), Some(1.0), None),
        ("invalid-lon", Some(1.0), Some(-180.01), None),
        ("invalid-alt", Some(1.0), Some(1.0), Some(20000.01)),
        ("nan", Some(f64::NAN), Some(1.0), None),
        ("infinite", Some(1.0), Some(1.0), Some(f64::INFINITY)),
    ] {
        add(
            format!("gnss/{name}"),
            "dynamics.vehicle.gnss",
            GnssState {
                latitude: lat,
                longitude: lon,
                altitude_m: alt,
            }
            .encode_to_vec(),
        );
    }
    for kilometers in [0, 1, 161, 2_000_000, 2_000_001, u64::MAX] {
        add(
            format!("odometer/{kilometers}"),
            "dynamics.vehicle.odometer",
            OdometerState {
                kilometers: Some(kilometers),
            }
            .encode_to_vec(),
        );
    }
    for position in [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(7),
        Some(8),
        Some(9),
        Some(10),
        Some(11),
        Some(12),
        Some(13),
        Some(14),
        Some(15),
        Some(16),
        Some(99),
        Some(10000),
    ] {
        for state in [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
        ] {
            add(
                format!("closure/{position:?}/{state:?}"),
                "body.closures.states",
                ClosureStates {
                    states: vec![ClosureState { position, state }],
                }
                .encode_to_vec(),
            );
        }
    }
    add(
        "closure/duplicate-ordered".into(),
        "body.closures.states",
        ClosureStates {
            states: vec![
                ClosureState {
                    position: Some(1),
                    state: Some(4),
                },
                ClosureState {
                    position: Some(99),
                    state: Some(7),
                },
                ClosureState {
                    position: Some(1),
                    state: Some(2),
                },
                ClosureState {
                    position: Some(2),
                    state: Some(8),
                },
            ],
        }
        .encode_to_vec(),
    );
    for position in [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(7),
        Some(8),
        Some(9),
        Some(14),
    ] {
        for state in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
            add(
                format!("lock/{position:?}/{state:?}"),
                "body.locks.states",
                LockStates {
                    states: vec![LockState { position, state }],
                }
                .encode_to_vec(),
            );
        }
    }
    for position in [None, Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)] {
        for status in [None, Some(0), Some(1), Some(2), Some(3)] {
            add(
                format!("tire/{position:?}/{status:?}"),
                "dynamics.tires.state",
                TireStates {
                    states: vec![TireState {
                        position,
                        status,
                        pressure_bar: Some(2.5),
                    }],
                }
                .encode_to_vec(),
            );
        }
    }
    for pressure in [
        None,
        Some(-0.1),
        Some(0.0),
        Some(10.0),
        Some(10.01),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        add(
            format!("pressure/{pressure:?}"),
            "dynamics.tires.state",
            TireStates {
                states: vec![TireState {
                    position: Some(1),
                    status: Some(1),
                    pressure_bar: pressure,
                }],
            }
            .encode_to_vec(),
        );
    }
    for cabin in [
        None,
        Some(-80.0),
        Some(100.0),
        Some(100.1),
        Some(f32::NAN),
        Some(f32::INFINITY),
    ] {
        add(
            format!("temperature/{cabin:?}"),
            "comfort.cabin.cabin_temperatures",
            CabinTemperatures {
                cabin_c: cabin,
                driver_c: Some(21.5),
            }
            .encode_to_vec(),
        );
    }
    for status in -1..=6 {
        add(
            format!("precondition/{status}"),
            "comfort.cabin.cabin_preconditioning_status",
            PreconditioningState {
                status: Some(status),
            }
            .encode_to_vec(),
        );
        add(
            format!("defrost/{status}"),
            "comfort.cabin.defrost_defog_status",
            DefrostState {
                status: Some(status),
            }
            .encode_to_vec(),
        );
    }
    cases
}

#[test]
fn full_canonical_wire_contract() {
    let rows: Vec<Value> = canonical_cases().into_iter().map(|(name,topic,payload)| {
        let mut notes = vec![];
        let result = decode_vehicle_telemetry_with_notes(&topic,&payload,at(),vehicle(),&mut notes);
        let output = match result { Ok(event)=>json!({"event":event}),Err(error)=>json!({"error":error.to_string()}) };
        json!({"name":name,"topic":topic,"wire_hex":hex::encode(&payload),"output":output,"notes":notes,"body_entries":body_entries(&topic,&payload),"capture_payload_allowed":capture_payload_allowed(&topic)})
    }).collect();
    assert_snapshot(
        "canonical",
        &json!(rows),
        include_str!("fixtures/canonical.json"),
    );
}

#[test]
fn subscription_timestamp_and_boundary_contract() {
    let timestamps: Vec<Value> = [
        Value::Null,
        json!("bad"),
        json!("2026-10-16T12:34:56.789+02:00"),
        json!("1800000000000"),
        json!(1800000000000i64),
        json!(1800000000),
        json!(-1),
        json!(1.5),
        json!(i64::MAX),
    ]
    .into_iter()
    .map(|input| json!({"input":input,"output":parse_source_at(Some(&input))}))
    .collect();
    let sessions: Vec<Value> = [-1,0,1,59,60,61].into_iter().map(|seconds| {
        let context=crate::ingestion::worker::ActiveSessionContext{session_id:Some(Uuid::from_u128(200)),started_at:Some(at()),ended_at:Some(at()+chrono::Duration::seconds(60)),..Default::default()};
        json!({"seconds":seconds,"session":matching_active_session(&context,at()+chrono::Duration::seconds(seconds))})
    }).collect();
    assert_snapshot(
        "boundaries",
        &json!({"subscription":subscription_message("synthetic-vehicle"),"timestamps":timestamps,"sessions":sessions,"schema_version":SCHEMA_VERSION,"signal":[without_signal_sentinel(Some(-151)),without_signal_sentinel(Some(-150)),without_signal_sentinel(Some(0)),without_signal_sentinel(Some(1))]}),
        include_str!("fixtures/boundaries.json"),
    );
}

pub(super) struct ScriptSocket {
    pub(super) incoming: VecDeque<Result<Message, SocketError>>,
    pub(super) outgoing: Vec<Message>,
    pub(super) pending: bool,
}
impl Stream for ScriptSocket {
    type Item = Result<Message, SocketError>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        if let Some(item) = self.incoming.pop_front() {
            Poll::Ready(Some(item))
        } else if self.pending {
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}
impl Sink<Message> for ScriptSocket {
    type Error = SocketError;
    fn poll_ready(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.outgoing.push(item);
        Ok(())
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test(start_paused = true)]
async fn scripted_acknowledgement_contract() {
    let mut output = vec![];
    for (name, incoming, pending) in [
        (
            "ack",
            vec![Ok(Message::Text(r#"{"type":"connection_ack"}"#.into()))],
            false,
        ),
        (
            "ping-before-ack",
            vec![
                Ok(Message::Ping(vec![1, 2, 3].into())),
                Ok(Message::Text(r#"{"type":"connection_ack"}"#.into())),
            ],
            false,
        ),
        ("malformed-json", vec![Ok(Message::Text("{".into()))], false),
        (
            "wrong-type-eof",
            vec![Ok(Message::Text(r#"{"type":"next"}"#.into()))],
            false,
        ),
        ("eof", vec![], false),
        ("timeout", vec![], true),
        (
            "transport-error",
            vec![Err(SocketError::ConnectionClosed)],
            false,
        ),
    ] {
        let mut socket = ScriptSocket {
            incoming: incoming.into(),
            outgoing: vec![],
            pending,
        };
        let result = wait_for_ack(&mut socket).await;
        output.push(json!({"name":name,"error":result.err().map(|e|e.to_string()),"sent":socket.outgoing.iter().map(|message|format!("{message:?}")).collect::<Vec<_>>()}));
    }
    assert_snapshot(
        "acknowledgement",
        &json!(output),
        include_str!("fixtures/acknowledgement.json"),
    );
}

#[tokio::test]
async fn envelope_errors_precede_database_access() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let context = crate::ingestion::worker::ActiveSessionContext::default();
    let mut output = vec![];
    for envelope in [
        Value::Null,
        json!({}),
        json!({"rvm":1}),
        json!({"rvm":"unknown","payload":"!"}),
        json!({"rvm":"vehicle.power.state"}),
        json!({"rvm":"vehicle.power.state","payload":"!"}),
        json!({"rvm":"vehicle.power.state","payload":"Cv8="}),
    ] {
        let result = persist_envelope(&pool, vehicle(), &envelope, &context, None).await;
        output.push(json!({"input":envelope,"error":result.err().map(|e|e.to_string())}));
    }
    assert_snapshot(
        "envelope-errors",
        &json!(output),
        include_str!("fixtures/envelope-errors.json"),
    );
}

#[test]
fn read_only_handshake_and_header_errors() {
    let mut tokens = RivianTokenBundle {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        app_session_token: "synthetic-app".into(),
        user_session_token: "synthetic-user".into(),
        csrf_token: "synthetic-csrf".into(),
        created_at: at(),
    };
    let request = transport::connection_request(&tokens).unwrap();
    assert_eq!(request.method(), http::Method::GET);
    assert_eq!(request.uri().to_string(), WS_URL);
    let mut headers = request
        .headers()
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_str().unwrap().to_string()))
        .collect::<BTreeMap<_, _>>();
    let socket_key = headers.remove("sec-websocket-key").unwrap();
    assert_eq!(BASE64.decode(socket_key).unwrap().len(), 16);
    assert_eq!(
        json!(headers),
        json!({"a-sess":"synthetic-app","u-sess":"synthetic-user","csrf-token":"synthetic-csrf","sec-websocket-protocol":"graphql-transport-ws","host":"api.rivian.com","connection":"Upgrade","upgrade":"websocket","sec-websocket-version":"13"})
    );
    let Message::Text(init) = transport::connection_init(&tokens, Uuid::from_u128(1)) else {
        panic!("text init")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&init).unwrap(),
        json!({"type":"connection_init","payload":{"client-name":"com.rivian.ios.consumer-apollo-ios","client-version":"1.13.0-1494","dc-cid":"m-ios-00000000-0000-0000-0000-000000000001","u-sess":"synthetic-user"}})
    );
    tokens.csrf_token.clear();
    assert!(transport::connection_request(&tokens)
        .unwrap()
        .headers()
        .get("csrf-token")
        .is_none());
    tokens.app_session_token = "bad\nheader".into();
    tokens.user_session_token = "bad\nheader".into();
    assert_eq!(
        transport::connection_request(&tokens)
            .unwrap_err()
            .to_string(),
        "invalid Rivian app session header"
    );
    tokens.app_session_token = "valid".into();
    assert_eq!(
        transport::connection_request(&tokens)
            .unwrap_err()
            .to_string(),
        "invalid Rivian user session header"
    );
    tokens.user_session_token = "valid".into();
    tokens.csrf_token = "bad\nheader".into();
    assert_eq!(
        transport::connection_request(&tokens)
            .unwrap_err()
            .to_string(),
        "invalid Rivian CSRF header"
    );
}

#[tokio::test(start_paused = true)]
async fn retry_delays_cap_reset_and_cancel_without_advancing_time() {
    let mut schedule = RetrySchedule::default();
    let mut delays = vec![];
    for _ in 0..10 {
        delays.push(schedule.delay().as_secs());
        schedule.advance();
    }
    assert_eq!(delays, vec![2, 4, 8, 16, 32, 64, 120, 120, 120, 120]);
    schedule.reset();
    assert_eq!(schedule.delay(), Duration::from_secs(2));
    let (tx, mut rx) = broadcast::channel(1);
    let started = tokio::time::Instant::now();
    assert!(transport::wait_to_retry(&mut rx, schedule.delay()).await);
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    tx.send(()).unwrap();
    let started = tokio::time::Instant::now();
    assert!(!transport::wait_to_retry(&mut rx, schedule.delay()).await);
    assert_eq!(started.elapsed(), Duration::ZERO);
    drop(tx);
    assert!(!transport::wait_to_retry(&mut rx, schedule.delay()).await);
}

#[test]
fn socket_actions_gate_subscription_before_envelope() {
    for frame in [
        json!(null),
        json!({}),
        json!({"id":"other","type":"error"}),
        json!({"id":SUBSCRIPTION_ID,"type":"next","payload":{}}),
        json!({"id":SUBSCRIPTION_ID,"type":"ping"}),
    ] {
        assert!(matches!(
            SocketAction::classify(Message::Text(frame.to_string().into())),
            SocketAction::Ignore
        ));
    }
    assert!(matches!(
        SocketAction::classify(Message::Text("{".into())),
        SocketAction::Ignore
    ));
    let payload = json!({"rvm":"vehicle.power.state","payload":"CAQ="});
    let frame =
        json!({"id":SUBSCRIPTION_ID,"type":"next","payload":{"data":{"parallaxMessages":payload}}});
    let SocketAction::Envelope(actual) =
        SocketAction::classify(Message::Text(frame.to_string().into()))
    else {
        panic!("expected envelope")
    };
    assert_eq!(actual, payload);
    for (kind, rejected) in [("error", true), ("complete", false)] {
        let action = SocketAction::classify(Message::Text(
            json!({"id":SUBSCRIPTION_ID,"type":kind}).to_string().into(),
        ));
        assert!(if rejected {
            matches!(action, SocketAction::Rejected)
        } else {
            matches!(action, SocketAction::Completed)
        });
    }
    let SocketAction::Reply(reply) = SocketAction::classify(Message::Ping(vec![9, 8, 7].into()))
    else {
        panic!("pong")
    };
    assert_eq!(reply, Message::Pong(vec![9, 8, 7].into()));
}

#[tokio::test]
async fn subscribed_driver_handles_transport_without_network() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let session = CollectorSession {
        vehicle_id: vehicle(),
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
    let (_tx, mut context) =
        watch::channel(crate::ingestion::worker::ActiveSessionContext::default());
    for (kind, error) in [
        ("error", "Parallax subscription rejected"),
        ("complete", "Parallax subscription completed"),
    ] {
        let mut socket = ScriptSocket {
            incoming: vec![
                Ok(Message::Text("{".into())),
                Ok(Message::Ping(vec![4, 5].into())),
                Ok(Message::Text(
                    json!({"id":"other","type":"error"}).to_string().into(),
                )),
                Ok(Message::Text(
                    json!({"id":SUBSCRIPTION_ID,"type":kind}).to_string().into(),
                )),
            ]
            .into(),
            outgoing: vec![],
            pending: false,
        };
        assert_eq!(
            drive_subscribed_socket(&pool, &session, &mut context, None, &mut socket)
                .await
                .unwrap_err()
                .to_string(),
            error
        );
        assert_eq!(socket.outgoing, vec![Message::Pong(vec![4, 5].into())]);
    }
    let mut socket = ScriptSocket {
        incoming: VecDeque::new(),
        outgoing: vec![],
        pending: false,
    };
    assert_eq!(
        drive_subscribed_socket(&pool, &session, &mut context, None, &mut socket)
            .await
            .unwrap_err()
            .to_string(),
        "Parallax socket ended"
    );
}

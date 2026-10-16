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

struct ScriptSocket {
    incoming: VecDeque<Result<Message, SocketError>>,
    outgoing: Vec<Message>,
    pending: bool,
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

use super::*;
use crate::ingestion::trip_signals::TripSignalFusion;
use chrono::Duration;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn at(ms: i64) -> DateTime<Utc> {
    "2026-10-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::milliseconds(ms)
}

fn sample(ms: i64, fields: Value) -> TelemetryEvent {
    let mut value =
        serde_json::to_value(TelemetryEvent::empty(Uuid::from_u128(17), at(ms))).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

enum Input {
    Sample(Box<TelemetryEvent>),
    Restart,
    Interrupt(i64),
}

fn frame(ms: i64, fields: Value) -> Input {
    Input::Sample(Box::new(sample(ms, fields)))
}

struct Stream {
    name: String,
    fusion: Option<bool>,
    inputs: Vec<Input>,
}

fn streams() -> Vec<Stream> {
    let mut streams = Vec::new();
    let mut add = |name: String, fusion, inputs| {
        streams.push(Stream {
            name,
            fusion,
            inputs,
        })
    };
    for power in [
        Value::Null,
        json!("sleep"),
        json!("ready"),
        json!("go"),
        json!("drive"),
        json!("charging"),
        json!("unknown"),
    ] {
        for speed in [Value::Null, json!(0.0), json!(2.0), json!(2.000_000_001)] {
            add(
                format!("start/{power}/{speed}"),
                None,
                vec![
                    frame(0, json!({"power_state":power,"speed_mph":speed})),
                    frame(1_000, json!({"power_state":"sleep"})),
                ],
            );
        }
    }
    for stopped_ms in [299_000, 300_000, 300_999, 301_000] {
        add(
            format!("stopped/{stopped_ms}"),
            None,
            vec![
                frame(0, json!({"power_state":"ready","speed_mph":30.0})),
                frame(stopped_ms, json!({})),
                frame(302_000, json!({"power_state":"sleep","speed_mph":30.0})),
            ],
        );
    }
    for age in [899_000, 900_000, 900_999, 901_000, -1_000] {
        add(
            format!("gear_age/{age}"),
            None,
            vec![
                frame(
                    -10_000,
                    json!({"power_state":"ready","speed_mph":0.0,"odometer_miles":100.0,"latitude":30.0,"longitude":-97.0}),
                ),
                frame(
                    0,
                    json!({"power_state":"drive","speed_mph":0.0,"odometer_miles":100.0}),
                ),
                frame(
                    age,
                    json!({"power_state":"drive","speed_mph":20.0,"odometer_miles":100.5,"latitude":30.01,"longitude":-97.01}),
                ),
                frame(
                    age + 1_000,
                    json!({"power_state":"sleep","odometer_miles":101.0}),
                ),
            ],
        );
    }
    for (name, shift, moving) in [
        (
            "rollback",
            json!({"power_state":"go","odometer_miles":101.0}),
            json!({"power_state":"go","speed_mph":30.0,"odometer_miles":90.0}),
        ),
        (
            "missing_motion_odometer",
            json!({"power_state":"drive","odometer_miles":100.2}),
            json!({"power_state":"drive","speed_mph":30.0}),
        ),
        (
            "ready_clears_shift",
            json!({"power_state":"drive","odometer_miles":100.2}),
            json!({"power_state":"ready","speed_mph":30.0,"odometer_miles":100.5}),
        ),
        (
            "missing_power_holds_shift",
            json!({"power_state":"go","odometer_miles":100.2}),
            json!({"speed_mph":30.0,"odometer_miles":100.5}),
        ),
    ] {
        add(
            name.into(),
            None,
            vec![
                frame(
                    0,
                    json!({"power_state":"ready","odometer_miles":100.0,"battery_level":80.0,"distance_to_empty_mi":250.0,"latitude":30.0,"longitude":-97.0}),
                ),
                frame(10_000, shift),
                frame(20_000, moving),
                frame(30_000, json!({"power_state":"go","speed_mph":30.0})),
                frame(40_000, json!({"power_state":"sleep"})),
            ],
        );
    }
    add(
        "metrics_partial_regen".into(),
        None,
        vec![
            frame(
                0,
                json!({"power_state":"drive","speed_mph":30.0,"battery_level":80.0,"battery_capacity_wh":135000.0,"distance_to_empty_mi":250.0,"odometer_miles":100.0,"latitude":0.0,"longitude":0.0,"altitude_m":10.0,"cabin_temp_c":99.0,"outside_temp_c":99.0,"power_kw":12.0,"drive_mode":"sport"}),
            ),
            frame(
                4_000,
                json!({"latitude":30.0,"longitude":-97.0,"altitude_m":12.0,"cabin_temp_c":20.0,"outside_temp_c":10.0,"drive_mode":"conserve"}),
            ),
            frame(5_000, json!({"regen_power_kw":-36.0,"power_kw":-36.0})),
            frame(3_000, json!({"regen_power_kw":-36.0,"altitude_m":9.0})),
            frame(5_000, json!({"regen_power_kw":-36.0})),
            frame(
                10_000,
                json!({"regen_power_kw":-36.0,"drive_mode":"conserve","latitude":30.01,"longitude":-97.01,"cabin_temp_c":22.0,"outside_temp_c":12.0}),
            ),
            frame(12_000, json!({"regen_power_kw":0.0})),
            frame(
                15_000,
                json!({"power_state":"sleep","regen_power_kw":-36.0,"power_kw":40.0,"altitude_m":15.0,"battery_level":78.0,"distance_to_empty_mi":245.0,"odometer_miles":102.0,"latitude":30.02,"longitude":-97.02,"cabin_temp_c":24.0,"outside_temp_c":14.0,"drive_mode":"sport"}),
            ),
        ],
    );
    for restart in [false, true] {
        let mut inputs = vec![
            frame(
                0,
                json!({"power_state":"ready","odometer_miles":100.0,"latitude":30.0,"longitude":-97.0}),
            ),
            frame(
                10_000,
                json!({"power_state":"drive","speed_mph":30.0,"odometer_miles":100.5,"altitude_m":10.0,"regen_power_kw":0.0}),
            ),
            frame(
                20_000,
                json!({"regen_power_kw":-36.0,"altitude_m":12.0,"cabin_temp_c":20.0,"drive_mode":"sport"}),
            ),
        ];
        if restart {
            inputs.push(Input::Restart);
        }
        inputs.extend([
            frame(25_000, json!({"regen_power_kw":-36.0,"altitude_m":30.0,"cabin_temp_c":22.0,"drive_mode":"sport"})),
            frame(321_000, json!({"power_state":"drive","odometer_miles":101.0})),
            frame(330_000, json!({"power_state":"drive","speed_mph":30.0,"odometer_miles":101.6})),
            Input::Interrupt(340_000), Input::Interrupt(341_000),
        ]);
        add(format!("restart/{restart}"), None, inputs);
    }
    add(
        "idle_restart_clears_anchor".into(),
        None,
        vec![
            frame(
                0,
                json!({"power_state":"ready","odometer_miles":100.0,"latitude":30.0,"longitude":-97.0}),
            ),
            frame(10_000, json!({"power_state":"drive"})),
            Input::Restart,
            frame(
                20_000,
                json!({"power_state":"drive","speed_mph":30.0,"odometer_miles":101.0}),
            ),
            Input::Interrupt(30_000),
        ],
    );
    for sparse in [false, true] {
        let mut inputs = vec![
            frame(
                0,
                json!({"power_state":"ready","speed_mph":0.0,"odometer_miles":200.0,"latitude":30.0,"longitude":-97.0,"battery_level":80.0,"battery_capacity_wh":135000.0}),
            ),
            frame(
                10_000,
                json!({"power_state":"drive","odometer_miles":200.0,"latitude":30.0,"longitude":-97.0}),
            ),
        ];
        for step in 1..=6 {
            let mut fields = json!({"odometer_miles":200.0 + 0.25 * step as f64,"latitude":30.0,"longitude":-97.0,"regen_power_kw":-3.6,"battery_level":80.0 - step as f64 * 0.25});
            if !sparse {
                fields["power_state"] = json!("drive");
                fields["speed_mph"] = json!(30.0);
            }
            inputs.push(frame(10_000 + step * 30_000, fields));
        }
        inputs.push(frame(
            220_000,
            json!({"power_state":"sleep","speed_mph":0.0,"odometer_miles":201.5}),
        ));
        add(format!("fusion/{sparse}"), Some(sparse), inputs);
    }
    streams
}

fn record_event(event: TripEvent) -> Value {
    match event {
        TripEvent::NoChange => json!({"transition":"no_change"}),
        TripEvent::TripStarted {
            trip_id,
            started_at,
        } => json!({"transition":"started","trip_id":trip_id,"started_at":started_at}),
        TripEvent::TripEnded { trip } => {
            let distance = compute_distance_odometer_or_gps(
                trip.start_odometer_mi,
                trip.end_odometer_mi,
                &trip.points,
            );
            let energy = compute_trip_energy(
                trip.soc_start,
                trip.soc_end,
                trip.battery_capacity_wh,
                trip.range_start_mi,
                trip.range_end_mi,
                distance,
                Some(400.0),
            );
            json!({"transition":"ended","trip":trip,"distance":distance,"energy":energy})
        }
    }
}

fn normalize_ids(value: &mut Value, ids: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "trip_id" | "active_trip_id") {
                    if let Some(id) = value.as_str() {
                        let next = format!("trip-{}", ids.len() + 1);
                        *value = json!(ids.entry(id.into()).or_insert(next));
                    }
                } else {
                    normalize_ids(value, ids);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_ids(value, ids);
            }
        }
        _ => {}
    }
}

fn traces() -> BTreeMap<String, Value> {
    streams().into_iter().map(|stream| {
        let mut detector = TripDetectorState::new(Uuid::from_u128(17));
        let mut fusion = stream.fusion.map(TripSignalFusion::new);
        let mut active = None;
        let mut allocated = Vec::new();
        let mut records = Vec::new();
        let mut ids = BTreeMap::new();
        for input in stream.inputs {
            let record = match input {
                Input::Sample(raw) => {
                    let original = serde_json::to_value(&raw).unwrap();
                    let (sample, diagnostics) = match fusion.as_mut() {
                        Some(fusion) => {
                            let (joined, diagnostics) = fusion.fuse_from(&raw, stream.fusion.unwrap());
                            (joined, Some(format!("{diagnostics:?}")))
                        }
                        None => (*raw.clone(), None),
                    };
                    assert_eq!(serde_json::to_value(&raw).unwrap(), original);
                    let decision = detector.start_decision(&sample);
                    let event = detector.process(&sample);
                    match &event {
                        TripEvent::TripStarted { trip_id, .. } => {
                            assert_eq!(active, None);
                            assert!(!allocated.contains(trip_id));
                            assert!(!trip_id.is_nil());
                            active = Some(*trip_id);
                            allocated.push(*trip_id);
                        }
                        TripEvent::TripEnded { trip } => {
                            assert_eq!(Some(trip.trip_id), active.take());
                        }
                        TripEvent::NoChange => {}
                    }
                    assert_eq!(detector.active_trip_id(), active);
                    json!({"input":sample,"diagnostics":diagnostics,"decision":decision,"event":record_event(event),"state":detector})
                }
                Input::Restart => {
                    detector = serde_json::from_value(serde_json::to_value(&detector).unwrap()).unwrap();
                    detector.resume_after_restart();
                    assert_eq!(detector.active_trip_id(), active);
                    assert_eq!(detector.vehicle_id(), Uuid::from_u128(17));
                    json!({"restart":true,"state":detector})
                }
                Input::Interrupt(ms) => {
                    let event = detector.close_interrupted_trip(at(ms));
                    if let TripEvent::TripEnded { trip } = &event {
                        assert_eq!(Some(trip.trip_id), active.take());
                    } else { assert_eq!(active, None); }
                    assert_eq!(detector.active_trip_id(), None);
                    json!({"interrupt":ms,"event":record_event(event),"state":detector})
                }
            };
            records.push(record);
        }
        let mut result = json!(records);
        normalize_ids(&mut result, &mut ids);
        (stream.name, result)
    }).collect()
}

#[test]
fn complete_fixed_streams_match_baseline() {
    let expected: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("trip_domain_contract_golden.json")).unwrap();
    let actual = traces();
    assert_eq!(actual.len(), expected.len());
    for (name, records) in actual {
        assert_eq!(records, expected[&name], "stream {name}");
    }
}

#[test]
#[ignore = "explicit baseline capture only; never regenerates fixtures during validation"]
fn capture_trip_domain_baseline() {
    let path = std::env::var("TRIP_DOMAIN_CAPTURE").expect("explicit capture destination");
    let traces = traces();
    let lines: Vec<_> = traces
        .iter()
        .map(|(name, value)| format!("{}:{}", json!(name), value))
        .collect();
    std::fs::write(path, format!("{{\n{}\n}}\n", lines.join(",\n"))).unwrap();
}

#[test]
fn energy_and_distance_fallback_boundaries() {
    for (soc_delta, range_delta, expected) in [
        (1.0, 2.0, Some((1350.0, "soc_delta"))),
        (0.999, 1.0, Some((400.0, "range_delta"))),
        (0.0, 0.999, Some((800.0, "historical"))),
        (-1.0, -1.0, Some((800.0, "historical"))),
    ] {
        assert_eq!(
            compute_trip_energy(
                Some(soc_delta),
                Some(0.0),
                Some(135000.0),
                Some(range_delta),
                Some(0.0),
                2.0,
                Some(400.0)
            ),
            expected
        );
    }
    assert_eq!(
        compute_trip_energy(
            Some(2.0),
            Some(0.0),
            Some(0.0),
            None,
            None,
            2.0,
            Some(400.0)
        ),
        Some((0.0, "soc_delta"))
    );
    assert_eq!(
        compute_trip_energy(None, None, None, Some(2.0), Some(0.0), 0.0, Some(-1.0)),
        Some((-2.0, "range_delta"))
    );
    assert_eq!(
        compute_trip_energy(None, None, None, None, None, 0.0, Some(400.0)),
        None
    );
    assert_eq!(
        compute_trip_energy(None, None, None, Some(2.0), Some(0.0), 2.0, None),
        None
    );
    assert!(compute_trip_energy(
        Some(2.0),
        Some(0.0),
        Some(f64::NAN),
        None,
        None,
        2.0,
        Some(400.0)
    )
    .unwrap()
    .0
    .is_nan());
    let points = vec![
        TrackPoint {
            ts: at(0),
            lat: 30.0,
            lng: -97.0,
            speed_mph: 30.0,
            altitude_m: None,
        },
        TrackPoint {
            ts: at(1000),
            lat: 30.01,
            lng: -97.01,
            speed_mph: 30.0,
            altitude_m: None,
        },
    ];
    let gps = compute_distance_miles(&points);
    assert!(gps > 0.0);
    for (start, end) in [
        (None, None),
        (Some(1.0), None),
        (Some(1.0), Some(1.0)),
        (Some(2.0), Some(1.0)),
        (Some(f64::NAN), Some(1.0)),
    ] {
        assert_eq!(compute_distance_odometer_or_gps(start, end, &points), gps);
    }
    assert_eq!(
        compute_distance_odometer_or_gps(Some(1.0), Some(3.0), &points),
        2.0
    );
    assert_eq!(compute_distance_miles(&[]), 0.0);
}

#[test]
fn nonfinite_regen_does_not_advance_interval_and_direct_speed_is_unfiltered() {
    let mut detector = TripDetectorState::new(Uuid::nil());
    let mut event = sample(0, json!({"power_state":"drive"}));
    event.speed_mph = Some(f64::NAN);
    assert_eq!(detector.start_decision(&event), "speed_below_threshold");
    event.speed_mph = Some(f64::INFINITY);
    assert_eq!(detector.process(&event).transition_name(), "started");
    for (ms, regen) in [
        (2000, f64::NAN),
        (3000, f64::NEG_INFINITY),
        (4000, f64::INFINITY),
    ] {
        event.ts = at(ms);
        event.regen_power_kw = Some(regen);
        detector.process(&event);
        assert_eq!(detector.last_regen_ts, Some(at(0)));
    }
    event.ts = at(5000);
    event.regen_power_kw = Some(-36.0);
    event.power_state = Some(PowerState::Sleep);
    let TripEvent::TripEnded { trip } = detector.process(&event) else {
        panic!("missing end");
    };
    assert_eq!(trip.regen_wh, Some(50.0));
}

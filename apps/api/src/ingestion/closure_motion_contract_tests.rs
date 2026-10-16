use super::*;
use serde_json::{json, Value};
use uuid::Uuid;

fn at(ms: i64) -> DateTime<Utc> {
    "2026-10-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::milliseconds(ms)
}

fn observe(tracker: &mut ClosureMotionTracker, now: i64, fields: Value) -> Value {
    let mut event =
        serde_json::to_value(TelemetryEvent::empty(Uuid::from_u128(17), at(-100000))).unwrap();
    event
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    let event: TelemetryEvent = serde_json::from_value(event).unwrap();
    let output = tracker.observe(&event, at(now));
    json!({"output":output,"settled":tracker.settled_closed,"motion":tracker.motion})
}

fn traces() -> BTreeMap<String, Value> {
    let mut traces = BTreeMap::new();
    let empty = TelemetryEvent::empty(Uuid::nil(), at(0));
    for (field, _) in closure_values(&empty) {
        for closed in [true, false] {
            let mut tracker = ClosureMotionTracker::default();
            let mut rows = vec![
                observe(&mut tracker, 0, json!({field:closed})),
                observe(
                    &mut tracker,
                    1000,
                    json!({field:!closed,"closure_transitions":{field:"ajar"}}),
                ),
                observe(&mut tracker, 2000, json!({field:closed})),
                observe(
                    &mut tracker,
                    3000,
                    json!({"closure_transitions":{field:"ajar"}}),
                ),
                observe(&mut tracker, 32999, json!({})),
                observe(&mut tracker, 33000, json!({})),
                observe(
                    &mut tracker,
                    34000,
                    json!({"closure_transitions":{field:"ajar"}}),
                ),
                observe(&mut tracker, 35000, json!({field:!closed})),
                observe(&mut tracker, 36000, json!({})),
            ];
            rows.push(observe(
                &mut tracker,
                37000,
                json!({"closure_transitions":{field:"ajar"}}),
            ));
            traces.insert(format!("{field}/{closed}"), json!(rows));
        }
    }
    let mut tracker = ClosureMotionTracker::default();
    let rows = vec![
        observe(
            &mut tracker,
            0,
            json!({"closure_transitions":{"future_closure":"ajar"}}),
        ),
        observe(
            &mut tracker,
            1000,
            json!({"closure_transitions":{"future_closure":"opening","closure_frunk_closed":"closing"}}),
        ),
        observe(
            &mut tracker,
            -1000,
            json!({"closure_transitions":{"future_closure":"ajar"}}),
        ),
        observe(
            &mut tracker,
            2000,
            json!({"closure_transitions":{"future_closure":"closing","closure_frunk_closed":"ajar"}}),
        ),
        observe(&mut tracker, 31000, json!({})),
        observe(&mut tracker, 32000, json!({})),
    ];
    traces.insert(
        "unknown_fields_direction_reversal_backward_clock".into(),
        json!(rows),
    );
    traces
}

#[test]
fn complete_motion_maps_and_memory_match_baseline() {
    let expected: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("closure_motion_contract_golden.json")).unwrap();
    assert_eq!(traces(), expected);
}

#[test]
#[ignore = "explicit baseline capture only; never regenerates fixtures during validation"]
fn capture_closure_motion_baseline() {
    let path = std::env::var("CLOSURE_MOTION_CAPTURE").expect("explicit capture destination");
    let lines: Vec<_> = traces()
        .iter()
        .map(|(name, value)| format!("{}:{}", json!(name), value))
        .collect();
    std::fs::write(path, format!("{{\n{}\n}}\n", lines.join(",\n"))).unwrap();
}

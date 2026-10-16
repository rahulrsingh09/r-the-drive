use super::*;
use serde_json::{json, Value};

pub(super) fn all_fields() -> TelemetryEvent {
    serde_json::from_str(include_str!("telemetry_contract_fixtures/all-fields.json")).unwrap()
}

pub(super) fn golden(name: &str, actual: Value, expected: &str) {
    assert_eq!(
        actual,
        serde_json::from_str::<Value>(expected).unwrap(),
        "{name}"
    );
}

#[test]
fn every_sparse_field_preserves_identity_and_arrival_order() {
    let full = all_fields();
    let source = serde_json::to_value(&full).unwrap();
    let target_id = Uuid::nil();
    for (field, value) in source.as_object().unwrap() {
        if matches!(field.as_str(), "vehicle_id" | "ts") {
            continue;
        }
        let mut patch = serde_json::to_value(blank_event(full.vehicle_id, full.ts)).unwrap();
        patch[field] = value.clone();
        let patch: TelemetryEvent = serde_json::from_value(patch).unwrap();
        let mut target = blank_event(target_id, full.ts + Duration::days(1));
        patch_event(&mut target, &patch);
        assert_eq!(target.vehicle_id, target_id, "{field}");
        assert_eq!(target.ts, full.ts, "{field}");
        let got = serde_json::to_value(&target).unwrap();
        if field == "closure_transitions" {
            assert!(got.get(field).is_none());
        } else {
            assert_eq!(&got[field], value, "{field}");
        }
        let meaningful = field != "closure_transitions" && !field.ends_with("_ts");
        assert_eq!(event_has_meaningful_payload(&patch), meaningful, "{field}");
        assert!(event_is_duplicate_patch(&patch, &full), "{field}");
    }
    let mut merged = full.clone();
    let mut patch = blank_event(Uuid::nil(), full.ts - Duration::hours(1));
    patch.battery_level = Some(1.0);
    patch.battery_level_ts = Some(full.ts - Duration::days(1));
    patch_event(&mut merged, &patch);
    assert_eq!(merged.battery_level, Some(1.0));
    assert_eq!(merged.battery_level_ts, patch.battery_level_ts);
    assert_eq!(merged.latitude, full.latitude);
    assert_eq!(merged.closure_transitions, full.closure_transitions);
}

#[test]
fn complete_sparse_snapshot_contract() {
    let event = all_fields();
    let full: Value = serde_json::from_str(&build_snapshot(&event, None)).unwrap();
    golden(
        "snapshot",
        full,
        include_str!("telemetry_contract_fixtures/snapshot.json"),
    );
    let mut partial = blank_event(event.vehicle_id, event.ts);
    partial.latitude = Some(5.0);
    partial.ota_current_status = Some("installing".into());
    let snapshot: Value =
        serde_json::from_str(&build_snapshot(&partial, Some(&Default::default()))).unwrap();
    assert_eq!(
        snapshot,
        json!({
            "type":"status", "ts":event.ts,
            "data":{"is_online":true,"ota_current_status":"installing",
                    "software_update_status":"installing","closure_motion":{}}
        })
    );
}

#[test]
fn nonfinite_and_signed_zero_have_explicit_projection_and_gate_behavior() {
    let mut event = blank_event(Uuid::nil(), all_fields().ts);
    event.battery_level = Some(f64::from_bits(0x7ff8_0000_0000_0042));
    event.power_kw = Some(f64::INFINITY);
    event.latitude = Some(f64::NEG_INFINITY);
    event.longitude = Some(-0.0);
    let mut patched = blank_event(event.vehicle_id, event.ts);
    patch_event(&mut patched, &event);
    assert_eq!(
        patched.battery_level.unwrap().to_bits(),
        0x7ff8_0000_0000_0042
    );
    assert_eq!(patched.longitude.unwrap().to_bits(), (-0.0_f64).to_bits());
    assert!(!event_is_duplicate_patch(&event, &event));
    let snapshot: Value = serde_json::from_str(&build_snapshot(&event, None)).unwrap();
    assert_eq!(snapshot["data"]["battery_level"], Value::Null);
    assert_eq!(snapshot["data"]["power_kw"], Value::Null);
    assert_eq!(snapshot["data"]["location"]["lat"], Value::Null);
    assert_eq!(
        snapshot["data"]["location"]["lng"]
            .as_f64()
            .unwrap()
            .to_bits(),
        (-0.0_f64).to_bits()
    );
    let mut gate = TelemetryPersistenceGate::new(event.vehicle_id);
    assert_eq!(gate.decide(&event), PersistenceDecision::Persist);
    event.ts += Duration::seconds(60);
    assert_eq!(
        gate.decide(&event),
        PersistenceDecision::Suppress(SuppressionReason::Threshold)
    );
    let mut zero = blank_event(event.vehicle_id, event.ts);
    zero.battery_level = Some(-0.0);
    let mut positive_zero = zero.clone();
    positive_zero.battery_level = Some(0.0);
    assert!(event_is_duplicate_patch(&zero, &positive_zero));
}

#[test]
fn gate_keeps_threshold_accumulation_timestamp_only_and_late_precedence() {
    let mut gate = TelemetryPersistenceGate::new(Uuid::nil());
    let mut event = blank_event(Uuid::nil(), all_fields().ts);
    event.power_state = Some(PowerState::Drive);
    event.speed_mph = Some(10.0);
    assert_eq!(gate.decide(&event), PersistenceDecision::Persist);
    let initial = event.ts;
    for (seconds, speed, decision) in [
        (
            4,
            12.0,
            PersistenceDecision::Suppress(SuppressionReason::Threshold),
        ),
        (
            5,
            12.0,
            PersistenceDecision::Suppress(SuppressionReason::Duplicate),
        ),
        (5, 12.1, PersistenceDecision::Persist),
        (
            -10,
            30.0,
            PersistenceDecision::Suppress(SuppressionReason::Threshold),
        ),
    ] {
        let mut next = blank_event(event.vehicle_id, initial + Duration::seconds(seconds));
        next.speed_mph = Some(speed);
        assert_eq!(gate.decide(&next), decision);
    }
    assert_eq!(gate.latest.as_ref().unwrap().speed_mph, Some(30.0));
    assert_eq!(gate.last_persisted_at, Some(initial + Duration::seconds(5)));
    let mut timestamp = blank_event(event.vehicle_id, initial + Duration::days(1));
    timestamp.speed_mph_ts = Some(timestamp.ts);
    assert_eq!(
        gate.decide(&timestamp),
        PersistenceDecision::Suppress(SuppressionReason::Empty)
    );
    assert_eq!(
        gate.latest.as_ref().unwrap().ts,
        initial - Duration::seconds(10)
    );
}

#[test]
fn lifecycle_precedence_matrix_includes_sparse_and_provider_guards() {
    for (power, charger, ota, online, expected) in [
        (None, None, None, None, VehicleState::Unknown),
        (None, None, None, Some(false), VehicleState::Offline),
        (
            Some(PowerState::Ready),
            None,
            None,
            Some(false),
            VehicleState::Offline,
        ),
        (
            Some(PowerState::Drive),
            None,
            None,
            Some(false),
            VehicleState::Drive,
        ),
        (
            Some(PowerState::Sleep),
            None,
            Some("installing"),
            Some(false),
            VehicleState::Updating,
        ),
        (
            Some(PowerState::Sleep),
            Some(ChargerState::Charging),
            Some("installing"),
            Some(false),
            VehicleState::Charging,
        ),
    ] {
        let mut event = blank_event(Uuid::nil(), all_fields().ts);
        event.power_state = power;
        event.charger_state = charger;
        event.ota_status = ota.map(str::to_owned);
        event.is_online = online;
        assert_eq!(infer_vehicle_state(&event), expected);
        for parallax in [false, true] {
            for active in [false, true] {
                let result = lifecycle_updates(&event, parallax, active);
                if parallax && active && event.charger_state.is_none() {
                    assert_eq!(
                        result,
                        LifecycleUpdates {
                            state_period: false,
                            power_state: false,
                            charging: false
                        }
                    );
                } else {
                    assert_eq!(
                        result.state_period,
                        event.power_state.is_some() || expected != VehicleState::Unknown
                    );
                    assert_eq!(result.power_state, event.power_state.is_some());
                    assert_eq!(
                        result.charging,
                        event.power_state.is_some() || event.charger_state.is_some()
                    );
                }
            }
        }
    }
}

#[test]
fn discarding_a_selected_write_preserves_both_gate_baselines() {
    let mut gate = TelemetryPersistenceGate::new(Uuid::nil());
    let mut event = blank_event(Uuid::nil(), all_fields().ts);
    event.power_state = Some(PowerState::Drive);
    event.speed_mph = Some(10.0);
    gate.decide(&event);
    let initial = event.ts;
    event.ts += Duration::seconds(1);
    event.speed_mph = Some(10.5);
    assert_eq!(
        gate.decide(&event),
        PersistenceDecision::Suppress(SuppressionReason::Threshold)
    );
    event.ts += Duration::seconds(5);
    event.speed_mph = Some(12.0);
    let failed = gate.select(&event);
    assert_eq!(failed.decision, PersistenceDecision::Persist);
    drop(failed);
    assert_eq!(gate.latest.as_ref().unwrap().speed_mph, Some(10.5));
    assert_eq!(gate.last_persisted.as_ref().unwrap().speed_mph, Some(10.0));
    assert_eq!(gate.last_persisted_at, Some(initial));
    let retry = gate.select(&event);
    assert_eq!(retry.decision, PersistenceDecision::Persist);
    gate.advance(retry);
    assert_eq!(gate.latest.as_ref().unwrap().speed_mph, Some(12.0));
    assert_eq!(gate.last_persisted_at, Some(event.ts));
    assert_eq!(
        gate.select(&event).decision,
        PersistenceDecision::Suppress(SuppressionReason::Duplicate)
    );
}

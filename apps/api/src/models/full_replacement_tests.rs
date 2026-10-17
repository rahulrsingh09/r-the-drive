use super::{cost_profile::{validate_tou_periods, TouPeriod}, telemetry::{TelemetryEvent, PowerState, ChargerState, DriveMode}, state_period::VehicleState};
use chrono::{TimeZone, Utc};
use uuid::Uuid;

#[test]
fn empty_telemetry_and_all_charging_signal_combinations_are_fixed() {
    let vehicle_id = Uuid::nil();
    let ts = Utc.timestamp_opt(1_700_000_000, 123_000_000).unwrap();
    let event = TelemetryEvent::empty(vehicle_id, ts);
    let mut serialized = serde_json::to_value(&event).unwrap();
    let fields = serialized.as_object_mut().unwrap();
    assert_eq!(fields.remove("vehicle_id").unwrap(), vehicle_id.to_string());
    assert_eq!(fields.remove("ts").unwrap(), "2023-11-14T22:13:20.123Z");
    assert!(!fields.contains_key("closure_transitions"));
    assert_eq!(fields.len(), 95);
    assert!(fields.values().all(serde_json::Value::is_null));
    for power in [None, Some(PowerState::Ready), Some(PowerState::Charging)] {
        for charger in [None, Some(ChargerState::Connected), Some(ChargerState::Charging)] {
            for status in [None, Some("chrgr_sts_connected_charging"), Some("CHRGR_STS_CONNECTED_CHARGING"), Some("chrgr_sts_not_connected")] {
                let mut event = TelemetryEvent::empty(vehicle_id, ts);
                event.power_state = power.clone(); event.charger_state = charger.clone(); event.charger_status = status.map(str::to_owned);
                assert_eq!(event.is_actively_charging(), power == Some(PowerState::Charging) || charger == Some(ChargerState::Charging) || status == Some("chrgr_sts_connected_charging"));
            }
        }
    }
}

#[test]
fn enum_alias_case_unknown_and_whitespace_contracts() {
    for (text, expected) in [("sleep", "sleep"), ("READY", "ready"), ("Go", "go"), ("drive", "drive"), ("charging", "charging"), ("", "unknown"), (" drive", "unknown"), ("ſleep", "unknown")] {
        let state: PowerState = text.parse().unwrap(); assert_eq!(serde_json::to_value(state).unwrap(), expected);
    }
    for (text, expected) in [("disconnected", "disconnected"), ("CONNECTED", "connected"), ("charging_active", "charging"), ("Charging", "charging"), ("charging_done", "done"), ("DONE", "done"), (" done", "unknown"), ("", "unknown")] {
        let state: ChargerState = text.parse().unwrap(); assert_eq!(serde_json::to_value(state).unwrap(), expected);
    }
    for (text, expected) in [("SPORT", "sport"), ("normal", "all_purpose"), ("everyday", "all_purpose"), ("all_purpose", "all_purpose"), ("distance", "conserve"), ("conserve", "conserve"), ("winter", "snow"), ("snow", "snow"), ("off_road_auto", "all_terrain"), ("off_road", "all_terrain"), ("all_terrain", "all_terrain"), ("off_road_sand", "soft_sand"), ("soft_sand", "soft_sand"), ("off_road_rocks", "rock_crawl"), ("rock_crawl", "rock_crawl"), ("off_road_sport_auto", "rally"), ("rally", "rally"), ("off_road_sport_drift", "drift"), ("drift", "drift"), ("tow", "towing"), ("towing", "towing"), (" tow", "unknown"), ("", "unknown")] {
        let state: DriveMode = text.parse().unwrap(); assert_eq!(state.as_str(), expected); assert_eq!(serde_json::to_value(state).unwrap(), expected);
    }
    for text in ["drive", "charging", "ready", "sleep", "offline", "updating", "unknown"] {
        assert_eq!(text.parse::<VehicleState>().unwrap().to_string(), text);
        assert_eq!(text.to_uppercase().parse::<VehicleState>().unwrap_err(), format!("unknown vehicle state {:?}", text.to_uppercase()));
    }
}

#[test]
fn tou_validation_boundary_and_error_precedence_contract() {
    let period = |label: &str, start, end, rate| TouPeriod { label: label.into(), start_minute: start, end_minute: end, rate };
    assert_eq!(validate_tou_periods(&[]).unwrap_err(), "TOU schedules need at least one period.");
    for rate in [0.0, -0.0, 0.32, 5.0] { assert!(validate_tou_periods(&[period("day", 0, 1440, rate)]).is_ok()); }
    for rate in [f64::NEG_INFINITY, f64::INFINITY, f64::NAN, -1.0] { assert_eq!(validate_tou_periods(&[period("day", 0, 1440, rate)]).unwrap_err(), "Each TOU period needs a non-negative rate."); }
    let cases = [
        (vec![period(" ", 4, 1, -1.0)], "Each TOU period needs a label."),
        (vec![period("day", 1, 0, 0.3)], "TOU periods must be contiguous and start at 00:00."),
        (vec![period("day", 0, 0, 0.3)], "Each TOU period must end after it starts."),
        (vec![period("day", 0, 1439, 0.3)], "TOU periods must cover the full day through 24:00."),
        (vec![period("day", 0, 1441, 0.3)], "TOU periods must cover the full day through 24:00."),
        (vec![period("a", 0, 60, 0.3), period("b", 61, 1440, 0.2)], "TOU periods must be contiguous and start at 00:00."),
    ];
    for (input, expected) in cases { assert_eq!(validate_tou_periods(&input).unwrap_err(), expected); }
    assert_eq!(validate_tou_periods(&[period("day", 0, 1440, 5.1)]).unwrap_err(), "Rate 5.1000 $/kWh is too high. Rates must be entered in dollars per kWh (e.g. 0.32 for 32¢/kWh), not cents.");
}

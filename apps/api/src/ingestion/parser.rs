//! Parses Rivian GraphQL WebSocket subscription messages into TelemetryEvent.
//!
//! Rivian sends partial updates — only changed fields are present.
//! Every field is optional. This function must never panic on any input.

use crate::models::telemetry::TelemetryEvent;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Missing vehicleState in payload")]
    MissingVehicleState,
    #[error("Invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChargingSessionEvent {
    pub live_data: Option<ChargingSessionLiveData>,
    pub live_data_present: bool,
    /// True only when the subscription explicitly returns
    /// `chargingSession: null`. An object with all nullable leaves is an
    /// empty partial update and must not clear a still-fresh live snapshot.
    pub explicit_null: bool,
    pub chart_data: Vec<ChargingSessionChartPoint>,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChargingSessionLiveData {
    pub power_kw: Option<f64>,
    pub kilometers_charged_per_hour: Option<f64>,
    pub range_added_this_session: Option<f64>,
    pub total_charged_energy: Option<f64>,
    pub time_elapsed_seconds: Option<f64>,
    pub time_remaining_seconds: Option<f64>,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub is_free_session: Option<bool>,
    pub vehicle_charger_state: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChargingSessionChartPoint {
    pub soc: Option<f64>,
    pub power_kw: Option<f64>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub time_estimation_validity_status: Option<String>,
    pub vehicle_charger_state: Option<String>,
}

/// Parse a subscription frame into an independent partial telemetry update.
pub fn parse_ws_message(raw: &str, vehicle_id: Uuid) -> Result<Option<TelemetryEvent>, ParseError> {
    let Some(frame) = SubscriptionFrame::decode(raw)? else {
        return Ok(None);
    };
    let state = frame
        .field("vehicleState")
        .ok_or(ParseError::MissingVehicleState)?;
    Ok(Some(VehicleInput(Leaf(Some(state))).into_event(vehicle_id)))
}

pub fn parse_charging_session_message(
    raw: &str,
) -> Result<Option<ChargingSessionEvent>, ParseError> {
    let Some(frame) = SubscriptionFrame::decode(raw)? else {
        return Ok(None);
    };
    match Presence::of(frame.field("chargingSession")) {
        Presence::Absent => Ok(None),
        Presence::Null => Ok(Some(ChargingSessionEvent {
            live_data: None,
            live_data_present: true,
            explicit_null: true,
            chart_data: Vec::new(),
            ts: Utc::now(),
        })),
        Presence::Value(session) => Ok(Some(parse_charging_session_value(session))),
    }
}

pub fn parse_battery_cell_type(raw: &str) -> Result<Option<String>, ParseError> {
    let Some(frame) = SubscriptionFrame::decode(raw)? else {
        return Ok(None);
    };
    Ok(Leaf(frame.field("vehicleState"))
        .child("batteryCellType")
        .child("value")
        .scalar()
        .text()
        .map(str::to_owned))
}

pub(crate) fn parse_charging_session_value(session: &Value) -> ChargingSessionEvent {
    SessionPatch::normalize(Leaf(Some(session))).into_event()
}

struct SubscriptionFrame(Value);

impl SubscriptionFrame {
    fn decode(raw: &str) -> Result<Option<Self>, ParseError> {
        let value: Value = serde_json::from_str(raw)?;
        Ok((value.get("type").and_then(Value::as_str) == Some("next")).then_some(Self(value)))
    }

    fn field(&self, name: &str) -> Option<&Value> {
        self.0.get("payload")?.get("data")?.get(name)
    }
}

enum Presence<T> {
    Absent,
    Null,
    Value(T),
}

impl<'a> Presence<&'a Value> {
    fn of(value: Option<&'a Value>) -> Self {
        match value {
            None => Self::Absent,
            Some(Value::Null) => Self::Null,
            Some(value) => Self::Value(value),
        }
    }
}

#[derive(Clone, Copy)]
struct Leaf<'a>(Option<&'a Value>);

impl<'a> Leaf<'a> {
    fn child(self, key: &str) -> Self {
        Self(self.0.and_then(|value| value.get(key)))
    }

    fn scalar(self) -> Self {
        Self(self.child("value").0.or(self.0))
    }

    fn text(self) -> Option<&'a str> {
        self.0?.as_str()
    }

    fn number(self, unit: NumericUnit) -> Option<f64> {
        let scalar = self.scalar().0?;
        let number = match scalar {
            Value::Number(value) => value.as_f64()?,
            Value::String(value) => value.trim().parse::<f64>().ok()?,
            _ => return None,
        };
        Some(unit.convert(number))
    }

    fn integer(self) -> Option<i32> {
        self.0?.as_i64()?.try_into().ok()
    }

    fn timestamp(self) -> Option<DateTime<Utc>> {
        self.text()?.parse().ok()
    }

    fn session_timestamp(self) -> Option<DateTime<Utc>> {
        self.scalar()
            .timestamp()
            .or_else(|| self.child("updatedAt").timestamp())
    }

    fn boolean(self, rule: BooleanRule) -> Option<bool> {
        if let Some(Value::Bool(value)) = self.0 {
            return (!matches!(rule, BooleanRule::StringFlag(_))).then_some(*value);
        }
        let text = match rule {
            BooleanRule::Charging => self.text()?.to_ascii_lowercase(),
            _ => self.text()?.to_lowercase(),
        };
        let (positive, negative): (&[&str], &[&str]) = match rule {
            BooleanRule::Strict => return None,
            BooleanRule::Charging => (&["true"], &["false"]),
            BooleanRule::Validity => (
                &["true", "valid", "ok", "yes"],
                &["false", "invalid", "bad", "no"],
            ),
            BooleanRule::Locked => (&["locked", "true"], &["unlocked", "false"]),
            BooleanRule::Closed => (&["closed", "true"], &["open", "opened", "false"]),
            BooleanRule::StringFlag(active) => return Some(active.contains(&text.as_str())),
            BooleanRule::Derate => {
                return Some(!["inactive", "none", "off"].contains(&text.as_str()))
            }
        };
        if positive.contains(&text.as_str()) {
            Some(true)
        } else if negative.contains(&text.as_str()) {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy)]
enum NumericUnit {
    Native,
    MetersPerSecond,
    Meters,
    Kilometers,
    KilowattHours,
    Bar,
}

impl NumericUnit {
    fn convert(self, value: f64) -> f64 {
        match self {
            Self::Native => value,
            Self::MetersPerSecond => value * 2.236_94,
            Self::Meters => value / 1609.344,
            Self::Kilometers => value / 1.609_344,
            Self::KilowattHours => value * 1000.0,
            Self::Bar => value * 14.503_773_8,
        }
    }
}

#[derive(Clone, Copy)]
enum BooleanRule {
    Strict,
    Charging,
    Validity,
    Locked,
    Closed,
    StringFlag(&'static [&'static str]),
    Derate,
}

struct Sample<T> {
    value: Option<T>,
    timestamp: Option<DateTime<Utc>>,
}

struct VehicleField<T> {
    source: &'static str,
    decode: fn(Leaf<'_>) -> Option<T>,
    assign: fn(&mut TelemetryEvent, Sample<T>),
}

impl<T> VehicleField<T> {
    fn normalize(&self, input: VehicleInput<'_>) -> Sample<T> {
        let field = input.0.child(self.source);
        Sample {
            value: (self.decode)(field.child("value")),
            timestamp: field.child("timeStamp").timestamp(),
        }
    }
}

// Bind timestamps to the same source as values without making them depend on successful decoding.
macro_rules! vehicle_field {
    ($source:literal, $target:ident $(, $timestamp:ident)? => $decode:expr) => {
        VehicleField {
            source: $source,
            decode: $decode,
            assign: |event, sample| {
                event.$target = sample.value;
                $(event.$timestamp = sample.timestamp;)?
            },
        }
    };
}

const NUMBER_FIELDS: &[VehicleField<f64>] = &[
    vehicle_field!("gnssAltitude", altitude_m => |v| v.number(NumericUnit::Native)),
    vehicle_field!("gnssSpeed", speed_mph, speed_mph_ts => |v| v.number(NumericUnit::MetersPerSecond)),
    vehicle_field!("batteryLevel", battery_level, battery_level_ts => |v| v.number(NumericUnit::Native)),
    vehicle_field!("batteryCapacity", battery_capacity_wh => |v| v.number(NumericUnit::KilowattHours)),
    vehicle_field!("distanceToEmpty", distance_to_empty_mi, distance_to_empty_mi_ts => |v| v.number(NumericUnit::Kilometers)),
    vehicle_field!("batteryLimit", battery_limit, battery_limit_ts => |v| v.number(NumericUnit::Native)),
    vehicle_field!("cabinClimateInteriorTemperature", cabin_temp_c => |v| v.number(NumericUnit::Native)),
    vehicle_field!("cabinClimateDriverTemperature", driver_temp_c => |v| v.number(NumericUnit::Native)),
    vehicle_field!("cabinClimateExteriorTemperature", outside_temp_c => |v| v.number(NumericUnit::Native)),
    vehicle_field!("vehiclePowerOutput", power_kw => |v| v.number(NumericUnit::Native)),
    vehicle_field!("regenerativeBrakingPower", regen_power_kw => |v| v.number(NumericUnit::Native)),
    vehicle_field!("vehicleMileage", odometer_miles, odometer_miles_ts => |v| v.number(NumericUnit::Meters)),
    vehicle_field!("tirePressureFrontLeft", tire_fl_psi => |v| v.number(NumericUnit::Bar)),
    vehicle_field!("tirePressureFrontRight", tire_fr_psi => |v| v.number(NumericUnit::Bar)),
    vehicle_field!("tirePressureRearLeft", tire_rl_psi => |v| v.number(NumericUnit::Bar)),
    vehicle_field!("tirePressureRearRight", tire_rr_psi => |v| v.number(NumericUnit::Bar)),
];

const INTEGER_FIELDS: &[VehicleField<i32>] = &[
    vehicle_field!("timeToEndOfCharge", time_to_end_of_charge_min, time_to_end_of_charge_min_ts => |v| v.integer()),
    vehicle_field!("steeringWheelHeat", steering_wheel_heat => |v| v.integer()),
    vehicle_field!("seatFrontLeftHeat", seat_fl_heat => |v| v.integer()),
    vehicle_field!("seatFrontRightHeat", seat_fr_heat => |v| v.integer()),
    vehicle_field!("seatRearLeftHeat", seat_rl_heat => |v| v.integer()),
    vehicle_field!("seatRearRightHeat", seat_rr_heat => |v| v.integer()),
    vehicle_field!("seatFrontLeftVent", seat_fl_vent => |v| v.integer()),
    vehicle_field!("seatFrontRightVent", seat_fr_vent => |v| v.integer()),
];

const TEXT_FIELDS: &[VehicleField<String>] = &[
    vehicle_field!("chargerStatus", charger_status, charger_status_ts => |v| v.text().map(str::to_owned)),
    vehicle_field!("gearStatus", gear_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("tirePressureStatusFrontLeft", tire_fl_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("tirePressureStatusFrontRight", tire_fr_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("tirePressureStatusRearLeft", tire_rl_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("tirePressureStatusRearRight", tire_rr_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("otaStatus", ota_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("otaCurrentStatus", ota_current_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("batteryHvThermalEvent", hv_thermal_event => |v| v.text().map(str::to_owned)),
    vehicle_field!("twelveVoltBatteryHealth", twelve_volt_health => |v| v.text().map(str::to_owned)),
    vehicle_field!("cabinPreconditioningStatus", cabin_precon_status => |v| v.text().map(str::to_owned)),
    vehicle_field!("cabinPreconditioningType", cabin_precon_type => |v| v.text().map(str::to_owned)),
    vehicle_field!("gearGuardVideoStatus", gear_guard_video_status => |v| v.text().map(str::to_owned)),
];

const BOOLEAN_FIELDS: &[VehicleField<bool>] = &[
    vehicle_field!("cabinClimateRunning", hvac_active => |v| v.boolean(BooleanRule::Strict)),
    vehicle_field!("tirePressureStatusValidFrontLeft", tire_fl_valid => |v| v.boolean(BooleanRule::Validity)),
    vehicle_field!("tirePressureStatusValidFrontRight", tire_fr_valid => |v| v.boolean(BooleanRule::Validity)),
    vehicle_field!("tirePressureStatusValidRearLeft", tire_rl_valid => |v| v.boolean(BooleanRule::Validity)),
    vehicle_field!("tirePressureStatusValidRearRight", tire_rr_valid => |v| v.boolean(BooleanRule::Validity)),
    vehicle_field!("doorFrontLeftLocked", door_front_left_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("doorFrontRightLocked", door_front_right_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("doorRearLeftLocked", door_rear_left_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("doorRearRightLocked", door_rear_right_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("doorFrontLeftClosed", door_front_left_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("doorFrontRightClosed", door_front_right_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("doorRearLeftClosed", door_rear_left_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("doorRearRightClosed", door_rear_right_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("closureFrunkLocked", closure_frunk_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("closureFrunkClosed", closure_frunk_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("closureLiftgateLocked", closure_liftgate_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("closureLiftgateClosed", closure_liftgate_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("closureTailgateLocked", closure_tailgate_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("closureTailgateClosed", closure_tailgate_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("chargePortState", charge_port_open => |v| v.boolean(BooleanRule::StringFlag(&["open", "ajar"]))),
    vehicle_field!("chargerDerateStatus", charger_derate_active => |v| v.boolean(BooleanRule::Derate)),
    vehicle_field!("petModeStatus", pet_mode_active => |v| v.boolean(BooleanRule::StringFlag(&["on", "active"]))),
    vehicle_field!("petModeTemperatureStatus", pet_mode_temp_ok => |v| v.boolean(BooleanRule::StringFlag(&["ok", "safe"]))),
    vehicle_field!("defrostDefogStatus", defrost_active => |v| v.boolean(BooleanRule::StringFlag(&["on", "active"]))),
    vehicle_field!("closureTonneauLocked", tonneau_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("closureTonneauClosed", tonneau_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("closureSideBinLeftLocked", side_bin_left_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("closureSideBinRightLocked", side_bin_right_locked => |v| v.boolean(BooleanRule::Locked)),
    vehicle_field!("closureSideBinLeftClosed", side_bin_left_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("closureSideBinRightClosed", side_bin_right_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("windowFrontLeftClosed", window_fl_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("windowFrontRightClosed", window_fr_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("windowRearLeftClosed", window_rl_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("windowRearRightClosed", window_rr_closed => |v| v.boolean(BooleanRule::Closed)),
    vehicle_field!("gearGuardLocked", gear_guard_locked => |v| v.boolean(BooleanRule::Strict)),
    vehicle_field!("wiperFluidState", wiper_fluid_low => |v| v.boolean(BooleanRule::StringFlag(&["low", "empty", "critical"]))),
    vehicle_field!("brakeFluidLow", brake_fluid_low => |v| v.boolean(BooleanRule::Strict)),
    vehicle_field!("alarmSoundStatus", alarm_active => |v| v.boolean(BooleanRule::StringFlag(&["active", "on", "sounding"]))),
];

#[derive(Clone, Copy)]
struct VehicleInput<'a>(Leaf<'a>);

impl<'a> VehicleInput<'a> {
    fn leaf(self, key: &str) -> Leaf<'a> {
        self.0.child(key).child("value")
    }

    fn text_alias(self, primary: &str, fallback: &str) -> Option<&'a str> {
        self.leaf(primary)
            .text()
            .or_else(|| self.leaf(fallback).text())
    }

    fn assign<T>(self, event: &mut TelemetryEvent, fields: &[VehicleField<T>]) {
        for field in fields {
            (field.assign)(event, field.normalize(self));
        }
    }

    fn into_event(self, vehicle_id: Uuid) -> TelemetryEvent {
        let ts = latest_timestamp(self.0).unwrap_or_else(Utc::now);
        let mut event = TelemetryEvent::empty(vehicle_id, ts);
        self.assign(&mut event, NUMBER_FIELDS);
        self.assign(&mut event, INTEGER_FIELDS);
        self.assign(&mut event, TEXT_FIELDS);
        self.assign(&mut event, BOOLEAN_FIELDS);
        self.assign(&mut event, &[
            vehicle_field!("powerState", power_state, power_state_ts => |v| v.text()?.parse().ok()),
        ]);
        self.assign(&mut event, &[
            vehicle_field!("chargerState", charger_state, charger_state_ts => |v| v.text()?.parse().ok()),
        ]);
        self.assign(
            &mut event,
            &[vehicle_field!("driveMode", drive_mode => |v| v.text()?.parse().ok())],
        );

        let location = self.0.child("gnssLocation");
        event.latitude = location.child("latitude").number(NumericUnit::Native);
        event.longitude = location.child("longitude").number(NumericUnit::Native);
        event.location_ts = location.child("timeStamp").timestamp();
        event.is_online = self
            .0
            .child("cloudConnection")
            .child("isOnline")
            .boolean(BooleanRule::Strict);
        event.heading_deg = self
            .leaf("gnssBearing")
            .number(NumericUnit::Native)
            .or_else(|| self.leaf("gnssHeading").number(NumericUnit::Native));
        event.ota_current_version = self
            .text_alias("otaCurrentVersion", "otaCurrentVersionGitHash")
            .map(str::to_owned);
        event.ota_available_version = self
            .text_alias("otaAvailableVersion", "otaAvailableVersionGitHash")
            .filter(|value| value.trim() != "0.0.0")
            .map(str::to_owned);
        event.service_mode = self
            .text_alias("serviceMode", "vehicleInServiceMode")
            .map(|text| ["on", "active", "true"].contains(&text.to_lowercase().as_str()));
        event
    }
}

fn latest_timestamp(root: Leaf<'_>) -> Option<DateTime<Utc>> {
    let mut pending: Vec<&Value> = root.0.into_iter().collect();
    let mut latest = None;
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                latest = latest.max(Leaf(fields.get("timeStamp")).timestamp());
                pending.extend(fields.values());
            }
            Value::Array(values) => pending.extend(values),
            _ => {}
        }
    }
    latest
}

struct SessionPatch {
    live: Presence<ChargingSessionLiveData>,
    chart: Vec<ChargingSessionChartPoint>,
}

impl SessionPatch {
    fn normalize(session: Leaf<'_>) -> Self {
        let live = match Presence::of(session.child("liveData").0) {
            Presence::Absent => Presence::Absent,
            Presence::Null => Presence::Null,
            Presence::Value(value) => {
                Presence::Value(ChargingSessionLiveData::normalize(Leaf(Some(value))))
            }
        };
        let chart = session
            .child("chartData")
            .0
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|value| ChargingSessionChartPoint::normalize(Leaf(Some(value))))
            .collect();
        Self { live, chart }
    }

    fn into_event(self) -> ChargingSessionEvent {
        let (live_data_present, live_data) = match self.live {
            Presence::Absent => (false, None),
            Presence::Null => (true, None),
            Presence::Value(live) => (true, Some(live)),
        };
        let ts = live_data
            .as_ref()
            .and_then(|live| live.start_time)
            .or_else(|| self.chart.first().and_then(|point| point.start_time))
            .unwrap_or_else(Utc::now);
        ChargingSessionEvent {
            live_data,
            live_data_present,
            explicit_null: false,
            chart_data: self.chart,
            ts,
        }
    }
}

impl ChargingSessionLiveData {
    fn normalize(input: Leaf<'_>) -> Self {
        Self {
            power_kw: input.child("powerKW").number(NumericUnit::Native),
            kilometers_charged_per_hour: input
                .child("kilometersChargedPerHour")
                .number(NumericUnit::Native),
            range_added_this_session: input
                .child("rangeAddedThisSession")
                .number(NumericUnit::Native),
            total_charged_energy: input
                .child("totalChargedEnergy")
                .number(NumericUnit::Native),
            time_elapsed_seconds: input.child("timeElapsed").number(NumericUnit::Native),
            time_remaining_seconds: input.child("timeRemaining").number(NumericUnit::Native),
            price: input.child("price").number(NumericUnit::Native),
            currency: input.child("currency").scalar().text().map(str::to_owned),
            is_free_session: input
                .child("isFreeSession")
                .scalar()
                .boolean(BooleanRule::Charging),
            vehicle_charger_state: input
                .child("vehicleChargerState")
                .scalar()
                .text()
                .map(str::to_owned),
            start_time: input.child("startTime").session_timestamp(),
        }
    }
}

impl ChargingSessionChartPoint {
    fn normalize(input: Leaf<'_>) -> Self {
        Self {
            soc: input.child("soc").number(NumericUnit::Native),
            power_kw: input.child("powerKW").number(NumericUnit::Native),
            start_time: input.child("startTime").session_timestamp(),
            end_time: input.child("endTime").session_timestamp(),
            time_estimation_validity_status: input
                .child("timeEstimationValidityStatus")
                .scalar()
                .text()
                .map(str::to_owned),
            vehicle_charger_state: input
                .child("vehicleChargerState")
                .scalar()
                .text()
                .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::telemetry::{DriveMode, PowerState};

    fn vid() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn connection_ack_returns_none() {
        let msg = r#"{"type":"connection_ack"}"#;
        assert!(parse_ws_message(msg, vid()).unwrap().is_none());
    }

    #[test]
    fn parse_battery_level() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "batteryLevel": { "timeStamp": "2024-01-15T10:30:00.000Z", "value": 82.5 },
                "powerState":   { "timeStamp": "2024-01-15T10:30:00.000Z", "value": "ready" }
            }}}
        })
        .to_string();
        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.battery_level, Some(82.5));
        assert_eq!(ev.power_state, Some(PowerState::Ready));
    }

    #[test]
    fn preserves_per_field_timestamps_for_freshness_tracking() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "batteryLevel": { "timeStamp": "2024-01-15T10:25:00Z", "value": 82.5 },
                "distanceToEmpty": { "timeStamp": "2024-01-15T10:26:00Z", "value": 161.0 },
                "powerState": { "timeStamp": "2024-01-15T10:27:00Z", "value": "ready" },
                "chargerStatus": { "timeStamp": "2024-01-15T10:28:00Z", "value": "chrgr_sts_not_connected" },
                "timeToEndOfCharge": { "timeStamp": "2024-01-15T10:29:00Z", "value": 45 },
                "gnssLocation": { "timeStamp": "2024-01-15T10:30:00Z", "latitude": 30.1, "longitude": -97.7 },
                "gnssSpeed": { "timeStamp": "2024-01-15T10:31:00Z", "value": 0.0 },
                "vehicleMileage": { "timeStamp": "2024-01-15T10:32:00Z", "value": 16093.44 }
            }}}
        })
        .to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(
            ev.battery_level_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:25:00+00:00"
        );
        assert_eq!(
            ev.distance_to_empty_mi_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:26:00+00:00"
        );
        assert_eq!(
            ev.power_state_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:27:00+00:00"
        );
        assert_eq!(
            ev.charger_status_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:28:00+00:00"
        );
        assert_eq!(
            ev.time_to_end_of_charge_min_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:29:00+00:00"
        );
        assert_eq!(
            ev.location_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:30:00+00:00"
        );
        assert_eq!(
            ev.speed_mph_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:31:00+00:00"
        );
        assert_eq!(
            ev.odometer_miles_ts.unwrap().to_rfc3339(),
            "2024-01-15T10:32:00+00:00"
        );
        assert_eq!(ev.ts.to_rfc3339(), "2024-01-15T10:32:00+00:00");
    }

    #[test]
    fn missing_vehicle_state_errors() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": {} }
        })
        .to_string();
        assert!(matches!(
            parse_ws_message(&msg, vid()),
            Err(ParseError::MissingVehicleState)
        ));
    }

    #[test]
    fn partial_update_leaves_fields_none() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "batteryLevel": { "timeStamp": "2024-01-15T10:30:00Z", "value": 75.0 }
            }}}
        })
        .to_string();
        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.battery_level, Some(75.0));
        assert!(ev.latitude.is_none());
        assert!(ev.power_state.is_none());
        assert!(ev.outside_temp_c.is_none());
        assert!(ev.power_kw.is_none());
    }

    #[test]
    fn parses_extended_metrics_when_present() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "cabinClimateExteriorTemperature": { "timeStamp": "2024-01-15T10:30:00Z", "value": 4.5 },
                "cabinClimateRunning": { "timeStamp": "2024-01-15T10:30:00Z", "value": true },
                "vehiclePowerOutput": { "timeStamp": "2024-01-15T10:30:00Z", "value": 22.1 },
                "regenerativeBrakingPower": { "timeStamp": "2024-01-15T10:30:00Z", "value": -8.4 },
                "gnssHeading": { "timeStamp": "2024-01-15T10:30:00Z", "value": 182.0 },
                "tirePressureFrontLeft": { "timeStamp": "2024-01-15T10:30:00Z", "value": 2.96 },
                "tirePressureFrontRight": { "timeStamp": "2024-01-15T10:30:00Z", "value": 2.95 },
                "tirePressureRearLeft": { "timeStamp": "2024-01-15T10:30:00Z", "value": 3.02 },
                "tirePressureRearRight": { "timeStamp": "2024-01-15T10:30:00Z", "value": 3.01 },
                "tirePressureStatusFrontLeft": { "timeStamp": "2024-01-15T10:30:00Z", "value": "OK" },
                "doorFrontLeftLocked": { "timeStamp": "2024-01-15T10:30:00Z", "value": "locked" },
                "doorFrontLeftClosed": { "timeStamp": "2024-01-15T10:30:00Z", "value": "closed" },
                "closureFrunkClosed": { "timeStamp": "2024-01-15T10:30:00Z", "value": "open" },
                "otaStatus": { "timeStamp": "2024-01-15T10:30:00Z", "value": "up_to_date" },
                "otaCurrentVersion": { "timeStamp": "2024-01-15T10:30:00Z", "value": "2024.11.02" }
            }}}
        }).to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.outside_temp_c, Some(4.5));
        assert_eq!(ev.hvac_active, Some(true));
        assert_eq!(ev.power_kw, Some(22.1));
        assert_eq!(ev.regen_power_kw, Some(-8.4));
        assert_eq!(ev.heading_deg, Some(182.0));
        assert_eq!(
            ev.tire_fl_psi.map(|v| (v * 10.0).round() / 10.0),
            Some(42.9)
        );
        assert_eq!(
            ev.tire_fr_psi.map(|v| (v * 10.0).round() / 10.0),
            Some(42.8)
        );
        assert_eq!(
            ev.tire_rl_psi.map(|v| (v * 10.0).round() / 10.0),
            Some(43.8)
        );
        assert_eq!(
            ev.tire_rr_psi.map(|v| (v * 10.0).round() / 10.0),
            Some(43.7)
        );
        assert_eq!(ev.tire_fl_status.as_deref(), Some("OK"));
        assert_eq!(ev.door_front_left_locked, Some(true));
        assert_eq!(ev.door_front_left_closed, Some(true));
        assert_eq!(ev.closure_frunk_closed, Some(false));
        assert_eq!(ev.ota_status.as_deref(), Some("up_to_date"));
        assert_eq!(ev.ota_current_version.as_deref(), Some("2024.11.02"));
    }

    #[test]
    fn parses_current_drive_mode_values() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "driveMode": { "timeStamp": "2024-01-15T10:30:00Z", "value": "everyday" }
            }}}
        })
        .to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.drive_mode, Some(DriveMode::AllPurpose));
    }

    #[test]
    fn parses_rivian_bearing_as_heading() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "gnssBearing": { "timeStamp": "2024-01-15T10:30:00Z", "value": 91.0 }
            }}}
        })
        .to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.heading_deg, Some(91.0));
    }

    #[test]
    fn parses_vehicle_mileage_as_miles() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "vehicleMileage": { "timeStamp": "2024-01-15T10:30:00Z", "value": 16093.44 }
            }}}
        })
        .to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.odometer_miles, Some(10.0));
    }

    #[test]
    fn parses_distance_to_empty_as_miles() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "distanceToEmpty": { "timeStamp": "2024-01-15T10:30:00Z", "value": 16.09344 }
            }}}
        })
        .to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.distance_to_empty_mi, Some(10.0));
    }

    #[test]
    fn parses_battery_capacity_as_wh() {
        let msg = serde_json::json!({
            "type": "next",
            "payload": { "data": { "vehicleState": {
                "batteryCapacity": { "timeStamp": "2024-01-15T10:30:00Z", "value": 111.2 }
            }}}
        })
        .to_string();

        let ev = parse_ws_message(&msg, vid()).unwrap().unwrap();
        assert_eq!(ev.battery_capacity_wh, Some(111_200.0));
    }

    #[test]
    fn garbage_json_errors() {
        assert!(parse_ws_message("not json at all", vid()).is_err());
    }

    #[test]
    fn empty_object_returns_none() {
        let msg = r#"{"type":"ping"}"#;
        assert!(parse_ws_message(msg, vid()).unwrap().is_none());
    }

    #[test]
    fn parses_charging_session_numbers_and_strings() {
        let msg = serde_json::json!({
        "type": "next",
        "payload": { "data": { "chargingSession": {
            "liveData": {
                "powerKW": "11.5",
                "kilometersChargedPerHour": 68.0,
                "rangeAddedThisSession": "42.25",
                "totalChargedEnergy": 7.5,
                "timeElapsed": "120",
                "timeRemaining": 360,
                "price": "1.25",
                "currency": "USD",
                "isFreeSession": "false",
                "vehicleChargerState": "Charging",
                "startTime": "2026-08-18T12:00:00Z"
            },
            "chartData": [{
                "soc": "55",
                "powerKW": 10,
                "startTime": "2026-08-18T12:00:00Z",
                "endTime": "2026-08-18T12:01:00Z",
                "timeEstimationValidityStatus": "VALID",
                "vehicleChargerState": "Charging"
            }]
        }}}})
        .to_string();
        let event = parse_charging_session_message(&msg).unwrap().unwrap();
        let live = event.live_data.unwrap();
        assert_eq!(live.power_kw, Some(11.5));
        assert_eq!(live.time_remaining_seconds, Some(360.0));
        assert_eq!(live.is_free_session, Some(false));
        assert_eq!(
            live.start_time.unwrap().to_rfc3339(),
            "2026-08-18T12:00:00+00:00"
        );
        assert_eq!(event.chart_data[0].soc, Some(55.0));
        assert_eq!(event.ts.to_rfc3339(), "2026-08-18T12:00:00+00:00");
    }

    #[test]
    fn parses_vehicle_state_envelopes_in_charging_session_values() {
        let msg = serde_json::json!({
        "type": "next",
        "payload": { "data": { "chargingSession": {
            "liveData": {
                "powerKW": { "value": 9.6, "updatedAt": "2026-08-18T12:00:00Z" },
                "totalChargedEnergy": { "value": "12.5" },
                "timeRemaining": { "value": 120 },
                "vehicleChargerState": { "value": "charging_active" },
                "startTime": { "value": "2026-08-18T11:00:00Z" }
            },
            "chartData": [{
                "soc": { "value": 66.0 },
                "powerKW": { "value": 9.5 },
                "startTime": { "value": "2026-08-18T11:59:00Z" }
            }]
        }}}})
        .to_string();
        let event = parse_charging_session_message(&msg).unwrap().unwrap();
        let live = event.live_data.unwrap();
        assert_eq!(live.power_kw, Some(9.6));
        assert_eq!(live.total_charged_energy, Some(12.5));
        assert_eq!(live.time_remaining_seconds, Some(120.0));
        assert_eq!(
            live.vehicle_charger_state.as_deref(),
            Some("charging_active")
        );
        assert_eq!(
            live.start_time.unwrap().to_rfc3339(),
            "2026-08-18T11:00:00+00:00"
        );
        assert_eq!(event.chart_data[0].soc, Some(66.0));
        assert_eq!(event.chart_data[0].power_kw, Some(9.5));
    }

    #[test]
    fn preserves_null_charging_session_and_no_active_object() {
        let null_msg = r#"{"type":"next","payload":{"data":{"chargingSession":null}}}"#;
        let null_event = parse_charging_session_message(null_msg).unwrap().unwrap();
        assert!(null_event.live_data.is_none());
        assert!(null_event.explicit_null);
        assert!(null_event.chart_data.is_empty());

        let empty_msg = r#"{"type":"next","payload":{"data":{"chargingSession":{"liveData":null,"chartData":[]}}}}"#;
        let event = parse_charging_session_message(empty_msg).unwrap().unwrap();
        assert!(event.live_data.is_none());
        assert!(event.live_data_present);
        assert!(!event.explicit_null);
        assert!(event.chart_data.is_empty());

        let chart_only =
            r#"{"type":"next","payload":{"data":{"chargingSession":{"chartData":[]}}}}"#;
        let event = parse_charging_session_message(chart_only).unwrap().unwrap();
        assert!(event.live_data.is_none());
        assert!(!event.live_data_present);
    }

    #[test]
    fn extracts_battery_cell_type_without_telemetry_field() {
        let msg = r#"{"type":"next","payload":{"data":{"vehicleState":{"batteryCellType":{"value":"4680"}}}}}"#;
        assert_eq!(
            parse_battery_cell_type(msg).unwrap().as_deref(),
            Some("4680")
        );
    }

    #[test]
    fn normalizes_only_ota_zero_sentinel() {
        for (value, expected) in [("0.0.0", None), ("2026.08.1", Some("2026.08.1"))] {
            let msg = serde_json::json!({
                "type": "next",
                "payload": { "data": { "vehicleState": {
                    "otaAvailableVersion": { "value": value }
                }}}
            })
            .to_string();
            assert_eq!(
                parse_ws_message(&msg, vid())
                    .unwrap()
                    .unwrap()
                    .ota_available_version
                    .as_deref(),
                expected
            );
        }
    }
}

#[cfg(test)]
mod provider_input_contract_tests {
    use super::*;
    use serde_json::json;

    const STAMP: &str = "2026-10-01T12:00:00Z";

    fn message(root: &str, value: Value) -> String {
        json!({"type": "next", "payload": {"data": {root: value}}}).to_string()
    }

    fn telemetry(state: Value) -> TelemetryEvent {
        parse_ws_message(&message("vehicleState", state), Uuid::nil())
            .unwrap()
            .unwrap()
    }

    fn session(value: Value) -> ChargingSessionEvent {
        parse_charging_session_message(&message("chargingSession", value))
            .unwrap()
            .unwrap()
    }

    fn assert_leaf(provider: &str, value: Value, output: &str, expected: Value) {
        let actual = telemetry(json!({
            "timeStamp": STAMP,
            provider: {"value": value}
        }));
        let mut empty =
            serde_json::to_value(TelemetryEvent::empty(Uuid::nil(), STAMP.parse().unwrap()))
                .unwrap();
        empty[output] = expected;
        assert_eq!(serde_json::to_value(actual).unwrap(), empty, "{provider}");
    }

    #[test]
    fn numeric_fields_and_units_leave_unrelated_fields_absent() {
        for (provider, output) in [
            ("gnssAltitude", "altitude_m"),
            ("batteryLevel", "battery_level"),
            ("batteryLimit", "battery_limit"),
            ("cabinClimateInteriorTemperature", "cabin_temp_c"),
            ("cabinClimateDriverTemperature", "driver_temp_c"),
            ("cabinClimateExteriorTemperature", "outside_temp_c"),
            ("vehiclePowerOutput", "power_kw"),
            ("regenerativeBrakingPower", "regen_power_kw"),
            ("gnssBearing", "heading_deg"),
            ("gnssHeading", "heading_deg"),
        ] {
            for value in [json!(-2.5), json!(" -2.5 "), json!({"value": "-2.5"})] {
                assert_leaf(provider, value, output, json!(-2.5));
            }
        }
        for (provider, input, output, expected) in [
            ("gnssSpeed", 10.0, "speed_mph", 22.3694),
            ("vehicleMileage", 1609.344, "odometer_miles", 1.0),
            ("distanceToEmpty", 1.609344, "distance_to_empty_mi", 1.0),
            ("batteryCapacity", 0.125, "battery_capacity_wh", 125.0),
            ("tirePressureFrontLeft", 1.0, "tire_fl_psi", 14.5037738),
            ("tirePressureFrontRight", 1.0, "tire_fr_psi", 14.5037738),
            ("tirePressureRearLeft", 1.0, "tire_rl_psi", 14.5037738),
            ("tirePressureRearRight", 1.0, "tire_rr_psi", 14.5037738),
        ] {
            let event =
                serde_json::to_value(telemetry(json!({provider: {"value": input}}))).unwrap();
            assert!((event[output].as_f64().unwrap() - expected).abs() < 1e-10);
            assert_leaf(provider, json!(0), output, json!(0.0));
        }
        let event = telemetry(json!({
            "gnssLocation": {"latitude": " 47.6 ", "longitude": {"value": -122.3}},
            "batteryLevel": 82
        }));
        assert_eq!(event.latitude, Some(47.6));
        assert_eq!(event.longitude, Some(-122.3));
        assert_eq!(event.battery_level, None);
    }

    #[test]
    fn numeric_and_integer_boundaries_do_not_coerce_other_shapes() {
        for invalid in [Value::Null, json!(true), json!([]), json!({}), json!("abc")] {
            assert_leaf("batteryLevel", invalid, "battery_level", Value::Null);
        }
        for (provider, output) in [
            ("timeToEndOfCharge", "time_to_end_of_charge_min"),
            ("steeringWheelHeat", "steering_wheel_heat"),
            ("seatFrontLeftHeat", "seat_fl_heat"),
            ("seatFrontRightHeat", "seat_fr_heat"),
            ("seatRearLeftHeat", "seat_rl_heat"),
            ("seatRearRightHeat", "seat_rr_heat"),
            ("seatFrontLeftVent", "seat_fl_vent"),
            ("seatFrontRightVent", "seat_fr_vent"),
        ] {
            for value in [i32::MIN, 0, i32::MAX] {
                assert_leaf(provider, json!(value), output, json!(value));
            }
            for value in [
                json!(i64::from(i32::MAX) + 1),
                json!(i64::from(i32::MIN) - 1),
                json!(u64::MAX),
                json!(1.0),
                json!("1"),
                json!({"value": 1}),
            ] {
                assert_leaf(provider, value, output, Value::Null);
            }
        }
        assert!(telemetry(json!({"batteryLevel": {"value": "NaN"}}))
            .battery_level
            .unwrap()
            .is_nan());
        assert_eq!(
            telemetry(json!({"batteryLevel": {"value": "1e999"}})).battery_level,
            Some(f64::INFINITY)
        );
    }

    #[test]
    fn open_text_and_unknown_enum_values_keep_their_contract() {
        for (provider, output) in [
            ("chargerStatus", "charger_status"),
            ("gearStatus", "gear_status"),
            ("tirePressureStatusFrontLeft", "tire_fl_status"),
            ("tirePressureStatusFrontRight", "tire_fr_status"),
            ("tirePressureStatusRearLeft", "tire_rl_status"),
            ("tirePressureStatusRearRight", "tire_rr_status"),
            ("otaCurrentVersion", "ota_current_version"),
            ("otaCurrentVersionGitHash", "ota_current_version"),
            ("otaAvailableVersion", "ota_available_version"),
            ("otaAvailableVersionGitHash", "ota_available_version"),
            ("otaStatus", "ota_status"),
            ("otaCurrentStatus", "ota_current_status"),
            ("batteryHvThermalEvent", "hv_thermal_event"),
            ("twelveVoltBatteryHealth", "twelve_volt_health"),
            ("cabinPreconditioningStatus", "cabin_precon_status"),
            ("cabinPreconditioningType", "cabin_precon_type"),
            ("gearGuardVideoStatus", "gear_guard_video_status"),
        ] {
            assert_leaf(
                provider,
                json!(" Future_STATE "),
                output,
                json!(" Future_STATE "),
            );
            assert_leaf(provider, json!({"value": "nested"}), output, Value::Null);
        }
        for (provider, output) in [
            ("powerState", "power_state"),
            ("chargerState", "charger_state"),
            ("driveMode", "drive_mode"),
        ] {
            assert_leaf(provider, json!("future"), output, json!("unknown"));
            assert_leaf(provider, json!(3), output, Value::Null);
        }
        assert_leaf("powerState", json!("READY"), "power_state", json!("ready"));
        assert_leaf(
            "powerState",
            json!(" ready "),
            "power_state",
            json!("unknown"),
        );
    }

    #[test]
    fn strict_booleans_and_closed_vocabularies_remain_distinct() {
        for (provider, output) in [
            ("cabinClimateRunning", "hvac_active"),
            ("gearGuardLocked", "gear_guard_locked"),
            ("brakeFluidLow", "brake_fluid_low"),
        ] {
            assert_leaf(provider, json!(false), output, json!(false));
            assert_leaf(provider, json!("true"), output, Value::Null);
        }
        assert_eq!(
            telemetry(json!({"cloudConnection": {"isOnline": false}})).is_online,
            Some(false)
        );
        assert_eq!(
            telemetry(json!({"cloudConnection": {"isOnline": "true"}})).is_online,
            None
        );
        for (provider, output, yes, no) in [
            (
                "doorFrontLeftLocked",
                "door_front_left_locked",
                "LOCKED",
                "unlocked",
            ),
            (
                "doorFrontRightLocked",
                "door_front_right_locked",
                "true",
                "false",
            ),
            (
                "doorRearLeftLocked",
                "door_rear_left_locked",
                "locked",
                "unlocked",
            ),
            (
                "doorRearRightLocked",
                "door_rear_right_locked",
                "locked",
                "unlocked",
            ),
            (
                "closureFrunkLocked",
                "closure_frunk_locked",
                "locked",
                "unlocked",
            ),
            (
                "closureLiftgateLocked",
                "closure_liftgate_locked",
                "locked",
                "unlocked",
            ),
            (
                "closureTailgateLocked",
                "closure_tailgate_locked",
                "locked",
                "unlocked",
            ),
            (
                "closureTonneauLocked",
                "tonneau_locked",
                "locked",
                "unlocked",
            ),
            (
                "closureSideBinLeftLocked",
                "side_bin_left_locked",
                "locked",
                "unlocked",
            ),
            (
                "closureSideBinRightLocked",
                "side_bin_right_locked",
                "locked",
                "unlocked",
            ),
            (
                "doorFrontLeftClosed",
                "door_front_left_closed",
                "CLOSED",
                "opened",
            ),
            (
                "doorFrontRightClosed",
                "door_front_right_closed",
                "true",
                "false",
            ),
            (
                "doorRearLeftClosed",
                "door_rear_left_closed",
                "closed",
                "open",
            ),
            (
                "doorRearRightClosed",
                "door_rear_right_closed",
                "closed",
                "open",
            ),
            (
                "closureFrunkClosed",
                "closure_frunk_closed",
                "closed",
                "open",
            ),
            (
                "closureLiftgateClosed",
                "closure_liftgate_closed",
                "closed",
                "open",
            ),
            (
                "closureTailgateClosed",
                "closure_tailgate_closed",
                "closed",
                "open",
            ),
            ("closureTonneauClosed", "tonneau_closed", "closed", "open"),
            (
                "closureSideBinLeftClosed",
                "side_bin_left_closed",
                "closed",
                "open",
            ),
            (
                "closureSideBinRightClosed",
                "side_bin_right_closed",
                "closed",
                "open",
            ),
            (
                "windowFrontLeftClosed",
                "window_fl_closed",
                "closed",
                "open",
            ),
            (
                "windowFrontRightClosed",
                "window_fr_closed",
                "closed",
                "open",
            ),
            ("windowRearLeftClosed", "window_rl_closed", "closed", "open"),
            (
                "windowRearRightClosed",
                "window_rr_closed",
                "closed",
                "open",
            ),
            (
                "tirePressureStatusValidFrontLeft",
                "tire_fl_valid",
                "YES",
                "bad",
            ),
            (
                "tirePressureStatusValidFrontRight",
                "tire_fr_valid",
                "ok",
                "no",
            ),
            (
                "tirePressureStatusValidRearLeft",
                "tire_rl_valid",
                "valid",
                "invalid",
            ),
            (
                "tirePressureStatusValidRearRight",
                "tire_rr_valid",
                "true",
                "false",
            ),
        ] {
            assert_leaf(provider, json!(yes), output, json!(true));
            assert_leaf(provider, json!(no), output, json!(false));
            assert_leaf(provider, json!(true), output, json!(true));
            assert_leaf(provider, json!("unknown"), output, Value::Null);
            assert_leaf(provider, json!(format!(" {yes} ")), output, Value::Null);
        }
    }

    #[test]
    fn string_flags_preserve_unknown_and_non_string_decisions() {
        for (provider, output, active) in [
            ("chargePortState", "charge_port_open", "AJAR"),
            ("petModeStatus", "pet_mode_active", "ACTIVE"),
            ("petModeTemperatureStatus", "pet_mode_temp_ok", "SAFE"),
            ("defrostDefogStatus", "defrost_active", "ON"),
            ("wiperFluidState", "wiper_fluid_low", "CRITICAL"),
            ("alarmSoundStatus", "alarm_active", "SOUNDING"),
            ("serviceMode", "service_mode", "TRUE"),
            ("vehicleInServiceMode", "service_mode", "ACTIVE"),
        ] {
            assert_leaf(provider, json!(active), output, json!(true));
            assert_leaf(provider, json!("future"), output, json!(false));
            assert_leaf(provider, json!(true), output, Value::Null);
        }
        assert_leaf(
            "chargerDerateStatus",
            json!("future"),
            "charger_derate_active",
            json!(true),
        );
        for inactive in ["INACTIVE", "none", "off"] {
            assert_leaf(
                "chargerDerateStatus",
                json!(inactive),
                "charger_derate_active",
                json!(false),
            );
        }
        assert_leaf(
            "chargerDerateStatus",
            json!(false),
            "charger_derate_active",
            json!(false),
        );
    }

    #[test]
    fn aliases_choose_first_valid_value_before_sentinel_filtering() {
        let ev = telemetry(json!({
            "gnssBearing": {"value": "bad"}, "gnssHeading": {"value": "91"},
            "otaCurrentVersion": {"value": ""}, "otaCurrentVersionGitHash": {"value": "fallback"},
            "otaAvailableVersion": {"value": " 0.0.0 "}, "otaAvailableVersionGitHash": {"value": "fallback"},
            "serviceMode": {"value": "future"}, "vehicleInServiceMode": {"value": "active"}
        }));
        assert_eq!(ev.heading_deg, Some(91.0));
        assert_eq!(ev.ota_current_version.as_deref(), Some(""));
        assert_eq!(ev.ota_available_version, None);
        assert_eq!(ev.service_mode, Some(false));
        assert_eq!(
            telemetry(json!({
                "otaAvailableVersion": {"value": false},
                "otaAvailableVersionGitHash": {"value": " version "}
            }))
            .ota_available_version
            .as_deref(),
            Some(" version ")
        );
    }

    #[test]
    fn field_timestamps_survive_bad_values_and_unknown_nested_fields_set_event_time() {
        for (provider, output) in [
            ("gnssLocation", "location_ts"),
            ("gnssSpeed", "speed_mph_ts"),
            ("batteryLevel", "battery_level_ts"),
            ("distanceToEmpty", "distance_to_empty_mi_ts"),
            ("batteryLimit", "battery_limit_ts"),
            ("powerState", "power_state_ts"),
            ("chargerState", "charger_state_ts"),
            ("chargerStatus", "charger_status_ts"),
            ("timeToEndOfCharge", "time_to_end_of_charge_min_ts"),
            ("vehicleMileage", "odometer_miles_ts"),
        ] {
            let ev = telemetry(json!({
                provider: {"value": null, "timeStamp": STAMP},
                "unknown": [{"timeStamp": "bad"}, {"nested": {"timeStamp": "2026-10-02T09:00:00-03:00"}}],
                "updatedAt": "2099-01-01T00:00:00Z"
            }));
            let value = serde_json::to_value(ev).unwrap();
            assert_eq!(value[output], json!("2026-10-01T12:00:00Z"), "{provider}");
            assert_eq!(value["ts"], json!("2026-10-02T12:00:00Z"));
        }
        let before = Utc::now();
        let ev = telemetry(json!({"batteryLevel": {"value": 1, "timeStamp": 123}}));
        assert!(ev.ts >= before && ev.ts <= Utc::now());
        assert_eq!(ev.battery_level_ts, None);
    }

    #[test]
    fn envelope_errors_missing_and_malformed_payloads_stay_separate() {
        for raw in ["", "{", r#"{"type":"next","payload":}"#] {
            assert!(matches!(
                parse_ws_message(raw, Uuid::nil()),
                Err(ParseError::InvalidJson(_))
            ));
            assert!(matches!(
                parse_charging_session_message(raw),
                Err(ParseError::InvalidJson(_))
            ));
            assert!(matches!(
                parse_battery_cell_type(raw),
                Err(ParseError::InvalidJson(_))
            ));
        }
        for raw in [
            "null",
            "[]",
            "17",
            "{}",
            r#"{"type":"NEXT"}"#,
            r#"{"type":"complete"}"#,
        ] {
            assert!(parse_ws_message(raw, Uuid::nil()).unwrap().is_none());
            assert!(parse_charging_session_message(raw).unwrap().is_none());
            assert!(parse_battery_cell_type(raw).unwrap().is_none());
        }
        for raw in [
            r#"{"type":"next"}"#,
            r#"{"type":"next","payload":{"data":null}}"#,
        ] {
            assert!(matches!(
                parse_ws_message(raw, Uuid::nil()),
                Err(ParseError::MissingVehicleState)
            ));
            assert!(parse_charging_session_message(raw).unwrap().is_none());
            assert!(parse_battery_cell_type(raw).unwrap().is_none());
        }
        for value in [Value::Null, json!([]), json!(false), json!("unexpected")] {
            let ev = telemetry(value);
            let empty = TelemetryEvent::empty(ev.vehicle_id, ev.ts);
            assert_eq!(
                serde_json::to_value(ev).unwrap(),
                serde_json::to_value(empty).unwrap()
            );
        }
    }

    #[test]
    fn duplicate_fields_and_repeated_messages_are_not_deduplicated() {
        let raw = r#"{"type":"next","payload":{"data":{"vehicleState":{"batteryLevel":{"value":10},"batteryLevel":{"value":20,"timeStamp":"2026-10-01T12:00:00Z"}}}}}"#;
        let first = parse_ws_message(raw, Uuid::nil()).unwrap().unwrap();
        let second = parse_ws_message(raw, Uuid::nil()).unwrap().unwrap();
        assert_eq!(first.battery_level, Some(20.0));
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(second).unwrap()
        );
        let raw = r#"{"type":"next","payload":{"data":{"vehicleState":{"batteryLevel":{"value":10,"value":30}}}}}"#;
        assert_eq!(
            parse_ws_message(raw, Uuid::nil())
                .unwrap()
                .unwrap()
                .battery_level,
            Some(30.0)
        );
    }

    #[test]
    fn charging_presence_is_not_the_same_as_a_nonempty_patch() {
        let missing = session(json!({}));
        assert!(
            !missing.live_data_present && !missing.explicit_null && missing.live_data.is_none()
        );
        let null = session(Value::Null);
        assert!(null.live_data_present && null.explicit_null && null.live_data.is_none());
        let null_leaf = session(json!({"liveData": null}));
        assert!(
            null_leaf.live_data_present
                && !null_leaf.explicit_null
                && null_leaf.live_data.is_none()
        );
        for value in [json!({}), json!({"powerKW": null}), json!(false), json!([])] {
            let event = session(json!({"liveData": value}));
            assert!(event.live_data_present && !event.explicit_null);
            let live = event.live_data.unwrap();
            assert_eq!(live.power_kw, None);
            assert_eq!(live.start_time, None);
        }
        // The value-level API is used without subscription-root null semantics.
        let direct = parse_charging_session_value(&Value::Null);
        assert!(!direct.explicit_null && !direct.live_data_present);
    }

    #[test]
    fn charging_all_fields_accept_one_envelope_and_preserve_text() {
        let event = session(json!({"liveData": {
            "powerKW": {"value": " 10.5 "},
            "kilometersChargedPerHour": {"value": "-1"},
            "rangeAddedThisSession": {"value": 2.5},
            "totalChargedEnergy": {"value": "3"},
            "timeElapsed": {"value": 4},
            "timeRemaining": {"value": "5"},
            "price": {"value": "-0.25"},
            "currency": {"value": " future "},
            "isFreeSession": {"value": "FaLsE"},
            "vehicleChargerState": {"value": "future-state"},
            "startTime": {"value": "bad", "updatedAt": STAMP}
        }}));
        assert_eq!(
            event.live_data,
            Some(ChargingSessionLiveData {
                power_kw: Some(10.5),
                kilometers_charged_per_hour: Some(-1.0),
                range_added_this_session: Some(2.5),
                total_charged_energy: Some(3.0),
                time_elapsed_seconds: Some(4.0),
                time_remaining_seconds: Some(5.0),
                price: Some(-0.25),
                currency: Some(" future ".into()),
                is_free_session: Some(false),
                vehicle_charger_state: Some("future-state".into()),
                start_time: Some(STAMP.parse().unwrap()),
            })
        );
        assert_eq!(event.ts, STAMP.parse::<DateTime<Utc>>().unwrap());
        for value in [
            json!(" true "),
            json!("yes"),
            json!(1),
            json!({"value": {"value": true}}),
        ] {
            assert_eq!(
                session(json!({"liveData": {"isFreeSession": value}}))
                    .live_data
                    .unwrap()
                    .is_free_session,
                None
            );
        }
    }

    #[test]
    fn chart_order_duplicates_invalid_points_and_timestamp_priority_are_preserved() {
        let point = json!({
            "soc": " 55 ", "powerKW": {"value": 7},
            "startTime": {"value": STAMP, "updatedAt": "2099-01-01T00:00:00Z"},
            "endTime": {"updatedAt": "2026-10-01T12:01:00Z"},
            "timeEstimationValidityStatus": "UNKNOWN",
            "vehicleChargerState": {"value": "future"}
        });
        let event = session(json!({"chartData": [point, point, null]}));
        assert_eq!(event.chart_data.len(), 3);
        assert_eq!(event.chart_data[0], event.chart_data[1]);
        assert_eq!(event.chart_data[0].soc, Some(55.0));
        assert_eq!(event.chart_data[0].power_kw, Some(7.0));
        assert_eq!(
            event.chart_data[0].end_time,
            Some("2026-10-01T12:01:00Z".parse().unwrap())
        );
        assert_eq!(
            event.chart_data[0]
                .time_estimation_validity_status
                .as_deref(),
            Some("UNKNOWN")
        );
        assert_eq!(
            event.chart_data[0].vehicle_charger_state.as_deref(),
            Some("future")
        );
        assert_eq!(event.chart_data[2].soc, None);
        assert_eq!(event.ts, STAMP.parse::<DateTime<Utc>>().unwrap());
        let live_first = session(json!({
            "liveData": {"startTime": "2026-09-01T00:00:00Z"}, "chartData": [point]
        }));
        assert_eq!(
            live_first.ts,
            "2026-09-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        let before = Utc::now();
        let invalid_first = session(json!({"chartData": [false, point]}));
        assert!(invalid_first.ts >= before && invalid_first.ts <= Utc::now());
        for bad in [Value::Null, json!({}), json!("not an array")] {
            assert!(session(json!({"chartData": bad})).chart_data.is_empty());
        }
    }

    #[test]
    fn battery_cell_type_accepts_only_the_existing_string_shapes() {
        for value in [json!("future-cell"), json!({"value": "future-cell"})] {
            assert_eq!(
                parse_battery_cell_type(&message(
                    "vehicleState",
                    json!({
                        "batteryCellType": {"value": value}
                    })
                ))
                .unwrap()
                .as_deref(),
                Some("future-cell")
            );
        }
        for value in [
            Value::Null,
            json!(4680),
            json!({"value": {"value": "4680"}}),
        ] {
            assert_eq!(
                parse_battery_cell_type(&message(
                    "vehicleState",
                    json!({
                        "batteryCellType": {"value": value}
                    })
                ))
                .unwrap(),
                None
            );
        }
    }
}

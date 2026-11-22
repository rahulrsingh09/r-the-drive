//! Replay stored telemetry rows through the live trip-signal fusion and
//! detector. Shared by the history rebuild and missed-trip recovery.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::models::telemetry::{ChargerState, DriveMode, PowerState, TelemetryEvent};

/// Columns selected from `timeseries.telemetry` for a replay row.
pub const REPLAY_TELEMETRY_COLUMNS: &str = "ts, vehicle_id, \
    latitude, longitude, altitude_m, speed_mph, \
    battery_level, battery_capacity_wh, distance_to_empty_mi, battery_limit, \
    power_state, charger_state, charger_status, time_to_end_of_charge_min, \
    drive_mode, gear_status, cabin_temp_c, outside_temp_c, \
    power_kw, regen_power_kw, heading_deg, odometer_miles, is_online";

#[derive(Debug, FromRow)]
pub struct ReplayTelemetryRow {
    pub ts: DateTime<Utc>,
    pub vehicle_id: Uuid,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f64>,
    pub speed_mph: Option<f64>,
    pub battery_level: Option<f64>,
    pub battery_capacity_wh: Option<f64>,
    pub distance_to_empty_mi: Option<f64>,
    pub battery_limit: Option<f64>,
    pub power_state: Option<String>,
    pub charger_state: Option<String>,
    pub charger_status: Option<String>,
    pub time_to_end_of_charge_min: Option<i32>,
    pub drive_mode: Option<String>,
    pub gear_status: Option<String>,
    pub cabin_temp_c: Option<f64>,
    pub outside_temp_c: Option<f64>,
    pub power_kw: Option<f64>,
    pub regen_power_kw: Option<f64>,
    pub heading_deg: Option<f64>,
    pub odometer_miles: Option<f64>,
    pub is_online: Option<bool>,
}

pub fn row_to_event(row: ReplayTelemetryRow) -> TelemetryEvent {
    let mut event = TelemetryEvent::empty(row.vehicle_id, row.ts);
    event.latitude = row.latitude;
    event.longitude = row.longitude;
    event.altitude_m = row.altitude_m;
    event.speed_mph = row.speed_mph;
    event.battery_level = row.battery_level;
    event.battery_capacity_wh = row.battery_capacity_wh;
    event.distance_to_empty_mi = row.distance_to_empty_mi;
    event.battery_limit = row.battery_limit;
    event.power_state = row.power_state.as_deref().and_then(parse_power_state);
    event.charger_state = row.charger_state.as_deref().and_then(parse_charger_state);
    event.charger_status = row.charger_status;
    event.time_to_end_of_charge_min = row.time_to_end_of_charge_min;
    event.drive_mode = row.drive_mode.as_deref().and_then(parse_drive_mode);
    event.gear_status = row.gear_status;
    event.cabin_temp_c = row.cabin_temp_c;
    event.outside_temp_c = row.outside_temp_c;
    event.power_kw = row.power_kw;
    event.regen_power_kw = row.regen_power_kw;
    event.heading_deg = row.heading_deg;
    event.odometer_miles = row.odometer_miles;
    event.is_online = row.is_online;
    event
}

/// Historical rows lack source provenance. Power-only rows match the typed
/// Parallax power frame; richer telemetry uses periodic freshness. A legacy
/// power-only frame remains indistinguishable, so this is an inference only.
pub fn replay_power_is_change_only(event: &TelemetryEvent) -> bool {
    event.power_state.is_some()
        && event.latitude.is_none()
        && event.longitude.is_none()
        && event.altitude_m.is_none()
        && event.speed_mph.is_none()
        && event.battery_level.is_none()
        && event.battery_capacity_wh.is_none()
        && event.distance_to_empty_mi.is_none()
        && event.battery_limit.is_none()
        && event.charger_state.is_none()
        && event.charger_status.is_none()
        && event.time_to_end_of_charge_min.is_none()
        && event.drive_mode.is_none()
        && event.gear_status.is_none()
        && event.cabin_temp_c.is_none()
        && event.outside_temp_c.is_none()
        && event.power_kw.is_none()
        && event.regen_power_kw.is_none()
        && event.heading_deg.is_none()
        && event.odometer_miles.is_none()
        && event.is_online.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::trip_signals::TripSignalFusion;
    use chrono::Duration;

    #[test]
    fn mixed_history_keeps_the_supplying_power_rows_policy() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::default();
        let mut power = TelemetryEvent::empty(Uuid::nil(), at);
        power.power_state = Some(PowerState::Go);
        assert!(replay_power_is_change_only(&power));
        fusion.fuse_from(&power, replay_power_is_change_only(&power));
        let mut direct_speed = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(30));
        direct_speed.speed_mph = Some(0.0);
        fusion.fuse_from(&direct_speed, replay_power_is_change_only(&direct_speed));
        let later = TelemetryEvent::empty(Uuid::nil(), at + Duration::minutes(20));
        assert_eq!(
            fusion.fuse_from(&later, false).0.power_state,
            Some(PowerState::Go)
        );
        power.ts = later.ts;
        power.is_online = Some(true);
        assert!(!replay_power_is_change_only(&power));
        fusion.fuse_from(&power, replay_power_is_change_only(&power));
        let after = TelemetryEvent::empty(Uuid::nil(), power.ts + Duration::minutes(3));
        assert_eq!(fusion.fuse_from(&after, false).0.power_state, None);
        power.ts = after.ts;
        power.is_online = None;
        fusion.fuse_from(&power, replay_power_is_change_only(&power));
        let after = TelemetryEvent::empty(Uuid::nil(), power.ts + Duration::minutes(20));
        assert_eq!(
            fusion.fuse_from(&after, false).0.power_state,
            Some(PowerState::Go)
        );
    }
}

fn parse_power_state(value: &str) -> Option<PowerState> {
    value.parse().ok()
}

fn parse_charger_state(value: &str) -> Option<ChargerState> {
    value.parse().ok()
}

fn parse_drive_mode(value: &str) -> Option<DriveMode> {
    value.parse().ok()
}

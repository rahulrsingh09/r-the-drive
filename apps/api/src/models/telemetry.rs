use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Raw telemetry event as received from the Rivian WebSocket.
/// All fields are Option because Rivian sends partial updates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub vehicle_id: Uuid,
    pub ts: DateTime<Utc>,

    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f64>,
    pub speed_mph: Option<f64>,
    pub location_ts: Option<DateTime<Utc>>,
    pub speed_mph_ts: Option<DateTime<Utc>>,

    pub battery_level: Option<f64>,
    pub battery_capacity_wh: Option<f64>,
    pub distance_to_empty_mi: Option<f64>,
    pub battery_limit: Option<f64>,
    pub battery_level_ts: Option<DateTime<Utc>>,
    pub distance_to_empty_mi_ts: Option<DateTime<Utc>>,
    pub battery_limit_ts: Option<DateTime<Utc>>,

    pub power_state: Option<PowerState>,
    pub charger_state: Option<ChargerState>,
    pub charger_status: Option<String>,
    pub time_to_end_of_charge_min: Option<i32>,
    pub power_state_ts: Option<DateTime<Utc>>,
    pub charger_state_ts: Option<DateTime<Utc>>,
    pub charger_status_ts: Option<DateTime<Utc>>,
    pub time_to_end_of_charge_min_ts: Option<DateTime<Utc>>,
    pub drive_mode: Option<DriveMode>,
    pub gear_status: Option<String>,

    pub cabin_temp_c: Option<f64>,
    pub driver_temp_c: Option<f64>,
    pub outside_temp_c: Option<f64>,
    pub hvac_active: Option<bool>,

    pub power_kw: Option<f64>,
    pub regen_power_kw: Option<f64>,

    pub heading_deg: Option<f64>,
    pub odometer_miles: Option<f64>,
    pub odometer_miles_ts: Option<DateTime<Utc>>,

    pub tire_fl_psi: Option<f64>,
    pub tire_fr_psi: Option<f64>,
    pub tire_rl_psi: Option<f64>,
    pub tire_rr_psi: Option<f64>,
    pub tire_fl_status: Option<String>,
    pub tire_fr_status: Option<String>,
    pub tire_rl_status: Option<String>,
    pub tire_rr_status: Option<String>,
    pub tire_fl_valid: Option<bool>,
    pub tire_fr_valid: Option<bool>,
    pub tire_rl_valid: Option<bool>,
    pub tire_rr_valid: Option<bool>,

    pub door_front_left_locked: Option<bool>,
    pub door_front_right_locked: Option<bool>,
    pub door_rear_left_locked: Option<bool>,
    pub door_rear_right_locked: Option<bool>,
    pub door_front_left_closed: Option<bool>,
    pub door_front_right_closed: Option<bool>,
    pub door_rear_left_closed: Option<bool>,
    pub door_rear_right_closed: Option<bool>,
    pub closure_frunk_locked: Option<bool>,
    pub closure_frunk_closed: Option<bool>,
    pub closure_liftgate_locked: Option<bool>,
    pub closure_liftgate_closed: Option<bool>,
    pub closure_tailgate_locked: Option<bool>,
    pub closure_tailgate_closed: Option<bool>,

    pub ota_current_version: Option<String>,
    pub ota_available_version: Option<String>,
    pub ota_status: Option<String>,
    pub ota_current_status: Option<String>,

    pub hv_thermal_event: Option<String>,
    pub twelve_volt_health: Option<String>,
    pub is_online: Option<bool>,

    // Extended vehicle state fields (added to WS subscription)
    pub charge_port_open: Option<bool>,
    pub charger_derate_active: Option<bool>,
    pub cabin_precon_status: Option<String>,
    pub cabin_precon_type: Option<String>,
    pub pet_mode_active: Option<bool>,
    pub pet_mode_temp_ok: Option<bool>,
    pub defrost_active: Option<bool>,
    pub steering_wheel_heat: Option<i32>,
    pub seat_fl_heat: Option<i32>,
    pub seat_fr_heat: Option<i32>,
    pub seat_rl_heat: Option<i32>,
    pub seat_rr_heat: Option<i32>,
    pub seat_fl_vent: Option<i32>,
    pub seat_fr_vent: Option<i32>,
    pub tonneau_locked: Option<bool>,
    pub tonneau_closed: Option<bool>,
    pub side_bin_left_locked: Option<bool>,
    pub side_bin_right_locked: Option<bool>,
    pub side_bin_left_closed: Option<bool>,
    pub side_bin_right_closed: Option<bool>,
    pub window_fl_closed: Option<bool>,
    pub window_fr_closed: Option<bool>,
    pub window_rl_closed: Option<bool>,
    pub window_rr_closed: Option<bool>,
    pub gear_guard_locked: Option<bool>,
    pub gear_guard_video_status: Option<String>,
    pub wiper_fluid_low: Option<bool>,
    pub brake_fluid_low: Option<bool>,
    pub alarm_active: Option<bool>,
    pub service_mode: Option<bool>,
    /// Closures caught mid-movement in this sample, keyed by their
    /// `*_closed` field. Live-only: the worker turns these into an
    /// opening/closing state for the status stream; they are never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_transitions: Option<BTreeMap<String, ClosureTransition>>,
}

impl TelemetryEvent {
    /// Returns true when any charging telemetry signal identifies an active
    /// session. Rivian can report charger status before power_state changes.
    pub fn is_actively_charging(&self) -> bool {
        matches!(self.power_state, Some(PowerState::Charging))
            || matches!(self.charger_state, Some(ChargerState::Charging))
            || self.charger_status.as_deref() == Some("chrgr_sts_connected_charging")
    }

    pub fn empty(vehicle_id: Uuid, ts: DateTime<Utc>) -> Self {
        Self {
            vehicle_id,
            ts,
            ..Self::default()
        }
    }
}

/// Rivian's transitional closure statuses (app enum 3, 4 and 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClosureTransition {
    /// Neither latched nor at rest; the direction comes from the last
    /// settled state.
    Ajar,
    Opening,
    Closing,
}

macro_rules! telemetry_vocabulary {
    ($name:ident, $rename:literal, { $($variant:ident => $canonical:literal $(| $alias:literal)*),+ $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
        #[sqlx(type_name = "text")]
        #[serde(rename_all = $rename)]
        pub enum $name { $($variant),+ }

        impl $name {
            const WORDS: &'static [(&'static str, &'static [&'static str], Self)] = &[
                $(($canonical, &[$($alias),*], Self::$variant)),+
            ];

            pub fn as_str(&self) -> &'static str {
                Self::WORDS.iter().find(|(_, _, state)| state == self)
                    .map_or("unknown", |(canonical, _, _)| *canonical)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ();
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let normalized = text.to_lowercase();
                Ok(Self::WORDS.iter()
                    .find(|(canonical, aliases, _)| *canonical == normalized || aliases.contains(&normalized.as_str()))
                    .map_or(Self::Unknown, |(_, _, state)| state.clone()))
            }
        }
    };
}

telemetry_vocabulary!(PowerState, "lowercase", {
    Sleep => "sleep", Ready => "ready", Go => "go", Drive => "drive",
    Charging => "charging", Unknown => "unknown",
});

telemetry_vocabulary!(ChargerState, "lowercase", {
    Disconnected => "disconnected", Connected => "connected",
    Charging => "charging" | "charging_active", Done => "done" | "charging_done",
    Unknown => "unknown",
});

#[cfg(test)]
mod tests {
    use super::{ChargerState, PowerState, TelemetryEvent};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn charging_predicate_accepts_each_live_signal() {
        let vehicle_id = Uuid::new_v4();
        let ts = Utc::now();

        let mut event = TelemetryEvent::empty(vehicle_id, ts);
        event.power_state = Some(PowerState::Charging);
        assert!(event.is_actively_charging());

        let mut event = TelemetryEvent::empty(vehicle_id, ts);
        event.charger_state = Some(ChargerState::Charging);
        assert!(event.is_actively_charging());

        let mut event = TelemetryEvent::empty(vehicle_id, ts);
        event.charger_status = Some("chrgr_sts_connected_charging".to_string());
        assert!(event.is_actively_charging());
    }

    #[test]
    fn charging_predicate_rejects_disconnected_status() {
        let mut event = TelemetryEvent::empty(Uuid::new_v4(), Utc::now());
        event.charger_status = Some("chrgr_sts_not_connected".to_string());
        event.charger_state = Some(ChargerState::Disconnected);
        assert!(!event.is_actively_charging());
    }
}

telemetry_vocabulary!(DriveMode, "snake_case", {
    Sport => "sport", AllPurpose => "all_purpose" | "everyday" | "normal",
    Conserve => "conserve" | "distance", Snow => "snow" | "winter",
    AllTerrain => "all_terrain" | "off_road" | "off_road_auto",
    SoftSand => "soft_sand" | "off_road_sand", RockCrawl => "rock_crawl" | "off_road_rocks",
    Rally => "rally" | "off_road_sport_auto", Drift => "drift" | "off_road_sport_drift",
    Towing => "towing" | "tow", Unknown => "unknown",
});

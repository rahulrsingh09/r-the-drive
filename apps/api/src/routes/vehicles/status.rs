//! Vehicle status snapshots, freshness and per-field availability decisions.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

use super::{
    normalize_remaining_range_miles, open_closures_json, rivian_renewal_state_for_auth, AppError,
    LatestVehicleTelemetry, RivianRenewalState, Uuid, VehicleRuntimeStateRow,
    VehicleStatusFieldSeenRow, VehicleStatusResponse, RIVIAN_RENEWAL_INTERVAL,
};

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StatusFieldAvailabilityState {
    Current,
    Historical,
    NeverSeen,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StatusFieldAvailabilityReasonCode {
    MissingRecentPayload,
    NeverSeen,
    InvalidSensor,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub(super) struct StatusFieldAvailability {
    pub(super) ever_seen: bool,
    pub(super) last_seen_at: Option<DateTime<Utc>>,
    pub(super) latest_event_at: Option<DateTime<Utc>>,
    pub(super) availability: StatusFieldAvailabilityState,
    pub(super) reason_code: Option<StatusFieldAvailabilityReasonCode>,
}

pub(super) fn timestamp_is_older_than(
    now: DateTime<Utc>,
    ts: Option<DateTime<Utc>>,
    threshold: Duration,
) -> bool {
    ts.is_some_and(|value| now - value >= threshold)
}

pub(super) fn classify_status_field_availability(
    latest_event_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    reason_override: Option<StatusFieldAvailabilityReasonCode>,
) -> StatusFieldAvailability {
    use StatusFieldAvailabilityReasonCode::{MissingRecentPayload, NeverSeen};
    let (availability, reason_code) = if last_seen_at.is_none() {
        (StatusFieldAvailabilityState::NeverSeen, Some(NeverSeen))
    } else if last_seen_at
        .zip(latest_event_at)
        .is_some_and(|(seen, latest)| seen < latest)
    {
        (
            StatusFieldAvailabilityState::Historical,
            reason_override.or(Some(MissingRecentPayload)),
        )
    } else {
        (StatusFieldAvailabilityState::Current, reason_override)
    };
    StatusFieldAvailability {
        ever_seen: last_seen_at.is_some(),
        last_seen_at,
        latest_event_at,
        availability,
        reason_code,
    }
}

pub(super) fn max_seen_at(
    values: impl IntoIterator<Item = Option<DateTime<Utc>>>,
) -> Option<DateTime<Utc>> {
    values.into_iter().flatten().max()
}

pub(super) fn insert_field_availability(
    availability: &mut BTreeMap<String, StatusFieldAvailability>,
    field: &str,
    latest_event_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    reason_override: Option<StatusFieldAvailabilityReasonCode>,
) {
    availability.insert(
        field.to_string(),
        classify_status_field_availability(latest_event_at, last_seen_at, reason_override),
    );
}

pub(super) fn derive_vehicle_status_freshness(
    now: DateTime<Utc>,
    runtime: Option<&VehicleRuntimeStateRow>,
    latest: &LatestVehicleTelemetry,
) -> (Option<String>, bool, Option<String>) {
    let charging = latest.charger_state.as_deref() == Some("charging")
        || latest
            .charger_status
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("chrgr_sts_connected_charging"))
        || latest.time_to_end_of_charge_min.is_some();
    let checks = [
        (
            true,
            runtime.and_then(|r| r.last_ws_received_at),
            10,
            "ws_silent",
        ),
        (
            latest.battery_level.is_some(),
            latest.battery_level_ts.or(latest.ts),
            90,
            "battery_stale",
        ),
        (
            latest.distance_to_empty_mi.is_some(),
            latest.distance_to_empty_mi_ts.or(latest.ts),
            90,
            "range_stale",
        ),
        (
            charging,
            latest
                .charger_status_ts
                .or(latest.charger_state_ts)
                .or(latest.time_to_end_of_charge_min_ts)
                .or(latest.ts),
            45,
            "charging_stale",
        ),
    ];
    let reason = checks
        .into_iter()
        .find_map(|(active, observed, minutes, reason)| {
            (active && timestamp_is_older_than(now, observed, Duration::minutes(minutes)))
                .then_some(reason)
        });
    let health = match reason {
        Some("ws_silent") => Some("stale".to_owned()),
        _ => runtime.and_then(|r| r.worker_health.clone()),
    };
    (health, reason.is_some(), reason.map(str::to_owned))
}

pub(super) struct Snapshot {
    capacity: Option<f64>,
    model: Option<String>,
    runtime: Option<VehicleRuntimeStateRow>,
    latest: LatestVehicleTelemetry,
    seen: VehicleStatusFieldSeenRow,
    issued: Option<DateTime<Utc>>,
    renewal: Option<RivianRenewalState>,
}

impl Snapshot {
    pub(super) async fn read(pool: &sqlx::PgPool, vehicle_id: Uuid) -> Result<Self, AppError> {
        let vehicle = sqlx::query_as::<_, (Option<f64>, String)>(
            "SELECT battery_capacity_wh, model FROM panda.vehicles WHERE id = $1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?;
        let runtime = sqlx::query_as::<_, VehicleRuntimeStateRow>("SELECT is_online, last_event_at, last_payload_at, last_heartbeat_at, \
                last_ws_received_at, last_ws_payload_received_at, last_ws_heartbeat_received_at, \
                last_charge_history_sync_at, last_charge_history_success_at, \
                worker_health, worker_health_msg, auth_state, auth_reason_code FROM panda.vehicle_runtime_state \
         WHERE vehicle_id = $1").bind(vehicle_id).fetch_optional(pool).await?;
        let issued = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT token_created_at FROM panda.vehicle_credentials WHERE vehicle_id = $1",
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?;
        let renewal = issued.map(|at| {
            rivian_renewal_state_for_auth(
                at,
                Utc::now(),
                runtime.as_ref().and_then(|r| r.auth_state.as_deref()),
            )
        });
        let latest = sqlx::query_as::<_, LatestVehicleTelemetry>(r#"
        SELECT
          ts, latitude, longitude, altitude_m, speed_mph, location_ts, speed_mph_ts,
          battery_level, battery_capacity_wh, distance_to_empty_mi, battery_limit,
          battery_level_ts, distance_to_empty_mi_ts, battery_limit_ts,
          power_state, power_state_ts,
          charger_state, charger_state_ts, charger_status, charger_status_ts,
          time_to_end_of_charge_min, time_to_end_of_charge_min_ts,
          drive_mode, gear_status, cabin_temp_c, driver_temp_c, outside_temp_c,
          heading_deg, odometer_miles, odometer_miles_ts,
          tire_fl_psi, tire_fr_psi, tire_rl_psi, tire_rr_psi,
          tire_fl_status, tire_fr_status, tire_rl_status, tire_rr_status,
          tire_fl_valid, tire_fr_valid, tire_rl_valid, tire_rr_valid,
          door_front_left_locked, door_front_right_locked, door_rear_left_locked, door_rear_right_locked,
          door_front_left_closed, door_front_right_closed, door_rear_left_closed, door_rear_right_closed,
          closure_frunk_locked, closure_frunk_closed, closure_liftgate_locked, closure_liftgate_closed,
          closure_tailgate_locked, closure_tailgate_closed,
          ota_current_version, ota_available_version, ota_status, ota_current_status,
          hv_thermal_event, twelve_volt_health,
          charge_port_open, charger_derate_active, cabin_precon_status, cabin_precon_type,
          pet_mode_active, pet_mode_temp_ok, defrost_active, steering_wheel_heat,
          seat_fl_heat, seat_fr_heat, seat_rl_heat, seat_rr_heat,
          seat_fl_vent, seat_fr_vent,
          tonneau_locked, tonneau_closed, side_bin_left_locked, side_bin_right_locked,
          side_bin_left_closed, side_bin_right_closed,
          window_fl_closed, window_fr_closed, window_rl_closed, window_rr_closed,
          gear_guard_locked, gear_guard_video_status, wiper_fluid_low, brake_fluid_low,
          alarm_active, service_mode
        FROM panda.vehicle_latest_status
        WHERE vehicle_id = $1
        "#).bind(vehicle_id).fetch_optional(pool).await?.unwrap_or_default();
        let seen = sqlx::query_as::<_, VehicleStatusFieldSeenRow>(
            r#"
        SELECT
          max(ts) FILTER (WHERE tire_fl_psi IS NOT NULL) AS tire_fl_psi_at,
          max(ts) FILTER (WHERE tire_fr_psi IS NOT NULL) AS tire_fr_psi_at,
          max(ts) FILTER (WHERE tire_rl_psi IS NOT NULL) AS tire_rl_psi_at,
          max(ts) FILTER (WHERE tire_rr_psi IS NOT NULL) AS tire_rr_psi_at,
          max(ts) FILTER (WHERE tire_fl_status IS NOT NULL) AS tire_fl_status_at,
          max(ts) FILTER (WHERE tire_fr_status IS NOT NULL) AS tire_fr_status_at,
          max(ts) FILTER (WHERE tire_rl_status IS NOT NULL) AS tire_rl_status_at,
          max(ts) FILTER (WHERE tire_rr_status IS NOT NULL) AS tire_rr_status_at,
          max(ts) FILTER (WHERE tire_fl_valid IS NOT NULL) AS tire_fl_valid_at,
          max(ts) FILTER (WHERE tire_fr_valid IS NOT NULL) AS tire_fr_valid_at,
          max(ts) FILTER (WHERE tire_rl_valid IS NOT NULL) AS tire_rl_valid_at,
          max(ts) FILTER (WHERE tire_rr_valid IS NOT NULL) AS tire_rr_valid_at,
          max(ts) FILTER (WHERE hv_thermal_event IS NOT NULL) AS hv_thermal_event_at,
          max(ts) FILTER (WHERE twelve_volt_health IS NOT NULL) AS twelve_volt_health_at,
          max(ts) FILTER (WHERE ota_current_version IS NOT NULL) AS ota_current_version_at,
          max(ts) FILTER (WHERE ota_available_version IS NOT NULL) AS ota_available_version_at,
          max(ts) FILTER (WHERE ota_status IS NOT NULL) AS ota_status_at,
          max(ts) FILTER (WHERE ota_current_status IS NOT NULL) AS ota_current_status_at,
          max(ts) FILTER (WHERE charge_port_open IS NOT NULL) AS charge_port_open_at,
          max(ts) FILTER (WHERE charger_derate_active IS NOT NULL) AS charger_derate_active_at,
          max(ts) FILTER (WHERE cabin_precon_status IS NOT NULL) AS cabin_precon_status_at,
          max(ts) FILTER (WHERE cabin_precon_type IS NOT NULL) AS cabin_precon_type_at,
          max(ts) FILTER (WHERE pet_mode_active IS NOT NULL) AS pet_mode_active_at,
          max(ts) FILTER (WHERE pet_mode_temp_ok IS NOT NULL) AS pet_mode_temp_ok_at,
          max(ts) FILTER (WHERE defrost_active IS NOT NULL) AS defrost_active_at,
          max(ts) FILTER (WHERE steering_wheel_heat IS NOT NULL) AS steering_wheel_heat_at,
          max(ts) FILTER (WHERE seat_fl_heat IS NOT NULL) AS seat_fl_heat_at,
          max(ts) FILTER (WHERE seat_fr_heat IS NOT NULL) AS seat_fr_heat_at,
          max(ts) FILTER (WHERE seat_rl_heat IS NOT NULL) AS seat_rl_heat_at,
          max(ts) FILTER (WHERE seat_rr_heat IS NOT NULL) AS seat_rr_heat_at,
          max(ts) FILTER (WHERE seat_fl_vent IS NOT NULL) AS seat_fl_vent_at,
          max(ts) FILTER (WHERE seat_fr_vent IS NOT NULL) AS seat_fr_vent_at,
          max(ts) FILTER (WHERE tonneau_locked IS NOT NULL) AS tonneau_locked_at,
          max(ts) FILTER (WHERE tonneau_closed IS NOT NULL) AS tonneau_closed_at,
          max(ts) FILTER (WHERE side_bin_left_locked IS NOT NULL) AS side_bin_left_locked_at,
          max(ts) FILTER (WHERE side_bin_right_locked IS NOT NULL) AS side_bin_right_locked_at,
          max(ts) FILTER (WHERE side_bin_left_closed IS NOT NULL) AS side_bin_left_closed_at,
          max(ts) FILTER (WHERE side_bin_right_closed IS NOT NULL) AS side_bin_right_closed_at,
          max(ts) FILTER (WHERE window_fl_closed IS NOT NULL) AS window_fl_closed_at,
          max(ts) FILTER (WHERE window_fr_closed IS NOT NULL) AS window_fr_closed_at,
          max(ts) FILTER (WHERE window_rl_closed IS NOT NULL) AS window_rl_closed_at,
          max(ts) FILTER (WHERE window_rr_closed IS NOT NULL) AS window_rr_closed_at,
          max(ts) FILTER (WHERE gear_guard_locked IS NOT NULL) AS gear_guard_locked_at,
          max(ts) FILTER (WHERE gear_guard_video_status IS NOT NULL) AS gear_guard_video_status_at,
          max(ts) FILTER (WHERE wiper_fluid_low IS NOT NULL) AS wiper_fluid_low_at,
          max(ts) FILTER (WHERE brake_fluid_low IS NOT NULL) AS brake_fluid_low_at,
          max(ts) FILTER (WHERE alarm_active IS NOT NULL) AS alarm_active_at,
          max(ts) FILTER (WHERE service_mode IS NOT NULL) AS service_mode_at,
          max(ts) FILTER (WHERE closure_frunk_closed IS NOT NULL) AS closure_frunk_closed_at,
          max(ts) FILTER (WHERE closure_liftgate_closed IS NOT NULL) AS closure_liftgate_closed_at,
          max(ts) FILTER (WHERE closure_tailgate_closed IS NOT NULL) AS closure_tailgate_closed_at,
          max(ts) FILTER (WHERE door_front_left_closed IS NOT NULL) AS door_front_left_closed_at,
          max(ts) FILTER (WHERE door_front_right_closed IS NOT NULL) AS door_front_right_closed_at,
          max(ts) FILTER (WHERE door_rear_left_closed IS NOT NULL) AS door_rear_left_closed_at,
          max(ts) FILTER (WHERE door_rear_right_closed IS NOT NULL) AS door_rear_right_closed_at
        FROM timeseries.telemetry
        WHERE vehicle_id = $1
        "#,
        )
        .bind(vehicle_id)
        .fetch_one(pool)
        .await?;
        let (capacity, model) =
            vehicle.map_or((None, None), |(capacity, model)| (capacity, Some(model)));
        Ok(Self {
            capacity,
            model,
            runtime,
            latest,
            seen,
            issued,
            renewal,
        })
    }
    fn availability(
        &self,
        latest_event_at: Option<DateTime<Utc>>,
    ) -> BTreeMap<String, StatusFieldAvailability> {
        let latest = &self.latest;
        let seen = &self.seen;
        let mut fields = BTreeMap::new();
        let observations = [
            ("hv_thermal_event", seen.hv_thermal_event_at, false),
            ("twelve_volt_health", seen.twelve_volt_health_at, false),
            ("ota_current_version", seen.ota_current_version_at, false),
            (
                "ota_available_version",
                seen.ota_available_version_at,
                false,
            ),
            ("ota_status", seen.ota_status_at, false),
            ("ota_current_status", seen.ota_current_status_at, false),
            ("charge_port_open", seen.charge_port_open_at, false),
            (
                "charger_derate_active",
                seen.charger_derate_active_at,
                false,
            ),
            ("cabin_precon_status", seen.cabin_precon_status_at, false),
            ("cabin_precon_type", seen.cabin_precon_type_at, false),
            ("pet_mode_active", seen.pet_mode_active_at, false),
            ("pet_mode_temp_ok", seen.pet_mode_temp_ok_at, false),
            ("defrost_active", seen.defrost_active_at, false),
            ("steering_wheel_heat", seen.steering_wheel_heat_at, false),
            ("seat_fl_heat", seen.seat_fl_heat_at, false),
            ("seat_fr_heat", seen.seat_fr_heat_at, false),
            ("seat_rl_heat", seen.seat_rl_heat_at, false),
            ("seat_rr_heat", seen.seat_rr_heat_at, false),
            ("seat_fl_vent", seen.seat_fl_vent_at, false),
            ("seat_fr_vent", seen.seat_fr_vent_at, false),
            ("tonneau_locked", seen.tonneau_locked_at, false),
            ("tonneau_closed", seen.tonneau_closed_at, false),
            ("side_bin_left_locked", seen.side_bin_left_locked_at, false),
            (
                "side_bin_right_locked",
                seen.side_bin_right_locked_at,
                false,
            ),
            ("side_bin_left_closed", seen.side_bin_left_closed_at, false),
            (
                "side_bin_right_closed",
                seen.side_bin_right_closed_at,
                false,
            ),
            ("window_fl_closed", seen.window_fl_closed_at, false),
            ("window_fr_closed", seen.window_fr_closed_at, false),
            ("window_rl_closed", seen.window_rl_closed_at, false),
            ("window_rr_closed", seen.window_rr_closed_at, false),
            ("gear_guard_locked", seen.gear_guard_locked_at, false),
            (
                "gear_guard_video_status",
                seen.gear_guard_video_status_at,
                false,
            ),
            ("wiper_fluid_low", seen.wiper_fluid_low_at, false),
            ("brake_fluid_low", seen.brake_fluid_low_at, false),
            ("alarm_active", seen.alarm_active_at, false),
            ("service_mode", seen.service_mode_at, false),
            ("closure_frunk_closed", seen.closure_frunk_closed_at, false),
            (
                "closure_liftgate_closed",
                seen.closure_liftgate_closed_at,
                false,
            ),
            (
                "closure_tailgate_closed",
                seen.closure_tailgate_closed_at,
                false,
            ),
            (
                "door_front_left_closed",
                seen.door_front_left_closed_at,
                false,
            ),
            (
                "door_front_right_closed",
                seen.door_front_right_closed_at,
                false,
            ),
            (
                "door_rear_left_closed",
                seen.door_rear_left_closed_at,
                false,
            ),
            (
                "door_rear_right_closed",
                seen.door_rear_right_closed_at,
                false,
            ),
            (
                "tire_fl_psi",
                seen.tire_fl_psi_at,
                latest.tire_fl_valid == Some(false),
            ),
            (
                "tire_fr_psi",
                seen.tire_fr_psi_at,
                latest.tire_fr_valid == Some(false),
            ),
            (
                "tire_rl_psi",
                seen.tire_rl_psi_at,
                latest.tire_rl_valid == Some(false),
            ),
            (
                "tire_rr_psi",
                seen.tire_rr_psi_at,
                latest.tire_rr_valid == Some(false),
            ),
            (
                "tire_fl_status",
                seen.tire_fl_status_at,
                latest.tire_fl_valid == Some(false),
            ),
            (
                "tire_fr_status",
                seen.tire_fr_status_at,
                latest.tire_fr_valid == Some(false),
            ),
            (
                "tire_rl_status",
                seen.tire_rl_status_at,
                latest.tire_rl_valid == Some(false),
            ),
            (
                "tire_rr_status",
                seen.tire_rr_status_at,
                latest.tire_rr_valid == Some(false),
            ),
            (
                "tire_fl_valid",
                seen.tire_fl_valid_at,
                latest.tire_fl_valid == Some(false),
            ),
            (
                "tire_fr_valid",
                seen.tire_fr_valid_at,
                latest.tire_fr_valid == Some(false),
            ),
            (
                "tire_rl_valid",
                seen.tire_rl_valid_at,
                latest.tire_rl_valid == Some(false),
            ),
            (
                "tire_rr_valid",
                seen.tire_rr_valid_at,
                latest.tire_rr_valid == Some(false),
            ),
        ];
        for (field, observed, invalid) in observations {
            insert_field_availability(
                &mut fields,
                field,
                latest_event_at,
                observed,
                invalid.then_some(StatusFieldAvailabilityReasonCode::InvalidSensor),
            );
        }
        let tire_seen = observations
            .iter()
            .filter(|(field, _, _)| field.starts_with("tire_"));
        let seen_at = max_seen_at(tire_seen.clone().map(|(_, at, _)| *at));
        let invalid = tire_seen.into_iter().any(|(_, _, invalid)| *invalid);
        insert_field_availability(
            &mut fields,
            "tire_pressure_status",
            latest_event_at,
            seen_at,
            invalid.then_some(StatusFieldAvailabilityReasonCode::InvalidSensor),
        );
        fields
    }

    pub(super) fn render(self, vehicle_id: Uuid, now: DateTime<Utc>) -> VehicleStatusResponse {
        let latest_event = self
            .latest
            .ts
            .or_else(|| self.runtime.as_ref().and_then(|r| r.last_event_at));
        let field_availability = self.availability(latest_event);
        let (worker_health, telemetry_stale, telemetry_stale_reason) =
            derive_vehicle_status_freshness(now, self.runtime.as_ref(), &self.latest);
        let range_miles = normalize_remaining_range_miles(
            self.latest.distance_to_empty_mi,
            self.latest.battery_level,
            self.latest.battery_capacity_wh.or(self.capacity),
            self.model.as_deref(),
        );
        let tires = [
            (
                self.latest.tire_fl_psi,
                self.latest.tire_fl_status.as_deref(),
                self.latest.tire_fl_valid,
            ),
            (
                self.latest.tire_fr_psi,
                self.latest.tire_fr_status.as_deref(),
                self.latest.tire_fr_valid,
            ),
            (
                self.latest.tire_rl_psi,
                self.latest.tire_rl_status.as_deref(),
                self.latest.tire_rl_valid,
            ),
            (
                self.latest.tire_rr_psi,
                self.latest.tire_rr_status.as_deref(),
                self.latest.tire_rr_valid,
            ),
        ];
        let mut tire_min_psi: Option<f64> = None;
        let mut first_status = None;
        let mut warning = None;
        let mut invalid = false;
        for (psi, status, valid) in tires {
            if let Some(psi) = psi.filter(|v| v.is_finite()) {
                if tire_min_psi.is_none_or(|minimum| psi.total_cmp(&minimum).is_lt()) {
                    tire_min_psi = Some(psi);
                }
            }
            if let Some(status) = status {
                first_status.get_or_insert(status);
                if !status.eq_ignore_ascii_case("ok") && !status.eq_ignore_ascii_case("unknown") {
                    warning.get_or_insert(status);
                }
            }
            invalid |= valid == Some(false);
        }
        let tire_pressure_status = if invalid {
            Some("invalid_sensor")
        } else {
            warning.or(first_status)
        }
        .map(str::to_owned);
        let doors_locked = [
            self.latest.door_front_left_locked,
            self.latest.door_front_right_locked,
            self.latest.door_rear_left_locked,
            self.latest.door_rear_right_locked,
        ]
        .into_iter()
        .try_fold(true, |locked, next| next.map(|next| locked && next));
        let open_closures = open_closures_json(&[
            ("Front left door", self.latest.door_front_left_closed),
            ("Front right door", self.latest.door_front_right_closed),
            ("Rear left door", self.latest.door_rear_left_closed),
            ("Rear right door", self.latest.door_rear_right_closed),
            ("Frunk", self.latest.closure_frunk_closed),
            ("Liftgate", self.latest.closure_liftgate_closed),
            ("Tailgate", self.latest.closure_tailgate_closed),
            ("Tonneau", self.latest.tonneau_closed),
            ("Left side bin", self.latest.side_bin_left_closed),
            ("Right side bin", self.latest.side_bin_right_closed),
            ("Front left window", self.latest.window_fl_closed),
            ("Front right window", self.latest.window_fr_closed),
            ("Rear left window", self.latest.window_rl_closed),
            ("Rear right window", self.latest.window_rr_closed),
        ]);
        let software_update_status = self
            .latest
            .ota_status
            .clone()
            .or_else(|| self.latest.ota_current_status.clone());
        let runtime = self.runtime.unwrap_or_default();
        let LatestVehicleTelemetry {
            ts,
            latitude,
            longitude,
            altitude_m,
            speed_mph,
            location_ts,
            speed_mph_ts,
            battery_level,
            battery_capacity_wh,
            battery_limit,
            battery_level_ts,
            distance_to_empty_mi_ts,
            battery_limit_ts,
            power_state,
            power_state_ts,
            charger_state,
            charger_state_ts,
            charger_status,
            charger_status_ts,
            time_to_end_of_charge_min,
            time_to_end_of_charge_min_ts,
            drive_mode,
            gear_status,
            cabin_temp_c,
            driver_temp_c,
            outside_temp_c,
            heading_deg,
            odometer_miles,
            odometer_miles_ts,
            tire_fl_psi,
            tire_fr_psi,
            tire_rl_psi,
            tire_rr_psi,
            tire_fl_status,
            tire_fr_status,
            tire_rl_status,
            tire_rr_status,
            tire_fl_valid,
            tire_fr_valid,
            tire_rl_valid,
            tire_rr_valid,
            door_front_left_locked,
            door_front_right_locked,
            door_rear_left_locked,
            door_rear_right_locked,
            door_front_left_closed,
            door_front_right_closed,
            door_rear_left_closed,
            door_rear_right_closed,
            closure_frunk_locked,
            closure_frunk_closed,
            closure_liftgate_locked,
            closure_liftgate_closed,
            closure_tailgate_locked,
            closure_tailgate_closed,
            ota_current_version,
            ota_available_version,
            ota_status,
            ota_current_status,
            hv_thermal_event,
            twelve_volt_health,
            charge_port_open,
            charger_derate_active,
            cabin_precon_status,
            cabin_precon_type,
            pet_mode_active,
            pet_mode_temp_ok,
            defrost_active,
            steering_wheel_heat,
            seat_fl_heat,
            seat_fr_heat,
            seat_rl_heat,
            seat_rr_heat,
            seat_fl_vent,
            seat_fr_vent,
            tonneau_locked,
            tonneau_closed,
            side_bin_left_locked,
            side_bin_right_locked,
            side_bin_left_closed,
            side_bin_right_closed,
            window_fl_closed,
            window_fr_closed,
            window_rl_closed,
            window_rr_closed,
            gear_guard_locked,
            gear_guard_video_status,
            wiper_fluid_low,
            brake_fluid_low,
            alarm_active,
            service_mode,
            ..
        } = self.latest;
        VehicleStatusResponse {
            vehicle_id,
            is_online: runtime.is_online.unwrap_or(false),
            last_event_at: runtime.last_event_at,
            last_updated: latest_event,
            last_payload_at: runtime.last_payload_at,
            last_heartbeat_at: runtime.last_heartbeat_at,
            last_ws_received_at: runtime.last_ws_received_at,
            last_ws_payload_received_at: runtime.last_ws_payload_received_at,
            last_ws_heartbeat_received_at: runtime.last_ws_heartbeat_received_at,
            last_charge_history_sync_at: runtime.last_charge_history_sync_at,
            last_charge_history_success_at: runtime.last_charge_history_success_at,
            worker_health,
            worker_health_msg: runtime.worker_health_msg,
            auth_state: runtime.auth_state,
            auth_reason_code: runtime.auth_reason_code,
            credential_issued_at: self.issued,
            expected_renewal_at: self.issued.map(|at| at + RIVIAN_RENEWAL_INTERVAL),
            renewal_state: self.renewal,
            battery_level,
            battery_level_ts: battery_level_ts.or(ts),
            range_miles,
            range_miles_ts: distance_to_empty_mi_ts.or(ts),
            battery_capacity_kwh: battery_capacity_wh.map(
                |w| {
                    if w > 1000.0 {
                        w / 1000.0
                    } else {
                        w
                    }
                },
            ),
            battery_limit,
            battery_limit_ts,
            power_state,
            power_state_ts: power_state_ts.or(ts),
            charger_state,
            charger_state_ts,
            charger_status,
            charger_status_ts,
            time_to_end_of_charge_min,
            time_to_end_of_charge_min_ts,
            speed_mph,
            speed_mph_ts,
            altitude_m,
            latitude,
            longitude,
            location_ts,
            drive_mode,
            gear_status,
            cabin_temp_c,
            driver_temp_c,
            outside_temp_c,
            heading_deg,
            odometer_miles,
            odometer_miles_ts,
            tire_fl_psi,
            tire_fr_psi,
            tire_rl_psi,
            tire_rr_psi,
            tire_min_psi,
            tire_fl_status,
            tire_fr_status,
            tire_rl_status,
            tire_rr_status,
            tire_fl_valid,
            tire_fr_valid,
            tire_rl_valid,
            tire_rr_valid,
            door_front_left_locked,
            door_front_right_locked,
            door_rear_left_locked,
            door_rear_right_locked,
            door_front_left_closed,
            door_front_right_closed,
            door_rear_left_closed,
            door_rear_right_closed,
            closure_frunk_locked,
            closure_frunk_closed,
            closure_liftgate_locked,
            closure_liftgate_closed,
            closure_tailgate_locked,
            closure_tailgate_closed,
            ota_current_version,
            ota_available_version,
            ota_status,
            ota_current_status,
            hv_thermal_event,
            twelve_volt_health,
            doors_locked,
            open_closures,
            tire_pressure_status,
            software_update_status,
            // Extended vehicleStatus fields (migration 0022)
            charge_port_open,
            charger_derate_active,
            cabin_precon_status,
            cabin_precon_type,
            pet_mode_active,
            pet_mode_temp_ok,
            defrost_active,
            steering_wheel_heat,
            seat_fl_heat,
            seat_fr_heat,
            seat_rl_heat,
            seat_rr_heat,
            seat_fl_vent,
            seat_fr_vent,
            tonneau_locked,
            tonneau_closed,
            side_bin_left_locked,
            side_bin_right_locked,
            side_bin_left_closed,
            side_bin_right_closed,
            window_fl_closed,
            window_fr_closed,
            window_rl_closed,
            window_rr_closed,
            gear_guard_locked,
            gear_guard_video_status,
            wiper_fluid_low,
            brake_fluid_low,
            alarm_active,
            service_mode,
            telemetry_stale,
            telemetry_stale_reason,
            field_availability,
        }
    }
}

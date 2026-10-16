use super::{
    AppError, DateTime, DemoFixtureSpec, Duration, ModelProfile, Postgres, Transaction, Utc, Uuid,
    DEMO_ACTIVE_DAY_OFFSETS,
};
use serde::Serialize;

#[derive(sqlx::FromRow)]
struct TripWindow {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    start_lat: Option<f64>,
    start_lng: Option<f64>,
    end_lat: Option<f64>,
    end_lng: Option<f64>,
    drive_mode: Option<String>,
}
#[derive(sqlx::FromRow)]
struct ChargeWindow {
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct Sample {
    ts: DateTime<Utc>,
    vehicle_id: Uuid,
    latitude: Option<f64>,
    longitude: Option<f64>,
    altitude_m: Option<f64>,
    speed_mph: Option<f64>,
    battery_level: Option<f64>,
    battery_capacity_wh: Option<f64>,
    distance_to_empty_mi: Option<f64>,
    battery_limit: Option<f64>,
    power_state: Option<String>,
    charger_state: Option<String>,
    charger_status: Option<String>,
    time_to_end_of_charge_min: Option<i32>,
    drive_mode: Option<String>,
    gear_status: Option<String>,
    cabin_temp_c: Option<f64>,
    driver_temp_c: Option<f64>,
    odometer_miles: Option<f64>,
    hv_thermal_event: Option<String>,
    twelve_volt_health: Option<String>,
    is_online: Option<bool>,
    heading_deg: Option<f64>,
    outside_temp_c: Option<f64>,
    hvac_active: Option<bool>,
    power_kw: Option<f64>,
    regen_power_kw: Option<f64>,
    tire_fl_psi: Option<f64>,
    tire_fr_psi: Option<f64>,
    tire_rl_psi: Option<f64>,
    tire_rr_psi: Option<f64>,
    tire_fl_status: Option<String>,
    tire_fr_status: Option<String>,
    tire_rl_status: Option<String>,
    tire_rr_status: Option<String>,
    door_front_left_locked: Option<bool>,
    door_front_right_locked: Option<bool>,
    door_rear_left_locked: Option<bool>,
    door_rear_right_locked: Option<bool>,
    door_front_left_closed: Option<bool>,
    door_front_right_closed: Option<bool>,
    door_rear_left_closed: Option<bool>,
    door_rear_right_closed: Option<bool>,
    closure_frunk_locked: Option<bool>,
    closure_frunk_closed: Option<bool>,
    closure_liftgate_locked: Option<bool>,
    closure_liftgate_closed: Option<bool>,
    closure_tailgate_locked: Option<bool>,
    closure_tailgate_closed: Option<bool>,
    ota_current_version: Option<String>,
    ota_status: Option<String>,
    trip_id: Option<Uuid>,
    charge_session_id: Option<Uuid>,
    charge_port_open: Option<bool>,
    charger_derate_active: Option<bool>,
    cabin_precon_status: Option<String>,
    cabin_precon_type: Option<String>,
    pet_mode_active: Option<bool>,
    pet_mode_temp_ok: Option<bool>,
    defrost_active: Option<bool>,
    steering_wheel_heat: Option<i32>,
    seat_fl_heat: Option<i32>,
    seat_fr_heat: Option<i32>,
    seat_rl_heat: Option<i32>,
    seat_rr_heat: Option<i32>,
    seat_fl_vent: Option<i32>,
    seat_fr_vent: Option<i32>,
    tonneau_locked: Option<bool>,
    tonneau_closed: Option<bool>,
    side_bin_left_locked: Option<bool>,
    side_bin_right_locked: Option<bool>,
    window_fl_closed: Option<bool>,
    window_fr_closed: Option<bool>,
    window_rl_closed: Option<bool>,
    window_rr_closed: Option<bool>,
    gear_guard_locked: Option<bool>,
    gear_guard_video_status: Option<String>,
    wiper_fluid_low: Option<bool>,
    brake_fluid_low: Option<bool>,
    alarm_active: Option<bool>,
    service_mode: Option<bool>,
    tire_fl_valid: Option<bool>,
    tire_fr_valid: Option<bool>,
    tire_rl_valid: Option<bool>,
    tire_rr_valid: Option<bool>,
    side_bin_left_closed: Option<bool>,
    side_bin_right_closed: Option<bool>,
}

struct Coverage {
    location: i64,
    battery: i64,
    tires: i64,
    doors: i64,
}
impl Coverage {
    fn new(fixture: &DemoFixtureSpec) -> Self {
        Self {
            location: (fixture.coverage.location * 1000.0).round() as i64,
            battery: (fixture.coverage.battery * 1000.0).round() as i64,
            tires: (fixture.coverage.tires * 1000.0).round() as i64,
            doors: (fixture.coverage.doors * 1000.0).round() as i64,
        }
    }
}

struct Timeline<T> {
    windows: Vec<T>,
    first: usize,
}
impl<T> Timeline<T> {
    fn at(
        &mut self,
        ts: DateTime<Utc>,
        bounds: impl Fn(&T) -> (DateTime<Utc>, DateTime<Utc>),
    ) -> Option<&T> {
        while self.first < self.windows.len() && bounds(&self.windows[self.first]).1 < ts {
            self.first += 1;
        }
        self.windows[self.first..]
            .iter()
            .take_while(|w| bounds(w).0 <= ts)
            .find(|w| bounds(w).1 >= ts)
    }
}

pub(super) async fn seed(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    anchor: DateTime<Utc>,
    fixture: &DemoFixtureSpec,
    profile: ModelProfile,
) -> Result<(), AppError> {
    let per_day = fixture.telemetry_rows / i64::from(fixture.active_days);
    if per_day * i64::from(fixture.active_days) != fixture.telemetry_rows {
        return Err(AppError::Validation(
            "demo telemetry row count must divide evenly across active days".into(),
        ));
    }
    let mut trips = Timeline { windows: sqlx::query_as::<_, TripWindow>(
        "SELECT id,started_at,ended_at,start_lat,start_lng,end_lat,end_lng,drive_mode FROM panda.trips WHERE vehicle_id=$1 ORDER BY started_at"
    ).bind(vehicle_id).fetch_all(&mut **tx).await?, first: 0 };
    let mut charges = Timeline { windows: sqlx::query_as::<_, ChargeWindow>(
        "SELECT id,started_at,ended_at FROM panda.charge_sessions WHERE vehicle_id=$1 AND ended_at IS NOT NULL ORDER BY started_at"
    ).bind(vehicle_id).fetch_all(&mut **tx).await?, first: 0 };
    let generator = Generator {
        vehicle_id,
        total: fixture.telemetry_rows,
        coverage: Coverage::new(fixture),
        profile,
    };
    let midnights: Vec<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT $1::timestamptz - interval '14 days' + make_interval(days => day_offset)
         FROM unnest($2::int[]) WITH ORDINALITY AS d(day_offset,ordinal) ORDER BY ordinal",
    )
    .bind(anchor)
    .bind(DEMO_ACTIVE_DAY_OFFSETS.map(|day| day as i32).as_slice())
    .fetch_all(&mut **tx)
    .await?;
    for (day, midnight) in midnights.into_iter().enumerate() {
        let rows: Vec<_> = (0..per_day)
            .map(|sample| {
                let micros = (sample as f64 * 86400.0 / per_day as f64 * 1_000_000.0)
                    .round_ties_even() as i64;
                let ts = midnight + Duration::microseconds(micros);
                let trip = trips.at(ts, |w| (w.started_at, w.ended_at));
                let charge = charges.at(ts, |w| (w.started_at, w.ended_at));
                generator.sample(ts, day as i64 * per_day + sample, trip, charge)
            })
            .collect();
        let result = sqlx::query(
            "INSERT INTO timeseries.telemetry (ts,vehicle_id,latitude,longitude,altitude_m,speed_mph,battery_level,battery_capacity_wh,distance_to_empty_mi,battery_limit,power_state,charger_state,charger_status,time_to_end_of_charge_min,drive_mode,gear_status,cabin_temp_c,driver_temp_c,odometer_miles,hv_thermal_event,twelve_volt_health,is_online,heading_deg,outside_temp_c,hvac_active,power_kw,regen_power_kw,tire_fl_psi,tire_fr_psi,tire_rl_psi,tire_rr_psi,tire_fl_status,tire_fr_status,tire_rl_status,tire_rr_status,door_front_left_locked,door_front_right_locked,door_rear_left_locked,door_rear_right_locked,door_front_left_closed,door_front_right_closed,door_rear_left_closed,door_rear_right_closed,closure_frunk_locked,closure_frunk_closed,closure_liftgate_locked,closure_liftgate_closed,closure_tailgate_locked,closure_tailgate_closed,ota_current_version,ota_status,trip_id,charge_session_id,charge_port_open,charger_derate_active,cabin_precon_status,cabin_precon_type,pet_mode_active,pet_mode_temp_ok,defrost_active,steering_wheel_heat,seat_fl_heat,seat_fr_heat,seat_rl_heat,seat_rr_heat,seat_fl_vent,seat_fr_vent,tonneau_locked,tonneau_closed,side_bin_left_locked,side_bin_right_locked,window_fl_closed,window_fr_closed,window_rl_closed,window_rr_closed,gear_guard_locked,gear_guard_video_status,wiper_fluid_low,brake_fluid_low,alarm_active,service_mode,tire_fl_valid,tire_fr_valid,tire_rl_valid,tire_rr_valid,side_bin_left_closed,side_bin_right_closed)
             SELECT ts,vehicle_id,latitude,longitude,altitude_m,speed_mph,battery_level,battery_capacity_wh,distance_to_empty_mi,battery_limit,power_state,charger_state,charger_status,time_to_end_of_charge_min,drive_mode,gear_status,cabin_temp_c,driver_temp_c,odometer_miles,hv_thermal_event,twelve_volt_health,is_online,heading_deg,outside_temp_c,hvac_active,power_kw,regen_power_kw,tire_fl_psi,tire_fr_psi,tire_rl_psi,tire_rr_psi,tire_fl_status,tire_fr_status,tire_rl_status,tire_rr_status,door_front_left_locked,door_front_right_locked,door_rear_left_locked,door_rear_right_locked,door_front_left_closed,door_front_right_closed,door_rear_left_closed,door_rear_right_closed,closure_frunk_locked,closure_frunk_closed,closure_liftgate_locked,closure_liftgate_closed,closure_tailgate_locked,closure_tailgate_closed,ota_current_version,ota_status,trip_id,charge_session_id,charge_port_open,charger_derate_active,cabin_precon_status,cabin_precon_type,pet_mode_active,pet_mode_temp_ok,defrost_active,steering_wheel_heat,seat_fl_heat,seat_fr_heat,seat_rl_heat,seat_rr_heat,seat_fl_vent,seat_fr_vent,tonneau_locked,tonneau_closed,side_bin_left_locked,side_bin_right_locked,window_fl_closed,window_fr_closed,window_rl_closed,window_rr_closed,gear_guard_locked,gear_guard_video_status,wiper_fluid_low,brake_fluid_low,alarm_active,service_mode,tire_fl_valid,tire_fr_valid,tire_rl_valid,tire_rr_valid,side_bin_left_closed,side_bin_right_closed FROM json_populate_recordset(NULL::timeseries.telemetry,$1::json)"
        ).bind(sqlx::types::Json(rows)).execute(&mut **tx).await?;
        if result.rows_affected() != per_day as u64 {
            return Err(AppError::Validation(format!(
                "demo telemetry batch {day} inserted {} rows instead of {per_day}",
                result.rows_affected()
            )));
        }
    }
    Ok(())
}

struct Generator {
    vehicle_id: Uuid,
    total: i64,
    coverage: Coverage,
    profile: ModelProfile,
}

impl Generator {
    fn sample(
        &self,
        ts: DateTime<Utc>,
        index: i64,
        trip: Option<&TripWindow>,
        charge: Option<&ChargeWindow>,
    ) -> Sample {
        let Self {
            vehicle_id,
            total,
            coverage,
            profile,
        } = self;
        let rem = index % 1000;
        let battery = rem < coverage.battery;
        let tires = rem < coverage.tires;
        let doors = rem < coverage.doors;
        let truck = doors && profile.has_truck_closures;
        let liftgate = doors && profile.has_liftgate;
        let driving = trip.is_some();
        let progress = index as f64 / ((*total as f64 - 1.0).max(1.0));
        let soc = 85.0 - progress * 17.0;
        let text = |present: bool, value: &str| present.then(|| value.to_owned());
        let interpolate = |start: Option<f64>, end: Option<f64>, window: &TripWindow| {
            let elapsed = (ts - window.started_at).num_microseconds()? as f64;
            let duration = (window.ended_at - window.started_at).num_microseconds()? as f64;
            if duration == 0.0 {
                return None;
            }
            Some(start? + (end? - start?) * (elapsed / duration))
        };
        Sample {
            ts,
            vehicle_id: *vehicle_id,
            latitude: trip
                .and_then(|w| interpolate(w.start_lat, w.end_lat, w))
                .or_else(|| {
                    (!driving && rem < coverage.location)
                        .then_some(38.8895 + (index % 17) as f64 / 100000.0)
                }),
            longitude: trip
                .and_then(|w| interpolate(w.start_lng, w.end_lng, w))
                .or_else(|| {
                    (!driving && rem < coverage.location)
                        .then_some(-77.0353 - (index % 19) as f64 / 100000.0)
                }),
            altitude_m: (driving || rem < coverage.location).then_some(8.0 + (index % 70) as f64),
            speed_mph: Some(if driving {
                18.0 + (index % 47) as f64
            } else {
                0.0
            }),
            battery_level: battery.then_some(soc),
            battery_capacity_wh: battery
                .then_some(profile.capacity_wh * (0.996 - progress * 0.006)),
            distance_to_empty_mi: battery.then_some(profile.max_range_mi * (soc / 100.0)),
            battery_limit: Some(85.0),
            power_state: Some(
                if driving {
                    "drive"
                } else if charge.is_some() {
                    "charging"
                } else if index % 7 == 0 {
                    "sleep"
                } else {
                    "ready"
                }
                .into(),
            ),
            charger_state: Some(
                if charge.is_some() {
                    "Charging"
                } else {
                    "Disconnected"
                }
                .into(),
            ),
            charger_status: Some(
                if charge.is_some() {
                    "chrgr_sts_connected_charging"
                } else {
                    "chrgr_sts_not_connected"
                }
                .into(),
            ),
            time_to_end_of_charge_min: charge.map(|w| {
                (((w.ended_at - ts).num_microseconds().unwrap() as f64 / 1_000_000.0 / 60.0).ceil()
                    as i32)
                    .max(0)
            }),
            drive_mode: trip.and_then(|w| w.drive_mode.clone()),
            gear_status: Some(if driving { "drive" } else { "park" }.into()),
            cabin_temp_c: (index % 13 < 8).then_some(20.0 + (index % 42) as f64 / 10.0),
            driver_temp_c: (index % 17 < 10).then_some(19.0 + (index % 35) as f64 / 10.0),
            odometer_miles: battery.then_some(15000.0 + index as f64 * 0.011),
            hv_thermal_event: text(index % 181 == 0, "none"),
            twelve_volt_health: text(index % 173 == 0, "normal"),
            is_online: Some(true),
            heading_deg: driving.then_some((index * 17 % 360) as f64),
            outside_temp_c: None,
            hvac_active: (index % 23 == 0).then_some(true),
            power_kw: driving.then_some(16.0 + (index % 55) as f64),
            regen_power_kw: (driving && index % 11 == 0).then_some(6.0 + (index % 18) as f64),
            tire_fl_psi: tires.then_some(47.5 + (index % 18) as f64 / 10.0),
            tire_fr_psi: tires.then_some(47.6 + (index % 19) as f64 / 10.0),
            tire_rl_psi: tires.then_some(49.0 + (index % 17) as f64 / 10.0),
            tire_rr_psi: tires.then_some(49.1 + (index % 16) as f64 / 10.0),
            tire_fl_status: text(tires, "normal"),
            tire_fr_status: text(tires, "normal"),
            tire_rl_status: text(tires, "normal"),
            tire_rr_status: text(tires, "normal"),
            door_front_left_locked: doors.then_some(true),
            door_front_right_locked: doors.then_some(true),
            door_rear_left_locked: doors.then_some(true),
            door_rear_right_locked: doors.then_some(true),
            door_front_left_closed: doors.then_some(true),
            door_front_right_closed: doors.then_some(true),
            door_rear_left_closed: doors.then_some(true),
            door_rear_right_closed: doors.then_some(true),
            closure_frunk_locked: doors.then_some(true),
            closure_frunk_closed: doors.then_some(true),
            closure_liftgate_locked: liftgate.then_some(true),
            closure_liftgate_closed: liftgate.then_some(true),
            closure_tailgate_locked: truck.then_some(true),
            closure_tailgate_closed: truck.then_some(true),
            ota_current_version: text(index % 100 == 0, "2026.18.0"),
            ota_status: text(index % 100 == 0, "idle"),
            trip_id: trip.map(|w| w.id),
            charge_session_id: charge.map(|w| w.id),
            charge_port_open: Some(charge.is_some()),
            charger_derate_active: (index % 211 == 0).then_some(false),
            cabin_precon_status: text(index % 191 == 0, "off"),
            cabin_precon_type: text(index % 191 == 0, "none"),
            pet_mode_active: (index % 223 == 0).then_some(false),
            pet_mode_temp_ok: (index % 223 == 0).then_some(true),
            defrost_active: (index % 229 == 0).then_some(false),
            steering_wheel_heat: (index % 227 == 0).then_some(0),
            seat_fl_heat: (index % 233 == 0).then_some(0),
            seat_fr_heat: (index % 239 == 0).then_some(0),
            seat_rl_heat: (index % 241 == 0).then_some(0),
            seat_rr_heat: (index % 251 == 0).then_some(0),
            seat_fl_vent: (index % 257 == 0).then_some(0),
            seat_fr_vent: (index % 263 == 0).then_some(0),
            tonneau_locked: truck.then_some(true),
            tonneau_closed: truck.then_some(true),
            side_bin_left_locked: truck.then_some(true),
            side_bin_right_locked: truck.then_some(true),
            window_fl_closed: doors.then_some(true),
            window_fr_closed: doors.then_some(true),
            window_rl_closed: doors.then_some(true),
            window_rr_closed: doors.then_some(true),
            gear_guard_locked: (index % 269 == 0).then_some(true),
            gear_guard_video_status: text(index % 269 == 0, "idle"),
            wiper_fluid_low: (index % 271 == 0).then_some(false),
            brake_fluid_low: (index % 277 == 0).then_some(false),
            alarm_active: (index % 281 == 0).then_some(false),
            service_mode: (index % 283 == 0).then_some(false),
            tire_fl_valid: tires.then_some(true),
            tire_fr_valid: tires.then_some(true),
            tire_rl_valid: tires.then_some(true),
            tire_rr_valid: tires.then_some(true),
            side_bin_left_closed: truck.then_some(true),
            side_bin_right_closed: truck.then_some(true),
        }
    }
}

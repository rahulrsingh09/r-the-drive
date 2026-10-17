//! Trip lifecycle reducer with durable vehicle observations and trip aggregates.
//! Start anchors, active-frame accumulation, and completion share the existing
//! checkpoint layout used by ingestion recovery.

use crate::models::telemetry::{PowerState, TelemetryEvent};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::location::valid_location_pair;

const STOPPED_SPEED_MPH: f64 = 2.0;
const STOPPED_DURATION_SECS: i64 = 300; // 5 min
#[allow(dead_code)]
const MIN_TRIP_DISTANCE_MI: f64 = 0.1;

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum TripEvent {
    TripStarted {
        trip_id: Uuid,
        started_at: DateTime<Utc>,
    },
    TripEnded {
        trip: CompletedTripData,
    },
    NoChange,
}

impl TripEvent {
    /// Safe for diagnostic logs: completed trips contain coordinates and IDs.
    pub fn transition_name(&self) -> &'static str {
        match self {
            Self::TripStarted { .. } => "started",
            Self::TripEnded { .. } => "ended",
            Self::NoChange => "no_change",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletedTripData {
    pub trip_id: Uuid,
    pub vehicle_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub points: Vec<TrackPoint>,

    // SOC / energy
    pub soc_start: Option<f64>,
    pub soc_end: Option<f64>,
    pub battery_capacity_wh: Option<f64>,

    // Odometer at trip boundaries
    pub start_odometer_mi: Option<f64>,
    pub end_odometer_mi: Option<f64>,

    // Range (distance-to-empty) at trip boundaries
    pub range_start_mi: Option<f64>,
    pub range_end_mi: Option<f64>,

    // Power envelope (positive = traction, negative = regen)
    pub power_max_kw: Option<f64>,
    pub power_min_kw: Option<f64>,

    // Cumulative elevation change
    pub elevation_gain_m: Option<f64>,
    pub elevation_loss_m: Option<f64>,

    // Average cabin temperature
    pub inside_temp_avg_c: Option<f64>,

    // Average outside (ambient) temperature
    pub outside_temp_avg_c: Option<f64>,

    // Regenerative braking energy (Wh)
    pub regen_wh: Option<f64>,

    pub dominant_drive_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackPoint {
    pub ts: DateTime<Utc>,
    pub lat: f64,
    pub lng: f64,
    pub speed_mph: f64,
    pub altitude_m: Option<f64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TripDetectorState {
    vehicle_id: Uuid,
    active_trip_id: Option<Uuid>,
    trip_started_at: Option<DateTime<Utc>>,
    track_points: Vec<TrackPoint>,

    soc_at_start: Option<f64>,
    last_soc: Option<f64>,
    battery_capacity: Option<f64>,

    start_odometer: Option<f64>,
    last_odometer: Option<f64>,

    range_at_start: Option<f64>,
    last_range: Option<f64>,

    power_max: Option<f64>,
    power_min: Option<f64>,

    last_altitude: Option<f64>,
    elevation_gain_acc: f64,
    elevation_loss_acc: f64,

    inside_temp_sum: f64,
    inside_temp_count: u32,

    outside_temp_sum: f64,
    outside_temp_count: u32,

    regen_wh_acc: f64,
    last_regen_ts: Option<DateTime<Utc>>,

    drive_modes: Vec<String>,
    last_moving_at: Option<DateTime<Utc>>,

    // Odometer when the vehicle last shifted into gear (or ended a trip while
    // still in gear). Sparse vehicles report motion only after the first
    // odometer steps, so this marks where the upcoming trip really began.
    in_gear: bool,
    gear_odometer: Option<f64>,
    // When that shift happened, and the last location seen while parked, so
    // a backdated start also gets the matching start time and place.
    gear_at: Option<DateTime<Utc>>,
    last_parked_fix: Option<TrackPoint>,
}

/// A gear shift older than this no longer describes the trip that starts
/// now (a missed drive must not be attributed to a much later one).
const GEAR_ANCHOR_MAX_AGE_SECS: i64 = 15 * 60;

#[derive(Clone, Copy)]
enum StartDecision {
    Active,
    MissingPower,
    InactivePower,
    MissingSpeed,
    Stopped,
    Eligible,
}

impl StartDecision {
    fn name(self) -> &'static str {
        match self {
            Self::Active => "already_active",
            Self::MissingPower => "power_missing",
            Self::InactivePower => "power_not_awake",
            Self::MissingSpeed => "speed_missing",
            Self::Stopped => "speed_below_threshold",
            Self::Eligible => "eligible",
        }
    }
}

struct StartAnchor {
    odometer: Option<f64>,
    at: DateTime<Utc>,
    parked: Option<TrackPoint>,
}

impl StartAnchor {
    fn resolve(
        state: &mut TripDetectorState,
        event: &TelemetryEvent,
        prior_odometer: Option<f64>,
    ) -> Self {
        let fresh = |at: DateTime<Utc>| {
            at <= event.ts && (event.ts - at).num_seconds() <= GEAR_ANCHOR_MAX_AGE_SECS
        };
        let shifted_at = state.gear_at.take().filter(|at| fresh(*at));
        let shifted_odometer = state.gear_odometer.take().filter(|_| shifted_at.is_some());
        let current = event.odometer_miles.or(prior_odometer);
        let odometer = shifted_odometer
            .filter(|gear| current.is_none_or(|now| *gear <= now))
            .or_else(|| {
                prior_odometer.filter(|prior| event.odometer_miles.is_some_and(|now| now > *prior))
            })
            .or(current);
        let backdated = matches!((odometer, current), (Some(start), Some(now)) if start < now);
        let parked = state
            .last_parked_fix
            .take()
            .filter(|fix| backdated && fresh(fix.ts));
        Self {
            odometer,
            at: if backdated {
                shifted_at.unwrap_or(event.ts)
            } else {
                event.ts
            },
            parked,
        }
    }
}

fn track_point(event: &TelemetryEvent, speed_mph: f64) -> Option<TrackPoint> {
    valid_location_pair(event.latitude, event.longitude).map(|(lat, lng)| TrackPoint {
        ts: event.ts,
        lat,
        lng,
        speed_mph,
        altitude_m: event.altitude_m,
    })
}

impl TripDetectorState {
    pub fn new(vehicle_id: Uuid) -> Self {
        Self {
            vehicle_id,
            ..Default::default()
        }
    }

    /// Identity carried by live telemetry and durable checkpoints.
    pub fn active_trip_id(&self) -> Option<Uuid> {
        self.active_trip_id
    }

    pub(super) fn vehicle_id(&self) -> Uuid {
        self.vehicle_id
    }

    pub(super) fn resume_after_restart(&mut self) {
        // Downtime cannot connect energy samples, elevations, or parked anchors.
        self.last_regen_ts = None;
        self.last_altitude = None;
        self.gear_at = None;
        self.gear_odometer = None;
        self.last_parked_fix = None;
    }

    pub(super) fn close_interrupted_trip(&mut self, at: DateTime<Utc>) -> TripEvent {
        let result = if self.active_trip_id.is_some() {
            self.close_trip(at)
        } else {
            TripEvent::NoChange
        };
        *self = Self::new(self.vehicle_id);
        result
    }

    fn decision(&self, event: &TelemetryEvent) -> StartDecision {
        if self.active_trip_id.is_some() {
            return StartDecision::Active;
        }
        match event.power_state {
            None => StartDecision::MissingPower,
            Some(PowerState::Drive | PowerState::Go | PowerState::Ready) => match event.speed_mph {
                None => StartDecision::MissingSpeed,
                Some(speed) if speed > STOPPED_SPEED_MPH => StartDecision::Eligible,
                Some(_) => StartDecision::Stopped,
            },
            Some(_) => StartDecision::InactivePower,
        }
    }

    /// Uses the same typed decision as process without exposing trip contents.
    pub fn start_decision(&self, event: &TelemetryEvent) -> &'static str {
        self.decision(event).name()
    }

    pub fn process(&mut self, event: &TelemetryEvent) -> TripEvent {
        let decision = self.decision(event);
        let prior_odometer = self.last_odometer;
        self.observe_vehicle(event, prior_odometer);
        match decision {
            StartDecision::Eligible => {
                let anchor = StartAnchor::resolve(self, event, prior_odometer);
                self.begin_trip(event, anchor)
            }
            StartDecision::Active => {
                self.accumulate(event);
                self.advance_regen_clock(event);
                if event
                    .speed_mph
                    .is_some_and(|speed| speed > STOPPED_SPEED_MPH)
                {
                    self.last_moving_at = Some(event.ts);
                }
                let stopped = self
                    .last_moving_at
                    .is_some_and(|last| (event.ts - last).num_seconds() > STOPPED_DURATION_SECS);
                if matches!(event.power_state, Some(PowerState::Sleep)) || stopped {
                    self.close_trip(event.ts)
                } else {
                    TripEvent::NoChange
                }
            }
            _ => {
                self.advance_regen_clock(event);
                if !self.in_gear
                    && !event
                        .speed_mph
                        .is_some_and(|speed| speed > STOPPED_SPEED_MPH)
                {
                    if let Some(point) = track_point(event, 0.0) {
                        self.last_parked_fix = Some(point);
                    }
                }
                TripEvent::NoChange
            }
        }
    }

    fn observe_vehicle(&mut self, event: &TelemetryEvent, prior_odometer: Option<f64>) {
        self.last_soc = event.battery_level.or(self.last_soc);
        self.battery_capacity = event.battery_capacity_wh.or(self.battery_capacity);
        self.last_odometer = event.odometer_miles.or(self.last_odometer);
        self.last_range = event.distance_to_empty_mi.or(self.last_range);
        if let Some(power) = &event.power_state {
            let in_gear = matches!(power, PowerState::Drive | PowerState::Go);
            match (self.in_gear, in_gear) {
                (false, true) => {
                    self.gear_odometer = prior_odometer.or(event.odometer_miles);
                    self.gear_at = Some(event.ts);
                }
                (_, false) => {
                    self.gear_odometer = None;
                    self.gear_at = None;
                }
                (true, true) => {}
            }
            self.in_gear = in_gear;
        }
    }

    fn begin_trip(&mut self, event: &TelemetryEvent, anchor: StartAnchor) -> TripEvent {
        let trip_id = Uuid::new_v4();
        self.active_trip_id = Some(trip_id);
        self.trip_started_at = Some(anchor.at);
        self.start_odometer = anchor.odometer;
        self.soc_at_start = self.last_soc;
        self.range_at_start = self.last_range;
        self.last_moving_at = Some(event.ts);
        self.last_altitude = event.altitude_m;
        self.power_max = event.power_kw;
        self.power_min = event.power_kw;
        self.elevation_gain_acc = 0.0;
        self.elevation_loss_acc = 0.0;
        self.inside_temp_sum = 0.0;
        self.inside_temp_count = 0;
        self.outside_temp_sum = 0.0;
        self.outside_temp_count = 0;
        self.regen_wh_acc = 0.0;
        // The start is the first integration boundary, including partial frames.
        self.last_regen_ts = Some(event.ts);
        self.track_points.extend(anchor.parked);
        self.track_points
            .extend(track_point(event, event.speed_mph.unwrap_or(0.0)));
        TripEvent::TripStarted {
            trip_id,
            started_at: anchor.at,
        }
    }

    fn accumulate(&mut self, event: &TelemetryEvent) {
        self.track_points
            .extend(track_point(event, event.speed_mph.unwrap_or(0.0)));
        if let Some(mode) = &event.drive_mode {
            self.drive_modes.push(mode.as_str().to_owned());
        }
        if let Some(kw) = event.power_kw {
            self.power_max = Some(self.power_max.map_or(kw, |prior| prior.max(kw)));
            self.power_min = Some(self.power_min.map_or(kw, |prior| prior.min(kw)));
        }
        if let Some(altitude) = event.altitude_m {
            if let Some(prior) = self.last_altitude.replace(altitude) {
                let delta = altitude - prior;
                if delta > 0.0 {
                    self.elevation_gain_acc += delta;
                } else {
                    self.elevation_loss_acc += -delta;
                }
            }
        }
        if let Some(temperature) = event.cabin_temp_c {
            self.inside_temp_sum += temperature;
            self.inside_temp_count += 1;
        }
        if let Some(temperature) = event.outside_temp_c {
            self.outside_temp_sum += temperature;
            self.outside_temp_count += 1;
        }
        if let (Some(prior), Some(kw)) = (self.last_regen_ts, event.regen_power_kw) {
            if kw.is_finite() && kw < 0.0 {
                let hours = ((event.ts - prior).num_milliseconds() as f64 / 3_600_000.0).max(0.0);
                self.regen_wh_acc += kw.abs() * 1000.0 * hours;
            }
        }
    }

    fn advance_regen_clock(&mut self, event: &TelemetryEvent) {
        if event.regen_power_kw.is_some_and(f64::is_finite)
            && self.last_regen_ts.is_none_or(|prior| event.ts >= prior)
        {
            self.last_regen_ts = Some(event.ts);
        }
    }

    fn close_trip(&mut self, ended_at: DateTime<Utc>) -> TripEvent {
        let finished = std::mem::replace(self, Self::new(self.vehicle_id));
        // Only vehicle observations and the next start anchor survive completion.
        self.last_soc = finished.last_soc;
        self.battery_capacity = finished.battery_capacity;
        self.last_odometer = finished.last_odometer;
        self.last_range = finished.last_range;
        self.in_gear = finished.in_gear;
        self.last_parked_fix = finished.last_parked_fix;
        (self.gear_odometer, self.gear_at) = if self.in_gear {
            (self.last_odometer, Some(ended_at))
        } else {
            (finished.gear_odometer, finished.gear_at)
        };
        TripEvent::TripEnded {
            trip: CompletedTripData {
                trip_id: finished.active_trip_id.unwrap_or_else(Uuid::new_v4),
                vehicle_id: finished.vehicle_id,
                started_at: finished.trip_started_at.unwrap_or(ended_at),
                ended_at,
                points: finished.track_points,
                soc_start: finished.soc_at_start,
                soc_end: finished.last_soc,
                battery_capacity_wh: finished.battery_capacity,
                start_odometer_mi: finished.start_odometer,
                end_odometer_mi: finished.last_odometer,
                range_start_mi: finished.range_at_start,
                range_end_mi: finished.last_range,
                power_max_kw: finished.power_max,
                power_min_kw: finished.power_min,
                elevation_gain_m: (finished.elevation_gain_acc > 0.0)
                    .then_some(finished.elevation_gain_acc),
                elevation_loss_m: (finished.elevation_loss_acc > 0.0)
                    .then_some(finished.elevation_loss_acc),
                inside_temp_avg_c: (finished.inside_temp_count > 0)
                    .then(|| finished.inside_temp_sum / finished.inside_temp_count as f64),
                outside_temp_avg_c: (finished.outside_temp_count > 0)
                    .then(|| finished.outside_temp_sum / finished.outside_temp_count as f64),
                regen_wh: (finished.regen_wh_acc > 0.0).then_some(finished.regen_wh_acc),
                dominant_drive_mode: mode_of(&finished.drive_modes),
            },
        }
    }
}

fn mode_of(v: &[String]) -> Option<String> {
    let mut ordered: Vec<&str> = v.iter().map(String::as_str).collect();
    ordered.sort_unstable();
    let counts: std::collections::HashMap<&str, usize> = ordered
        .chunk_by(|left, right| left == right)
        .map(|group| (group[0], group.len()))
        .collect();
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value.to_owned())
}

/// Compute distance from odometer delta if both endpoints are available.
/// Falls back to GPS haversine if odometer is missing or the delta is zero.
pub fn compute_distance_odometer_or_gps(
    start_odo: Option<f64>,
    end_odo: Option<f64>,
    points: &[TrackPoint],
) -> f64 {
    start_odo
        .zip(end_odo)
        .map(|(start, end)| end - start)
        .filter(|distance| *distance > 0.0)
        .unwrap_or_else(|| compute_distance_miles(points))
}

/// Estimate trip energy using a ranked ensemble strategy.
/// Returns `(energy_wh, strategy_name)` or `None` if no strategy applies.
pub fn compute_trip_energy(
    soc_start: Option<f64>,
    soc_end: Option<f64>,
    battery_capacity_wh: Option<f64>,
    range_start_mi: Option<f64>,
    range_end_mi: Option<f64>,
    distance_miles: f64,
    historical_wh_per_mi: Option<f64>,
) -> Option<(f64, &'static str)> {
    let soc_estimate =
        soc_start
            .zip(soc_end)
            .zip(battery_capacity_wh)
            .and_then(|((start, end), capacity)| {
                let loss = start - end;
                (loss >= 1.0).then(|| ((loss / 100.0) * capacity, "soc_delta"))
            });
    soc_estimate
        .or_else(|| {
            range_start_mi
                .zip(range_end_mi)
                .zip(historical_wh_per_mi)
                .and_then(|((start, end), efficiency)| {
                    let loss = start - end;
                    (loss >= 1.0).then_some((loss * efficiency, "range_delta"))
                })
        })
        .or_else(|| {
            historical_wh_per_mi
                .filter(|_| distance_miles > 0.0)
                .map(|efficiency| (distance_miles * efficiency, "historical"))
        })
}

pub fn compute_distance_miles(points: &[TrackPoint]) -> f64 {
    points
        .windows(2)
        .map(|w| haversine_miles(w[0].lat, w[0].lng, w[1].lat, w[1].lng))
        .sum()
}

pub fn haversine_miles(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 3_958.8;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

#[cfg(test)]
#[path = "trip_domain_contract_tests.rs"]
mod trip_domain_contract_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::telemetry::PowerState;
    use chrono::Duration;

    fn mk_event_base(power: PowerState, speed: f64, offset_secs: i64) -> TelemetryEvent {
        let base: DateTime<Utc> = "2024-01-15T08:00:00Z".parse().unwrap();
        let ts = base + chrono::Duration::seconds(offset_secs);
        TelemetryEvent {
            latitude: Some(30.267),
            longitude: Some(-97.743),
            speed_mph: Some(speed),
            battery_level: Some(80.0),
            battery_capacity_wh: Some(135_000.0),
            power_state: Some(power),
            ..TelemetryEvent::empty(Uuid::nil(), ts)
        }
    }

    fn mk_event(power: PowerState, speed: f64, offset_secs: i64) -> TelemetryEvent {
        mk_event_base(power, speed, offset_secs)
    }

    #[test]
    fn start_diagnostics_match_actual_detector_decisions() {
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut sample = TelemetryEvent::empty(Uuid::nil(), Utc::now());
        assert_eq!(detector.start_decision(&sample), "power_missing");
        sample.power_state = Some(PowerState::Sleep);
        assert_eq!(detector.start_decision(&sample), "power_not_awake");
        sample.power_state = Some(PowerState::Go);
        assert_eq!(detector.start_decision(&sample), "speed_missing");
        for speed in [0.0, 2.0, f64::NAN] {
            sample.speed_mph = Some(speed);
            assert_eq!(detector.start_decision(&sample), "speed_below_threshold");
            assert_eq!(detector.process(&sample).transition_name(), "no_change");
        }
        sample.speed_mph = Some(3.0);
        assert_eq!(detector.start_decision(&sample), "eligible");
        assert_eq!(detector.process(&sample).transition_name(), "started");
        assert_eq!(detector.start_decision(&sample), "already_active");
        sample.power_state = Some(PowerState::Sleep);
        assert_eq!(detector.process(&sample).transition_name(), "ended");
    }

    #[test]
    fn partial_state_frames_do_not_shorten_regen_intervals() {
        let at = Utc::now();
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut sample = TelemetryEvent::empty(Uuid::nil(), at);
        sample.power_state = Some(PowerState::Drive);
        sample.speed_mph = Some(30.0);
        sample.regen_power_kw = Some(0.0);
        assert_eq!(detector.process(&sample).transition_name(), "started");
        detector.process(&TelemetryEvent::empty(
            Uuid::nil(),
            at + Duration::seconds(4),
        ));
        sample.ts = at + Duration::seconds(5);
        sample.regen_power_kw = Some(-36.0);
        detector.process(&sample);
        sample.ts = at + Duration::seconds(3);
        detector.process(&sample); // Late sample cannot move the boundary back.
        sample.ts = at + Duration::seconds(10);
        sample.regen_power_kw = Some(-36.0);
        detector.process(&sample);
        sample.ts = at + Duration::seconds(11);
        sample.power_state = Some(PowerState::Sleep);
        sample.regen_power_kw = None;
        let TripEvent::TripEnded { trip } = detector.process(&sample) else {
            panic!("expected completed trip");
        };
        assert_eq!(trip.regen_wh, Some(100.0));
    }

    #[test]
    fn partial_trip_start_keeps_available_battery_and_range() {
        let at = Utc::now();
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut baseline = TelemetryEvent::empty(Uuid::nil(), at);
        baseline.battery_level = Some(80.0);
        baseline.distance_to_empty_mi = Some(240.0);
        detector.process(&baseline);
        let mut motion = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(30));
        motion.power_state = Some(PowerState::Go);
        motion.speed_mph = Some(30.0);
        assert_eq!(detector.process(&motion).transition_name(), "started");
        motion.ts += Duration::seconds(30);
        motion.power_state = Some(PowerState::Sleep);
        motion.battery_level = Some(79.0);
        motion.distance_to_empty_mi = Some(237.0);
        let TripEvent::TripEnded { trip } = detector.process(&motion) else {
            panic!("expected completed trip");
        };
        assert_eq!(trip.soc_start, Some(80.0));
        assert_eq!(trip.soc_end, Some(79.0));
        assert_eq!(trip.range_start_mi, Some(240.0));
        assert_eq!(trip.range_end_mi, Some(237.0));
    }

    #[test]
    fn trip_starts_moving_awake() {
        let mut d = TripDetectorState::new(Uuid::nil());
        assert!(matches!(
            d.process(&mk_event(PowerState::Drive, 35.0, 0)),
            TripEvent::TripStarted { .. }
        ));
    }

    #[test]
    fn active_trip_id_exposed() {
        let mut d = TripDetectorState::new(Uuid::nil());
        assert!(d.active_trip_id().is_none());
        d.process(&mk_event(PowerState::Drive, 35.0, 0));
        assert!(d.active_trip_id().is_some());
    }

    #[test]
    fn no_trip_when_stationary() {
        let mut d = TripDetectorState::new(Uuid::nil());
        assert_eq!(
            d.process(&mk_event(PowerState::Ready, 0.0, 0)),
            TripEvent::NoChange
        );
    }

    #[test]
    fn trip_ends_on_sleep() {
        let mut d = TripDetectorState::new(Uuid::nil());
        d.process(&mk_event(PowerState::Drive, 35.0, 0));
        assert!(matches!(
            d.process(&mk_event(PowerState::Sleep, 0.0, 10)),
            TripEvent::TripEnded { .. }
        ));
    }

    #[test]
    fn trip_ends_after_5min_stopped() {
        let mut d = TripDetectorState::new(Uuid::nil());
        d.process(&mk_event(PowerState::Drive, 35.0, 0));
        d.process(&mk_event(PowerState::Ready, 1.5, 60));
        assert!(matches!(
            d.process(&mk_event(PowerState::Ready, 0.0, 400)),
            TripEvent::TripEnded { .. }
        ));
    }

    #[test]
    fn trip_start_odometer_is_the_reading_before_motion() {
        let mut d = TripDetectorState::new(Uuid::nil());
        let mut parked = mk_event(PowerState::Ready, 0.0, 0);
        parked.odometer_miles = Some(100.0);
        d.process(&parked);
        let mut moved = mk_event(PowerState::Go, 30.0, 60);
        moved.odometer_miles = Some(100.62);
        d.process(&moved);
        let TripEvent::TripEnded { trip } = d.process(&mk_event(PowerState::Sleep, 0.0, 120))
        else {
            panic!("expected TripEnded");
        };
        assert_eq!(trip.start_odometer_mi, Some(100.0));
    }

    #[test]
    fn trip_start_without_odometer_uses_last_known_reading() {
        let mut d = TripDetectorState::new(Uuid::nil());
        let mut parked = mk_event(PowerState::Ready, 0.0, 0);
        parked.odometer_miles = Some(100.0);
        d.process(&parked);
        d.process(&mk_event(PowerState::Go, 30.0, 60));
        let TripEvent::TripEnded { trip } = d.process(&mk_event(PowerState::Sleep, 0.0, 120))
        else {
            panic!("expected TripEnded");
        };
        assert_eq!(trip.start_odometer_mi, Some(100.0));
    }

    #[test]
    fn trip_resumed_in_gear_starts_at_previous_trip_end() {
        let mut d = TripDetectorState::new(Uuid::nil());
        let mut parked = mk_event(PowerState::Ready, 0.0, 0);
        parked.odometer_miles = Some(100.0);
        d.process(&parked);
        let mut first = mk_event(PowerState::Drive, 30.0, 60);
        first.odometer_miles = Some(101.0);
        d.process(&first);
        let mut stopped = mk_event(PowerState::Drive, 0.0, 400);
        stopped.odometer_miles = Some(101.0);
        let TripEvent::TripEnded { trip } = d.process(&stopped) else {
            panic!("expected TripEnded");
        };
        assert_eq!(trip.start_odometer_mi, Some(100.0));

        let mut resumed = mk_event(PowerState::Drive, 30.0, 500);
        resumed.odometer_miles = Some(101.6);
        d.process(&resumed);
        let TripEvent::TripEnded { trip } = d.process(&mk_event(PowerState::Sleep, 0.0, 600))
        else {
            panic!("expected TripEnded");
        };
        assert_eq!(trip.start_odometer_mi, Some(101.0));
    }

    /// Dense (R1) telemetry reports speed and odometer on every sample, so the
    /// gear-shift start odometer must not change the measured distance.
    #[test]
    fn dense_r1_trip_distance_is_unchanged_by_gear_start_odometer() {
        use crate::ingestion::trip_signals::TripSignalFusion;

        let mut fusion = TripSignalFusion::new(false);
        let mut d = TripDetectorState::new(Uuid::nil());
        let mut samples = Vec::new();
        let mut parked = mk_event(PowerState::Ready, 0.0, 0);
        parked.odometer_miles = Some(200.0);
        samples.push(parked);
        let mut shifted = mk_event(PowerState::Drive, 0.0, 10);
        shifted.odometer_miles = Some(200.0);
        samples.push(shifted);
        for step in 1..=20 {
            let mut moving = mk_event(PowerState::Drive, 30.0, 10 + step * 30);
            moving.odometer_miles = Some(200.0 + 0.25 * step as f64);
            samples.push(moving);
        }
        let mut sleep = mk_event(PowerState::Sleep, 0.0, 700);
        sleep.odometer_miles = Some(205.0);
        samples.push(sleep);

        let mut ended = None;
        for sample in &samples {
            if let TripEvent::TripEnded { trip } = d.process(&fusion.fuse(sample)) {
                ended = Some(trip);
            }
        }
        let trip = ended.expect("trip ended");
        // The start odometer is the shift reading, so the start time is the
        // shift too, 30 seconds before the first moving sample.
        assert_eq!(trip.started_at, samples[1].ts);
        assert_eq!(trip.start_odometer_mi, Some(200.0));
        assert_eq!(trip.end_odometer_mi, Some(205.0));
        let distance = compute_distance_odometer_or_gps(
            trip.start_odometer_mi,
            trip.end_odometer_mi,
            &trip.points,
        );
        assert!((distance - 5.0).abs() < 1e-9, "{distance}");
    }

    #[test]
    fn trip_data_carries_trip_id() {
        let mut d = TripDetectorState::new(Uuid::nil());
        let ev = d.process(&mk_event(PowerState::Drive, 35.0, 0));
        let trip_id = match ev {
            TripEvent::TripStarted { trip_id, .. } => trip_id,
            _ => panic!("expected TripStarted"),
        };
        match d.process(&mk_event(PowerState::Sleep, 0.0, 10)) {
            TripEvent::TripEnded { trip } => assert_eq!(trip.trip_id, trip_id),
            _ => panic!("expected TripEnded"),
        }
    }

    #[test]
    fn ignores_zero_zero_location_samples_until_valid_coordinates_arrive() {
        let mut d = TripDetectorState::new(Uuid::nil());

        let mut start = mk_event(PowerState::Drive, 35.0, 0);
        start.latitude = Some(0.0);
        start.longitude = Some(0.0);
        assert!(matches!(d.process(&start), TripEvent::TripStarted { .. }));

        let mut mid = mk_event(PowerState::Drive, 36.0, 60);
        mid.latitude = Some(30.267);
        mid.longitude = Some(-97.743);
        assert!(matches!(d.process(&mid), TripEvent::NoChange));

        let mut end = mk_event(PowerState::Sleep, 0.0, 600);
        end.latitude = Some(30.268);
        end.longitude = Some(-97.742);
        let TripEvent::TripEnded { trip } = d.process(&end) else {
            panic!("expected TripEnded");
        };

        assert_eq!(
            trip.points.first().map(|p| (p.lat, p.lng)),
            Some((30.267, -97.743))
        );
        assert_eq!(
            trip.points.last().map(|p| (p.lat, p.lng)),
            Some((30.268, -97.742))
        );
    }

    #[test]
    fn energy_strategy_soc_delta() {
        let (wh, strategy) = compute_trip_energy(
            Some(90.0),
            Some(70.0),
            Some(135_000.0),
            None,
            None,
            50.0,
            None,
        )
        .unwrap();
        assert_eq!(strategy, "soc_delta");
        assert!((wh - 27_000.0).abs() < 1.0);
    }

    #[test]
    fn energy_strategy_range_delta_fallback() {
        let (wh, strategy) = compute_trip_energy(
            Some(80.0),
            Some(79.5),
            None, // SOC delta < 1 %
            Some(250.0),
            Some(230.0),
            20.0,
            Some(400.0),
        )
        .unwrap();
        assert_eq!(strategy, "range_delta");
        assert!((wh - 8_000.0).abs() < 10.0);
    }

    #[test]
    fn energy_strategy_historical_fallback() {
        let (wh, strategy) =
            compute_trip_energy(None, None, None, None, None, 10.0, Some(350.0)).unwrap();
        assert_eq!(strategy, "historical");
        assert!((wh - 3_500.0).abs() < 1.0);
    }

    #[test]
    fn haversine_austin_san_antonio() {
        let d = haversine_miles(30.267_153, -97.743_061, 29.424_122, -98.493_629);
        assert!(
            (d - 73.6).abs() < 1.0,
            "Expected ~73.6 straight-line miles, got {d}"
        );
    }
}

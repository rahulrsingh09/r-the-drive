//! Join timestamped partial readings for every vehicle without rewriting stored
//! telemetry. Direct speed wins; plausible GNSS/odometer motion fills gaps.
//! Power retention follows the source: Parallax reports changes, periodic legacy
//! samples expire. History replay infers the source policy from each row's shape.

use super::{location::valid_location_pair, trip_detector::haversine_miles};
use crate::models::telemetry::{PowerState, TelemetryEvent};
use chrono::{DateTime, Duration, Utc};

const POWER_MAX_AGE: Duration = Duration::minutes(2);
const FIX_MAX_AGE: Duration = Duration::minutes(2);
const MAX_FIX_INTERVAL: Duration = Duration::minutes(5);
const MAX_PLAUSIBLE_MPH: f64 = 130.0;
const MOVING_MPH: f64 = 2.0;

#[derive(Debug)]
pub struct FusionDiagnostics {
    pub power_origin: &'static str,
    pub power_policy: &'static str,
    pub power_outcome: &'static str,
    pub reported_power_outcome: &'static str,
    pub speed_origin: &'static str,
    pub reported_speed_outcome: &'static str,
    pub gnss_outcome: &'static str,
    pub odometer_outcome: &'static str,
    pub gnss_interval_seconds: Option<i64>,
    pub gnss_distance_miles: Option<f64>,
    pub odometer_interval_seconds: Option<i64>,
    pub odometer_delta_miles: Option<f64>,
    pub moving_segments: u8,
}

impl Default for FusionDiagnostics {
    fn default() -> Self {
        Self {
            power_origin: "absent",
            power_policy: "periodic",
            power_outcome: "missing",
            reported_power_outcome: "missing",
            speed_origin: "absent",
            reported_speed_outcome: "missing",
            gnss_outcome: "missing",
            odometer_outcome: "missing",
            gnss_interval_seconds: None,
            gnss_distance_miles: None,
            odometer_interval_seconds: None,
            odometer_delta_miles: None,
            moving_segments: 0,
        }
    }
}

#[derive(Clone, Copy, Default)]
enum PowerRetention {
    #[default]
    Periodic,
    UntilChanged,
}

impl PowerRetention {
    fn from_source(change_only: bool) -> Self {
        if change_only {
            Self::UntilChanged
        } else {
            Self::Periodic
        }
    }

    fn retains(self, age: Duration) -> bool {
        matches!(self, Self::UntilChanged) || age <= POWER_MAX_AGE
    }

    fn diagnostic(self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::UntilChanged => "change_only",
        }
    }
}

struct ObservedPower {
    value: PowerState,
    at: DateTime<Utc>,
    retention: PowerRetention,
}

#[derive(Default)]
struct PowerMemory(Option<ObservedPower>);

impl PowerMemory {
    fn join(
        &mut self,
        sample: &TelemetryEvent,
        retention: PowerRetention,
        joined: &mut TelemetryEvent,
        diagnostics: &mut FusionDiagnostics,
    ) {
        if let Some(value) = &sample.power_state {
            let at = sample.power_state_ts.unwrap_or(sample.ts);
            let acceptable = at <= sample.ts + Duration::seconds(5)
                && retention.retains(sample.ts - at)
                && self.0.as_ref().is_none_or(|prior| at >= prior.at);
            if acceptable {
                self.0 = Some(ObservedPower {
                    value: value.clone(),
                    at,
                    retention,
                });
                diagnostics.reported_power_outcome = "accepted";
                diagnostics.power_origin = "reported";
                diagnostics.power_outcome = "accepted";
            } else {
                joined.power_state = None;
                joined.power_state_ts = None;
                diagnostics.reported_power_outcome = "stale_future_or_out_of_order";
                diagnostics.power_outcome = "stale_future_or_out_of_order";
            }
        }
        let Some(cached) = &self.0 else {
            return;
        };
        diagnostics.power_policy = cached.retention.diagnostic();
        if joined.power_state.is_none() {
            if sample.ts >= cached.at && cached.retention.retains(sample.ts - cached.at) {
                joined.power_state = Some(cached.value.clone());
                joined.power_state_ts = Some(cached.at);
                diagnostics.power_origin = "carried_forward";
                diagnostics.power_outcome = "accepted";
            } else {
                diagnostics.power_outcome = "stale_or_future";
            }
        }
    }
}

enum SelectedSpeed {
    Missing(Option<DateTime<Utc>>),
    Reported(f64, Option<DateTime<Utc>>),
    Gnss(f64, DateTime<Utc>),
    Odometer(f64, DateTime<Utc>),
}

impl SelectedSpeed {
    fn from_report(sample: &TelemetryEvent, diagnostics: &mut FusionDiagnostics) -> Self {
        let Some(mph) = sample.speed_mph else {
            return Self::Missing(sample.speed_mph_ts);
        };
        let at = sample.speed_mph_ts.unwrap_or(sample.ts);
        if at > sample.ts || sample.ts - at > FIX_MAX_AGE {
            diagnostics.reported_speed_outcome = "stale_or_future";
            Self::Missing(None)
        } else if !mph.is_finite() || !(0.0..=MAX_PLAUSIBLE_MPH).contains(&mph) {
            diagnostics.reported_speed_outcome = "invalid";
            Self::Missing(None)
        } else {
            diagnostics.reported_speed_outcome = "accepted";
            Self::Reported(mph, sample.speed_mph_ts)
        }
    }

    fn allows_odometer(&self) -> bool {
        matches!(self, Self::Missing(_) | Self::Gnss(0.0, _))
    }

    fn write(self, joined: &mut TelemetryEvent, diagnostics: &mut FusionDiagnostics) {
        let (mph, at, origin) = match self {
            Self::Missing(at) => (None, at, "absent"),
            Self::Reported(mph, at) => (Some(mph), at, "reported"),
            Self::Gnss(mph, at) => (Some(mph), Some(at), "gnss"),
            Self::Odometer(mph, at) => (Some(mph), Some(at), "odometer"),
        };
        joined.speed_mph = mph;
        joined.speed_mph_ts = at;
        diagnostics.speed_origin = origin;
    }
}

#[derive(Default)]
enum MotionEvidence {
    #[default]
    Stationary,
    Candidate,
    Confirmed(u8),
}

impl MotionEvidence {
    fn segments(&self) -> u8 {
        match self {
            Self::Stationary => 0,
            Self::Candidate => 1,
            Self::Confirmed(count) => *count,
        }
    }

    fn observe(&mut self, mph: f64) -> (Option<f64>, &'static str) {
        if !mph.is_finite() || mph > MAX_PLAUSIBLE_MPH {
            *self = Self::Stationary;
            return (None, "implausible");
        }
        if mph <= MOVING_MPH {
            *self = Self::Stationary;
            return (Some(0.0), "stationary");
        }
        *self = match self {
            Self::Stationary => Self::Candidate,
            Self::Candidate => Self::Confirmed(2),
            Self::Confirmed(count) => Self::Confirmed(count.saturating_add(1)),
        };
        match self {
            Self::Confirmed(_) => (Some(mph), "derived"),
            _ => (Some(0.0), "provisional_zero"),
        }
    }
}

#[derive(Clone, Copy)]
struct PositionFix {
    latitude: f64,
    longitude: f64,
    at: DateTime<Utc>,
}

#[derive(Default)]
struct GnssHistory {
    last: Option<PositionFix>,
    motion: MotionEvidence,
}

impl GnssHistory {
    fn join(
        &mut self,
        sample: &TelemetryEvent,
        speed: &mut SelectedSpeed,
        joined: &mut TelemetryEvent,
        diagnostics: &mut FusionDiagnostics,
    ) {
        if matches!(speed, SelectedSpeed::Reported(..)) {
            self.motion = MotionEvidence::Stationary;
        }
        let coordinates = valid_location_pair(sample.latitude, sample.longitude)
            .filter(|(lat, lon)| (-90.0..=90.0).contains(lat) && (-180.0..=180.0).contains(lon));
        let Some((latitude, longitude)) = coordinates else {
            if sample.latitude.is_some() || sample.longitude.is_some() {
                Self::discard_location(joined);
                diagnostics.gnss_outcome = "invalid_or_incomplete";
            }
            return;
        };
        let fix = PositionFix {
            latitude,
            longitude,
            at: sample.location_ts.unwrap_or(sample.ts),
        };
        if fix.at > sample.ts + Duration::seconds(5)
            || sample.ts - fix.at > FIX_MAX_AGE
            || self.last.is_some_and(|prior| fix.at <= prior.at)
        {
            Self::discard_location(joined);
            diagnostics.gnss_outcome = "stale_future_or_out_of_order";
            return;
        }
        diagnostics.gnss_outcome = "baseline";
        if let Some(prior) = self.last.replace(fix) {
            let seconds = (fix.at - prior.at).num_seconds();
            diagnostics.gnss_interval_seconds = Some(seconds);
            if !(5..=MAX_FIX_INTERVAL.num_seconds()).contains(&seconds) {
                self.motion = MotionEvidence::Stationary;
                diagnostics.gnss_outcome = "interval_out_of_range";
                return;
            }
            let miles =
                haversine_miles(prior.latitude, prior.longitude, fix.latitude, fix.longitude);
            diagnostics.gnss_distance_miles = Some(miles);
            if matches!(speed, SelectedSpeed::Reported(..)) {
                diagnostics.gnss_outcome = "reported_speed_precedence";
                return;
            }
            let (mph, outcome) = self.motion.observe(miles * 3600.0 / seconds as f64);
            diagnostics.gnss_outcome = outcome;
            if let Some(mph) = mph {
                *speed = SelectedSpeed::Gnss(mph, fix.at);
            }
        }
    }

    fn discard_location(joined: &mut TelemetryEvent) {
        joined.latitude = None;
        joined.longitude = None;
        joined.location_ts = None;
    }
}

#[derive(Clone, Copy)]
struct OdometerReading {
    miles: f64,
    at: DateTime<Utc>,
}

#[derive(Default)]
struct OdometerHistory(Option<OdometerReading>);

impl OdometerHistory {
    fn join(
        &mut self,
        sample: &TelemetryEvent,
        speed: &mut SelectedSpeed,
        diagnostics: &mut FusionDiagnostics,
    ) {
        let Some(miles) = sample
            .odometer_miles
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            if sample.odometer_miles.is_some() {
                diagnostics.odometer_outcome = "invalid";
            }
            return;
        };
        let reading = OdometerReading {
            miles,
            at: sample.odometer_miles_ts.unwrap_or(sample.ts),
        };
        if reading.at > sample.ts + Duration::seconds(5)
            || self.0.is_some_and(|prior| reading.at <= prior.at)
        {
            diagnostics.odometer_outcome = "future_or_out_of_order";
            return;
        }
        diagnostics.odometer_outcome = "baseline";
        let Some(prior) = self.0.replace(reading) else {
            return;
        };
        let seconds = (reading.at - prior.at).num_seconds();
        let delta = reading.miles - prior.miles;
        diagnostics.odometer_interval_seconds = Some(seconds);
        diagnostics.odometer_delta_miles = Some(delta);
        diagnostics.odometer_outcome = if !speed.allows_odometer() {
            "speed_precedence"
        } else if sample.ts - reading.at > FIX_MAX_AGE {
            "stale"
        } else if !(5..=MAX_FIX_INTERVAL.num_seconds()).contains(&seconds) {
            "interval_out_of_range"
        } else if delta <= 0.0 {
            "no_increase"
        } else {
            let mph = delta * 3600.0 / seconds as f64;
            if mph.is_finite() && mph <= MAX_PLAUSIBLE_MPH {
                *speed = SelectedSpeed::Odometer(mph, reading.at);
                "derived"
            } else {
                "implausible"
            }
        };
    }
}

#[derive(Default)]
struct StreamSignals {
    power: PowerMemory,
    gnss: GnssHistory,
    odometer: OdometerHistory,
}

/// One instance belongs to one vehicle stream, as scoped by the ingestion worker.
#[derive(Default)]
pub struct TripSignalFusion {
    default_retention: PowerRetention,
    stream: StreamSignals,
}

impl TripSignalFusion {
    /// Set a default source policy for callers with a known uniform source.
    pub fn new(change_only_power: bool) -> Self {
        Self {
            default_retention: PowerRetention::from_source(change_only_power),
            ..Self::default()
        }
    }

    pub fn fuse(&mut self, sample: &TelemetryEvent) -> TelemetryEvent {
        self.join(sample, self.default_retention).0
    }

    /// The accepted power observation owns retention, independently of later sources.
    pub fn fuse_from(
        &mut self,
        sample: &TelemetryEvent,
        change_only_power: bool,
    ) -> (TelemetryEvent, FusionDiagnostics) {
        self.join(sample, PowerRetention::from_source(change_only_power))
    }

    fn join(
        &mut self,
        sample: &TelemetryEvent,
        retention: PowerRetention,
    ) -> (TelemetryEvent, FusionDiagnostics) {
        let mut joined = sample.clone();
        let mut diagnostics = FusionDiagnostics::default();
        self.stream
            .power
            .join(sample, retention, &mut joined, &mut diagnostics);
        let mut speed = SelectedSpeed::from_report(sample, &mut diagnostics);
        self.stream
            .gnss
            .join(sample, &mut speed, &mut joined, &mut diagnostics);
        self.stream
            .odometer
            .join(sample, &mut speed, &mut diagnostics);
        speed.write(&mut joined, &mut diagnostics);
        diagnostics.moving_segments = self.stream.gnss.motion.segments();
        (joined, diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use crate::models::telemetry::{PowerState, TelemetryEvent};

    use super::TripSignalFusion;

    fn charge_core_at(seconds: i64) -> chrono::DateTime<Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000 + seconds, 0).unwrap()
    }

    fn charge_core_sample(seconds: i64) -> TelemetryEvent {
        TelemetryEvent::empty(Uuid::from_u128(1), charge_core_at(seconds))
    }

    fn charge_core_fix(seconds: i64, longitude: f64) -> TelemetryEvent {
        let mut sample = charge_core_sample(seconds);
        sample.latitude = Some(0.0);
        sample.longitude = Some(longitude);
        sample
    }

    #[test]
    fn charge_core_empty_diagnostics_and_passthrough_are_stable() {
        let mut sample = charge_core_sample(0);
        sample.battery_level = Some(44.0);
        sample.regen_power_kw = Some(-3.0);
        sample.cabin_temp_c = Some(21.0);
        sample.charger_status = Some("chrgr_sts_connected_no_chrg".into());
        let original = serde_json::to_value(&sample).unwrap();
        let (joined, diagnostics) = TripSignalFusion::default().fuse_from(&sample, true);
        assert_eq!(serde_json::to_value(&joined).unwrap(), original);
        assert_eq!(serde_json::to_value(&sample).unwrap(), original);
        assert_eq!(
            format!("{diagnostics:?}"),
            "FusionDiagnostics { power_origin: \"absent\", power_policy: \"periodic\", power_outcome: \"missing\", reported_power_outcome: \"missing\", speed_origin: \"absent\", reported_speed_outcome: \"missing\", gnss_outcome: \"missing\", odometer_outcome: \"missing\", gnss_interval_seconds: None, gnss_distance_miles: None, odometer_interval_seconds: None, odometer_delta_miles: None, moving_segments: 0 }"
        );
    }

    #[test]
    fn charge_core_power_age_future_and_equal_timestamp_policy_boundaries() {
        for (age_ms, accepted) in [
            (120_000, true),
            (120_001, false),
            (-5_000, true),
            (-5_001, false),
        ] {
            let mut sample = charge_core_sample(0);
            sample.power_state = Some(PowerState::Drive);
            sample.power_state_ts = Some(sample.ts - Duration::milliseconds(age_ms));
            let (joined, diag) = TripSignalFusion::default().fuse_from(&sample, false);
            assert_eq!(joined.power_state.is_some(), accepted, "{age_ms}");
            assert_eq!(
                diag.reported_power_outcome,
                if accepted {
                    "accepted"
                } else {
                    "stale_future_or_out_of_order"
                }
            );
        }
        for latched in [false, true] {
            let mut fusion = TripSignalFusion::default();
            let mut sample = charge_core_sample(0);
            sample.power_state = Some(PowerState::Drive);
            fusion.fuse_from(&sample, latched);
            let mut later = charge_core_sample(120);
            assert_eq!(
                fusion.fuse_from(&later, !latched).0.power_state,
                Some(PowerState::Drive)
            );
            later.ts += Duration::milliseconds(1);
            assert_eq!(
                fusion.fuse_from(&later, !latched).0.power_state.is_some(),
                latched
            );
            sample.power_state = Some(PowerState::Ready);
            let (joined, diag) = fusion.fuse_from(&sample, !latched);
            assert_eq!(joined.power_state, Some(PowerState::Ready));
            assert_eq!(diag.reported_power_outcome, "accepted");
            assert_eq!(
                diag.power_policy,
                if latched { "periodic" } else { "change_only" }
            );
            assert_eq!(joined.power_state_ts, None);
        }
    }

    #[test]
    fn charge_core_accepted_future_power_waits_until_observation_before_carrying() {
        let mut fusion = TripSignalFusion::new(true);
        let mut sample = charge_core_sample(0);
        sample.power_state = Some(PowerState::Go);
        sample.power_state_ts = Some(charge_core_at(5));
        assert_eq!(fusion.fuse(&sample).power_state, Some(PowerState::Go));
        let (before, diag) = fusion.fuse_from(&charge_core_sample(4), true);
        assert_eq!(before.power_state, None);
        assert_eq!(diag.power_outcome, "stale_or_future");
        assert_eq!(
            fusion.fuse(&charge_core_sample(5)).power_state,
            Some(PowerState::Go)
        );
    }

    #[test]
    fn charge_core_reported_speed_bounds_and_invalid_fallback() {
        for (speed, age_ms, outcome) in [
            (0.0, 0, "accepted"),
            (130.0, 120_000, "accepted"),
            (130.001, 0, "invalid"),
            (-0.001, 0, "invalid"),
            (f64::NAN, 0, "invalid"),
            (f64::INFINITY, 0, "invalid"),
            (f64::NEG_INFINITY, 0, "invalid"),
            (10.0, 120_001, "stale_or_future"),
            (10.0, -1, "stale_or_future"),
            (f64::NAN, -1, "stale_or_future"),
        ] {
            let mut fusion = TripSignalFusion::default();
            let mut baseline = charge_core_sample(0);
            baseline.odometer_miles = Some(100.0);
            fusion.fuse(&baseline);
            let mut sample = charge_core_sample(60);
            sample.odometer_miles = Some(101.0);
            sample.speed_mph = Some(speed);
            sample.speed_mph_ts = Some(sample.ts - Duration::milliseconds(age_ms));
            let (joined, diag) = fusion.fuse_from(&sample, false);
            assert_eq!(diag.reported_speed_outcome, outcome);
            assert_eq!(
                joined.speed_mph,
                Some(if outcome == "accepted" { speed } else { 60.0 })
            );
            assert_eq!(
                diag.speed_origin,
                if outcome == "accepted" {
                    "reported"
                } else {
                    "odometer"
                }
            );
            assert_eq!(
                joined.speed_mph_ts,
                if outcome == "accepted" {
                    sample.speed_mph_ts
                } else {
                    Some(sample.ts)
                }
            );
        }
    }

    #[test]
    fn charge_core_location_shape_age_future_and_order_boundaries() {
        for (lat, lon) in [
            (None, Some(1.0)),
            (Some(0.0), None),
            (Some(0.0), Some(0.0)),
            (Some(90.001), Some(1.0)),
            (Some(1.0), Some(-180.001)),
            (Some(f64::NAN), Some(1.0)),
            (Some(1.0), Some(f64::INFINITY)),
        ] {
            let mut sample = charge_core_sample(0);
            sample.latitude = lat;
            sample.longitude = lon;
            let (joined, diag) = TripSignalFusion::default().fuse_from(&sample, false);
            assert_eq!(
                (joined.latitude, joined.longitude, joined.location_ts),
                (None, None, None)
            );
            assert_eq!(diag.gnss_outcome, "invalid_or_incomplete");
        }
        for (lat, lon) in [(-90.0, -180.0), (90.0, 180.0), (0.0, 1.0)] {
            let mut sample = charge_core_fix(0, lon);
            sample.latitude = Some(lat);
            assert_eq!(
                TripSignalFusion::default().fuse(&sample).latitude,
                Some(lat)
            );
        }
        for (age_ms, accepted) in [
            (120_000, true),
            (120_001, false),
            (-5_000, true),
            (-5_001, false),
        ] {
            let mut sample = charge_core_fix(0, 1.0);
            sample.location_ts = Some(sample.ts - Duration::milliseconds(age_ms));
            let mut fusion = TripSignalFusion::default();
            let (joined, diag) = fusion.fuse_from(&sample, false);
            assert_eq!(joined.latitude.is_some(), accepted);
            assert_eq!(
                diag.gnss_outcome,
                if accepted {
                    "baseline"
                } else {
                    "stale_future_or_out_of_order"
                }
            );
            if accepted {
                for skew in [0, -1] {
                    sample.location_ts = sample
                        .location_ts
                        .map(|at| at + Duration::milliseconds(skew));
                    let (joined, diag) = fusion.fuse_from(&sample, false);
                    assert_eq!(joined.latitude, None);
                    assert_eq!(diag.gnss_outcome, "stale_future_or_out_of_order");
                }
            }
        }
    }

    #[test]
    fn charge_core_gnss_interval_quantization_and_motion_confirmation() {
        for (milliseconds, outcome) in [
            (4_999, "interval_out_of_range"),
            (5_000, "stationary"),
            (300_000, "stationary"),
            (300_999, "stationary"),
            (301_000, "interval_out_of_range"),
        ] {
            let mut fusion = TripSignalFusion::default();
            fusion.fuse(&charge_core_fix(0, 1.0));
            let mut sample = charge_core_fix(0, 1.0);
            sample.ts += Duration::milliseconds(milliseconds);
            let (_, diag) = fusion.fuse_from(&sample, false);
            assert_eq!(diag.gnss_outcome, outcome);
            assert_eq!(diag.gnss_interval_seconds, Some(milliseconds / 1_000));
        }
        for (mph, first, second) in [
            (1.999999, "stationary", "stationary"),
            (2.000001, "provisional_zero", "derived"),
            (129.999999, "provisional_zero", "derived"),
            (130.000001, "implausible", "implausible"),
        ] {
            let step = (mph / 60.0 / 3_958.8_f64).to_degrees();
            let mut fusion = TripSignalFusion::default();
            fusion.fuse(&charge_core_fix(0, 1.0));
            for (i, outcome) in [(1, first), (2, second)] {
                let (joined, diag) =
                    fusion.fuse_from(&charge_core_fix(i * 60, 1.0 + i as f64 * step), false);
                assert_eq!(diag.gnss_outcome, outcome, "{mph}");
                match outcome {
                    "derived" => assert!((joined.speed_mph.unwrap() - mph).abs() < 1e-8),
                    "implausible" => assert_eq!(joined.speed_mph, None),
                    _ => assert_eq!(joined.speed_mph, Some(0.0)),
                }
                assert_eq!(
                    diag.moving_segments,
                    if mph > 2.0 && mph <= 130.0 {
                        i as u8
                    } else {
                        0
                    }
                );
            }
        }
    }

    #[test]
    fn charge_core_motion_evidence_exact_thresholds_and_saturation() {
        use super::MotionEvidence;
        for (mph, value, outcome) in [
            (2.0, Some(0.0), "stationary"),
            (2.0_f64.next_up(), Some(0.0), "provisional_zero"),
            (130.0, Some(0.0), "provisional_zero"),
            (130.0_f64.next_up(), None, "implausible"),
            (f64::NAN, None, "implausible"),
            (f64::INFINITY, None, "implausible"),
        ] {
            let mut evidence = MotionEvidence::default();
            assert_eq!(evidence.observe(mph), (value, outcome));
        }
        let mut evidence = MotionEvidence::default();
        evidence.observe(130.0);
        for _ in 0..300 {
            assert_eq!(evidence.observe(130.0), (Some(130.0), "derived"));
        }
        assert_eq!(evidence.segments(), u8::MAX);
        assert_eq!(evidence.observe(2.0), (Some(0.0), "stationary"));
        assert_eq!(evidence.segments(), 0);
        assert_eq!(evidence.observe(3.0), (Some(0.0), "provisional_zero"));
    }

    #[test]
    fn charge_core_gnss_positive_speed_wins_over_odometer() {
        let mut fusion = TripSignalFusion::default();
        for i in 0..3 {
            let mut sample = charge_core_fix(i * 60, 1.0 + i as f64 * 0.01);
            sample.odometer_miles = Some(100.0 + i as f64);
            let (joined, diag) = fusion.fuse_from(&sample, false);
            if i == 2 {
                assert_eq!(diag.speed_origin, "gnss");
                assert_eq!(diag.odometer_outcome, "speed_precedence");
                assert!(joined.speed_mph.unwrap() > 2.0);
                assert!(joined.speed_mph.unwrap() < 60.0);
            }
        }
    }

    #[test]
    fn charge_core_odometer_interval_speed_and_rollback_boundaries() {
        for (seconds, miles, outcome, speed) in [
            (4, 0.01, "interval_out_of_range", None),
            (5, 0.01, "derived", Some(7.2)),
            (300, 1.0, "derived", Some(12.0)),
            (301, 1.0, "interval_out_of_range", None),
            (180, 6.5, "derived", Some(130.0)),
            (180, 6.5001, "implausible", None),
            (180, 0.1, "derived", Some(2.0)),
            (60, 0.0, "no_increase", None),
            (60, -1.0, "no_increase", None),
        ] {
            let mut fusion = TripSignalFusion::default();
            fusion.fuse(&odometer(charge_core_at(0), 100.0));
            let (joined, diag) =
                fusion.fuse_from(&odometer(charge_core_at(seconds), 100.0 + miles), false);
            assert_eq!(diag.odometer_outcome, outcome);
            assert_eq!(diag.odometer_interval_seconds, Some(seconds));
            match speed {
                Some(expected) => assert!((joined.speed_mph.unwrap() - expected).abs() < 1e-8),
                None => assert_eq!(joined.speed_mph, None),
            }
        }
        let mut fusion = TripSignalFusion::default();
        fusion.fuse(&odometer(charge_core_at(0), 100.0));
        fusion.fuse(&odometer(charge_core_at(60), 0.0));
        assert_eq!(
            fusion.fuse(&odometer(charge_core_at(120), 1.0)).speed_mph,
            Some(60.0)
        );
    }

    #[test]
    fn charge_core_odometer_invalid_age_future_and_order_boundaries() {
        for miles in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001] {
            let (_, diag) =
                TripSignalFusion::default().fuse_from(&odometer(charge_core_at(0), miles), false);
            assert_eq!(diag.odometer_outcome, "invalid");
        }
        for (age_ms, outcome) in [
            (120_000, "derived"),
            (120_001, "stale"),
            (-5_000, "derived"),
            (-5_001, "future_or_out_of_order"),
        ] {
            let mut fusion = TripSignalFusion::default();
            fusion.fuse(&odometer(charge_core_at(0), 100.0));
            let mut sample = odometer(charge_core_at(180), 101.0);
            sample.odometer_miles_ts = Some(sample.ts - Duration::milliseconds(age_ms));
            let (_, diag) = fusion.fuse_from(&sample, false);
            assert_eq!(diag.odometer_outcome, outcome);
        }
        let mut fusion = TripSignalFusion::default();
        fusion.fuse(&odometer(charge_core_at(60), 100.0));
        for seconds in [60, 59] {
            let (_, diag) = fusion.fuse_from(&odometer(charge_core_at(seconds), 101.0), false);
            assert_eq!(diag.odometer_outcome, "future_or_out_of_order");
        }
        assert_eq!(
            fusion.fuse(&odometer(charge_core_at(120), 101.0)).speed_mph,
            Some(60.0)
        );
    }

    #[test]
    fn charge_core_interleaved_vehicle_streams_and_cold_restart_are_independent() {
        let mut first = TripSignalFusion::new(true);
        let mut second = TripSignalFusion::new(false);
        let mut sample = charge_core_fix(0, 1.0);
        sample.power_state = Some(PowerState::Drive);
        sample.odometer_miles = Some(100.0);
        first.fuse(&sample);
        let mut other = charge_core_fix(0, 50.0);
        other.vehicle_id = Uuid::from_u128(2);
        other.power_state = Some(PowerState::Sleep);
        other.odometer_miles = Some(2_000.0);
        second.fuse(&other);
        sample.ts = charge_core_at(60);
        sample.power_state = None;
        sample.odometer_miles = Some(101.0);
        let joined = first.fuse(&sample);
        assert_eq!(joined.power_state, Some(PowerState::Drive));
        assert_eq!(joined.speed_mph, Some(60.0));
        other.ts = charge_core_at(60);
        other.power_state = None;
        let joined = second.fuse(&other);
        assert_eq!(joined.vehicle_id, Uuid::from_u128(2));
        assert_eq!(joined.power_state, Some(PowerState::Sleep));
        assert_eq!(joined.speed_mph, Some(0.0));
        let (restarted, diag) = TripSignalFusion::new(true).fuse_from(&sample, true);
        assert_eq!(restarted.power_state, None);
        assert_eq!(restarted.speed_mph, None);
        assert_eq!(diag.gnss_outcome, "baseline");
        assert_eq!(diag.odometer_outcome, "baseline");
    }

    #[test]
    fn charge_core_full_sequence_and_downstream_trip_contract() {
        use crate::ingestion::trip_detector::{
            compute_distance_odometer_or_gps, TripDetectorState, TripEvent,
        };
        let mut fusion = TripSignalFusion::new(true);
        let mut actual_detector = TripDetectorState::new(Uuid::from_u128(1));
        let mut expected_detector = actual_detector.clone();
        let mut actual_id = None;
        let mut expected_id = None;
        for i in 0..4 {
            let mut raw = charge_core_fix(i * 60, 1.0);
            raw.odometer_miles = Some(100.0 + i.min(2) as f64);
            raw.battery_level = Some(80.0 - i.min(2) as f64);
            raw.battery_capacity_wh = Some(100_000.0);
            raw.regen_power_kw = Some(-3.0);
            if i == 0 {
                raw.power_state = Some(PowerState::Drive);
            }
            if i == 3 {
                raw.power_state = Some(PowerState::Sleep);
            }
            let original = serde_json::to_value(&raw).unwrap();
            let mut expected = raw.clone();
            if i == 1 || i == 2 {
                expected.power_state = Some(PowerState::Drive);
                expected.power_state_ts = Some(charge_core_at(0));
            }
            if i > 0 {
                expected.speed_mph = Some(if i == 3 { 0.0 } else { 60.0 });
                expected.speed_mph_ts = Some(raw.ts);
            }
            let joined = fusion.fuse(&raw);
            assert_eq!(
                serde_json::to_value(&joined).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            assert_eq!(serde_json::to_value(&raw).unwrap(), original);
            match (
                actual_detector.process(&joined),
                expected_detector.process(&expected),
            ) {
                (TripEvent::NoChange, TripEvent::NoChange) => assert!(i == 0 || i == 2),
                (
                    TripEvent::TripStarted {
                        trip_id: actual,
                        started_at,
                    },
                    TripEvent::TripStarted {
                        trip_id: expected,
                        started_at: expected_at,
                    },
                ) => {
                    assert_eq!(i, 1);
                    assert_eq!(started_at, charge_core_at(0));
                    assert_eq!(started_at, expected_at);
                    actual_id = Some(actual);
                    expected_id = Some(expected);
                }
                (TripEvent::TripEnded { trip }, TripEvent::TripEnded { trip: mut expected }) => {
                    assert_eq!(i, 3);
                    assert_eq!(Some(trip.trip_id), actual_id);
                    assert_eq!(Some(expected.trip_id), expected_id);
                    expected.trip_id = trip.trip_id;
                    assert_eq!(trip, expected);
                    assert_eq!(trip.ended_at, charge_core_at(180));
                    assert_eq!(trip.regen_wh, Some(100.0));
                    assert_eq!(
                        (trip.soc_start, trip.soc_end, trip.battery_capacity_wh),
                        (Some(79.0), Some(78.0), Some(100_000.0))
                    );
                    assert_eq!(
                        compute_distance_odometer_or_gps(
                            trip.start_odometer_mi,
                            trip.end_odometer_mi,
                            &trip.points
                        ),
                        2.0
                    );
                }
                (actual, expected) => panic!("transition mismatch: {actual:?}, {expected:?}"),
            }
        }
        assert_eq!(actual_detector.active_trip_id(), None);
    }

    #[test]
    fn source_policy_follows_latest_accepted_power_not_vehicle_model() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::default();
        let mut power = TelemetryEvent::empty(Uuid::nil(), at);
        power.power_state = Some(PowerState::Go);
        fusion.fuse_from(&power, true);
        let later = TelemetryEvent::empty(Uuid::nil(), at + Duration::minutes(20));
        let (joined, diag) = fusion.fuse_from(&later, false);
        assert_eq!(joined.power_state, Some(PowerState::Go));
        assert_eq!(diag.power_policy, "change_only");
        assert_eq!(diag.power_origin, "carried_forward");
        power.ts = later.ts;
        power.power_state = Some(PowerState::Ready);
        fusion.fuse_from(&power, false);
        let after = TelemetryEvent::empty(Uuid::nil(), later.ts + Duration::minutes(3));
        let (joined, diag) = fusion.fuse_from(&after, true);
        assert_eq!(joined.power_state, None);
        assert_eq!(diag.power_policy, "periodic");
        assert_eq!(diag.power_outcome, "stale_or_future");
    }

    #[test]
    fn diagnostics_distinguish_provisional_zero_and_odometer_override() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::default();
        fusion.fuse(&fix_with_odometer(at, -97.0, 55.92));
        let sample = fix_with_odometer(at + Duration::seconds(60), -96.99, 56.54);
        let (joined, diag) = fusion.fuse_from(&sample, false);
        assert_eq!(diag.gnss_outcome, "provisional_zero");
        assert_eq!(diag.odometer_outcome, "derived");
        assert_eq!(diag.speed_origin, "odometer");
        assert_eq!(diag.odometer_interval_seconds, Some(60));
        assert!((diag.odometer_delta_miles.unwrap() - 0.62).abs() < 1e-9);
        assert!(joined.speed_mph.unwrap() > 2.0);
        assert_eq!(sample.speed_mph, None, "source reading stays unchanged");
        let repeated = fusion.fuse_from(&sample, false).1;
        assert_eq!(repeated.gnss_outcome, "stale_future_or_out_of_order");
        assert_eq!(repeated.odometer_outcome, "future_or_out_of_order");
    }

    #[test]
    fn invalid_or_old_power_cannot_replace_newer_cached_state() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::default();
        let mut sample = TelemetryEvent::empty(Uuid::nil(), at);
        sample.power_state = Some(PowerState::Sleep);
        fusion.fuse_from(&sample, true);
        sample.ts += Duration::seconds(60);
        sample.power_state = Some(PowerState::Go);
        sample.power_state_ts = Some(at - Duration::seconds(1));
        assert_eq!(
            fusion.fuse_from(&sample, false).0.power_state,
            Some(PowerState::Sleep)
        );
        sample.power_state_ts = Some(sample.ts + Duration::seconds(60));
        assert_eq!(
            fusion.fuse_from(&sample, false).0.power_state,
            Some(PowerState::Sleep)
        );
    }

    #[test]
    fn periodic_source_missing_speed_uses_same_drive_sequence() {
        use crate::ingestion::trip_detector::{TripDetectorState, TripEvent};
        let at = Utc::now();
        let mut fusion = TripSignalFusion::default();
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut first = odometer(at, 100.0);
        first.power_state = Some(PowerState::Drive);
        detector.process(&fusion.fuse_from(&first, false).0);
        let mut moved = odometer(at + Duration::seconds(60), 100.62);
        moved.power_state = Some(PowerState::Drive);
        assert!(matches!(
            detector.process(&fusion.fuse_from(&moved, false).0),
            TripEvent::TripStarted { .. }
        ));
        let mut sleep = odometer(at + Duration::seconds(120), 100.62);
        sleep.power_state = Some(PowerState::Sleep);
        let TripEvent::TripEnded { trip } = detector.process(&fusion.fuse_from(&sleep, false).0)
        else {
            panic!("expected completed trip");
        };
        assert_eq!(trip.start_odometer_mi, Some(100.0));
        assert_eq!(trip.end_odometer_mi, Some(100.62));
    }

    #[test]
    fn reported_speed_interrupts_gnss_confirmation() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::default();
        let mut fix = TelemetryEvent::empty(Uuid::nil(), at);
        fix.latitude = Some(30.0);
        fix.longitude = Some(-97.0);
        fusion.fuse(&fix);
        for offset in [30, 60] {
            fix.ts = at + Duration::seconds(offset);
            fix.longitude = Some(-97.0 + offset as f64 / 30.0 * 0.001);
            fusion.fuse(&fix);
        }
        let mut stopped = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(70));
        stopped.speed_mph = Some(0.0);
        assert_eq!(fusion.fuse(&stopped).speed_mph, Some(0.0));
        fix.ts = at + Duration::seconds(90);
        fix.longitude = Some(-96.997);
        let (joined, diag) = fusion.fuse_from(&fix, false);
        assert_eq!(joined.speed_mph, Some(0.0));
        assert_eq!(diag.gnss_outcome, "provisional_zero");
    }

    #[test]
    fn r2_sparse_power_and_three_fixes_produce_motion() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut power = TelemetryEvent::empty(Uuid::nil(), at);
        power.power_state = Some(PowerState::Go);
        fusion.fuse(&power);
        for (offset, lon) in [(10, -97.0), (40, -96.999), (70, -96.998)] {
            let mut fix = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(offset));
            fix.latitude = Some(30.0);
            fix.longitude = Some(lon);
            fix.location_ts = Some(fix.ts);
            let joined = fusion.fuse(&fix);
            assert_eq!(joined.power_state, Some(PowerState::Go));
            if offset == 70 {
                assert!(joined.speed_mph.unwrap_or(0.0) > 2.0);
            }
        }
    }

    #[test]
    fn stale_power_is_not_joined_for_dense_vehicles() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(false);
        let mut power = TelemetryEvent::empty(Uuid::nil(), at);
        power.power_state = Some(PowerState::Go);
        fusion.fuse(&power);
        let fix = TelemetryEvent::empty(Uuid::nil(), at + Duration::minutes(4));
        assert_eq!(fusion.fuse(&fix).power_state, None);
    }

    #[test]
    fn sparse_power_holds_until_the_vehicle_reports_another_state() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut power = TelemetryEvent::empty(Uuid::nil(), at);
        power.power_state = Some(PowerState::Go);
        fusion.fuse(&power);
        let later = TelemetryEvent::empty(Uuid::nil(), at + Duration::minutes(20));
        assert_eq!(fusion.fuse(&later).power_state, Some(PowerState::Go));

        power.ts = at + Duration::minutes(21);
        power.power_state = Some(PowerState::Ready);
        fusion.fuse(&power);
        let after = TelemetryEvent::empty(Uuid::nil(), at + Duration::minutes(22));
        assert_eq!(fusion.fuse(&after).power_state, Some(PowerState::Ready));
    }

    #[test]
    fn location_jump_cannot_derive_speed() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut fix = TelemetryEvent::empty(Uuid::nil(), at);
        fix.latitude = Some(30.0);
        fix.longitude = Some(-97.0);
        fix.location_ts = Some(fix.ts);
        fusion.fuse(&fix);
        fix.ts += Duration::seconds(30);
        fix.latitude = Some(35.0);
        fix.location_ts = Some(fix.ts);
        assert_eq!(fusion.fuse(&fix).speed_mph, None);
    }

    fn odometer(at: chrono::DateTime<Utc>, miles: f64) -> TelemetryEvent {
        let mut event = TelemetryEvent::empty(Uuid::nil(), at);
        event.odometer_miles = Some(miles);
        event
    }

    #[test]
    fn sparse_odometer_increment_derives_speed() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        assert_eq!(fusion.fuse(&odometer(at, 55.92)).speed_mph, None);
        let speed = fusion
            .fuse(&odometer(at + Duration::seconds(60), 56.54))
            .speed_mph
            .expect("odometer speed");
        assert!((speed - 37.2).abs() < 0.1, "{speed}");
        // A parked reading hours later is not motion.
        let parked = fusion.fuse(&odometer(at + Duration::hours(3), 56.54));
        assert_eq!(parked.speed_mph, None);
        let resumed = fusion.fuse(&odometer(at + Duration::hours(4), 57.16));
        assert_eq!(resumed.speed_mph, None);
    }

    fn fix_with_odometer(at: chrono::DateTime<Utc>, lon: f64, miles: f64) -> TelemetryEvent {
        let mut event = odometer(at, miles);
        event.latitude = Some(30.0);
        event.longitude = Some(lon);
        event.location_ts = Some(at);
        event
    }

    /// Repeated GNSS coordinates yield a derived zero, but the odometer on the
    /// same sample shows the vehicle moved; the odometer speed must win.
    #[test]
    fn odometer_increment_overrides_stationary_gps_zero_on_mixed_sample() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        fusion.fuse(&fix_with_odometer(at, -97.0, 55.92));
        let joined = fusion.fuse(&fix_with_odometer(at + Duration::seconds(60), -97.0, 56.54));
        let speed = joined.speed_mph.expect("odometer speed");
        assert!((speed - 37.2).abs() < 0.1, "{speed}");
        assert_eq!(joined.speed_mph_ts, Some(at + Duration::seconds(60)));
    }

    /// The first moving GNSS segment is held at a provisional zero; an odometer
    /// increment on the same sample still contributes.
    #[test]
    fn odometer_increment_overrides_provisional_gps_zero_on_mixed_sample() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        fusion.fuse(&fix_with_odometer(at, -97.0, 55.92));
        let joined = fusion.fuse(&fix_with_odometer(
            at + Duration::seconds(60),
            -96.99,
            56.54,
        ));
        assert!(joined.speed_mph.expect("odometer speed") > 2.0);
    }

    #[test]
    fn stationary_gps_zero_stands_without_odometer_increment() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        fusion.fuse(&fix_with_odometer(at, -97.0, 55.92));
        let joined = fusion.fuse(&fix_with_odometer(at + Duration::seconds(60), -97.0, 55.92));
        assert_eq!(joined.speed_mph, Some(0.0));
    }

    #[test]
    fn reported_zero_speed_is_not_replaced_by_odometer() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        fusion.fuse(&odometer(at, 55.92));
        let mut sample = odometer(at + Duration::seconds(60), 56.54);
        sample.speed_mph = Some(0.0);
        assert_eq!(fusion.fuse(&sample).speed_mph, Some(0.0));
    }

    #[test]
    fn stale_odometer_on_mixed_sample_keeps_gps_zero() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        fusion.fuse(&fix_with_odometer(at, -97.0, 55.92));
        let mut sample = fix_with_odometer(at + Duration::minutes(4), -97.0, 56.54);
        sample.odometer_miles_ts = Some(at + Duration::seconds(60));
        assert_eq!(fusion.fuse(&sample).speed_mph, Some(0.0));
    }

    #[test]
    fn implausible_odometer_jump_on_mixed_sample_keeps_gps_zero() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        fusion.fuse(&fix_with_odometer(at, -97.0, 55.92));
        let joined = fusion.fuse(&fix_with_odometer(at + Duration::seconds(60), -97.0, 60.0));
        assert_eq!(joined.speed_mph, Some(0.0));
    }

    /// Minute-spaced fixes that repeat the same coordinates while the odometer
    /// climbs must still produce a trip.
    #[test]
    fn r2_drive_with_repeated_gps_and_rising_odometer_produces_a_trip() {
        use crate::ingestion::trip_detector::{TripDetectorState, TripEvent};

        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut go = fix_with_odometer(at, -97.0, 55.92);
        go.power_state = Some(PowerState::Go);
        detector.process(&fusion.fuse(&go));
        let mut started = false;
        for (i, secs) in [60, 120, 180, 240].into_iter().enumerate() {
            let sample = fix_with_odometer(
                at + Duration::seconds(secs),
                -97.0,
                55.92 + 0.62 * (i + 1) as f64,
            );
            if let TripEvent::TripStarted { .. } = detector.process(&fusion.fuse(&sample)) {
                started = true;
            }
        }
        assert!(started, "trip should start from odometer motion");
    }

    #[test]
    fn periodic_sources_also_derive_missing_speed() {
        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(false);
        fusion.fuse(&odometer(at, 55.92));
        let next = fusion.fuse(&odometer(at + Duration::seconds(60), 56.54));
        assert!(next.speed_mph.unwrap() > 2.0);
    }

    /// Timings from a real R2 drive that was missed: power reported Go once,
    /// then only minute-spaced GNSS fixes and 1 km odometer steps arrived.
    #[test]
    fn r2_drive_with_late_motion_produces_a_trip() {
        use crate::ingestion::trip_detector::{
            compute_distance_odometer_or_gps, TripDetectorState, TripEvent,
        };

        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut events = vec![odometer(at - Duration::minutes(34), 55.92)];
        // Parked at home before shifting.
        let mut home = TelemetryEvent::empty(Uuid::nil(), at - Duration::seconds(30));
        home.latitude = Some(29.9);
        home.longitude = Some(-97.0);
        home.location_ts = Some(home.ts);
        events.push(home);
        let mut go = TelemetryEvent::empty(Uuid::nil(), at);
        go.power_state = Some(PowerState::Go);
        events.push(go);
        for (secs, miles) in [
            (73, 56.54),
            (216, 57.17),
            (355, 57.79),
            (410, 58.41),
            (471, 59.03),
            (515, 59.65),
            (557, 60.27),
            (613, 60.89),
            (681, 61.52),
        ] {
            events.push(odometer(at + Duration::seconds(secs), miles));
        }
        for (i, secs) in [60, 120, 241, 360, 421, 482, 542, 601, 661, 722]
            .into_iter()
            .enumerate()
        {
            let mut fix = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(secs));
            fix.latitude = Some(30.0 + i as f64 * 0.01);
            fix.longitude = Some(-97.0);
            fix.location_ts = Some(fix.ts);
            events.push(fix);
        }
        let mut ready = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(772));
        ready.power_state = Some(PowerState::Ready);
        events.push(ready);
        let mut sleep = TelemetryEvent::empty(Uuid::nil(), at + Duration::seconds(1200));
        sleep.power_state = Some(PowerState::Sleep);
        events.push(sleep);
        events.sort_by_key(|event| event.ts);

        let mut started = None;
        let mut ended = None;
        for event in &events {
            match detector.process(&fusion.fuse(event)) {
                TripEvent::TripStarted { started_at, .. } => started = Some(started_at),
                TripEvent::TripEnded { trip } => ended = Some(trip),
                TripEvent::NoChange => {}
            }
        }

        // Motion is first seen minutes later, but the trip starts at the
        // shift to Go and at the parked location.
        assert_eq!(started.expect("trip started"), at);
        let trip = ended.expect("trip ended");
        assert_eq!(trip.started_at, at);
        let first = trip.points.first().expect("start point");
        assert_eq!((first.lat, first.lng, first.speed_mph), (29.9, -97.0, 0.0));
        assert_eq!(trip.start_odometer_mi, Some(55.92));
        assert_eq!(trip.end_odometer_mi, Some(61.52));
        let distance = compute_distance_odometer_or_gps(
            trip.start_odometer_mi,
            trip.end_odometer_mi,
            &trip.points,
        );
        assert!((distance - 5.6).abs() < 0.01, "{distance}");
    }

    /// A shift long before the motion belongs to a missed drive, not this
    /// trip: keep the prior-odometer rule and the first-motion start.
    #[test]
    fn stale_gear_shift_does_not_anchor_a_later_trip() {
        use crate::ingestion::trip_detector::{TripDetectorState, TripEvent};

        let at = Utc::now();
        let mut fusion = TripSignalFusion::new(true);
        let mut detector = TripDetectorState::new(Uuid::nil());
        let mut events = vec![odometer(at - Duration::minutes(40), 50.0)];
        let mut go = TelemetryEvent::empty(Uuid::nil(), at - Duration::minutes(30));
        go.power_state = Some(PowerState::Go);
        events.push(go);
        events.push(odometer(at - Duration::seconds(60), 55.92));
        events.push(odometer(at, 56.54));
        events.sort_by_key(|event| event.ts);

        let mut started = None;
        for event in &events {
            if let TripEvent::TripStarted { started_at, .. } = detector.process(&fusion.fuse(event))
            {
                started = Some(started_at);
            }
        }
        let started = started.expect("trip started");
        assert!(started > at - Duration::minutes(15), "{started}");
    }
}

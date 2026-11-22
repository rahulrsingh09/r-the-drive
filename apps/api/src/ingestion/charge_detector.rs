//! Detects charge session boundaries from charger_state transitions.
//! Extended with energy integration, charger-type classification, and max/avg
//! charge rate tracking.

use crate::models::telemetry::{ChargerState, TelemetryEvent};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::location::valid_location_pair;
use crate::services::charging::{classify_charger_type, DC_MIN_RATE_KW, PLAUSIBLE_MAX_CHARGE_KW};

const CHARGE_TIMEOUT_SECS: i64 = 1800; // 30 min

/// A plugged-in vehicle cannot move. A fresh odometer reading, taken after
/// charging was last seen and at least `MOVED_MIN_MILES` beyond the odometer
/// from then, ends the session at that last charging reading. The R2 reports
/// the odometer in 1 km steps, so one step (0.62 mi) is not enough.
const MOVED_MIN_MILES: f64 = 1.0;
const MOTION_READING_MAX_AGE_SECS: i64 = 120;

/// A state of charge this far above the last charging reading is charging
/// evidence, even when the frame carries no charger signal.
const SOC_RISE_MIN_PCT: f64 = 0.5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedChargeSession {
    pub session_id: Uuid,
    pub vehicle_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    #[serde(with = "optional_float_bits")]
    pub location_lat: Option<f64>,
    #[serde(with = "optional_float_bits")]
    pub location_lng: Option<f64>,
    #[serde(with = "optional_float_bits")]
    pub soc_start: Option<f64>,
    #[serde(with = "optional_float_bits")]
    pub soc_end: Option<f64>,
    #[serde(with = "optional_float_bits")]
    pub charge_limit: Option<f64>,
    #[serde(with = "optional_float_bits")]
    pub battery_capacity_wh: Option<f64>,

    /// Energy drawn from the grid (integrated from |power_kw| × Δt).
    #[serde(with = "optional_float_bits")]
    pub energy_used_wh: Option<f64>,
    /// Energy delivered to the pack: (soc_end - soc_start) × battery_capacity.
    #[serde(with = "optional_float_bits")]
    pub energy_added_wh: Option<f64>,
    /// Peak AC/DC power recorded during the session.
    #[serde(with = "optional_float_bits")]
    pub peak_charge_kw: Option<f64>,
    /// Average charge rate = energy_used_kwh / duration_hours.
    #[serde(with = "optional_float_bits")]
    pub avg_charge_rate_kw: Option<f64>,
    /// Classified charger type: "ac", "ac_l2", or "dc".
    pub charger_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActiveChargeSessionSnapshot {
    pub session_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub soc_start: Option<f64>,
    pub last_soc: Option<f64>,
    pub charge_limit: Option<f64>,
    pub battery_capacity_wh: Option<f64>,
    pub last_charge_ts: DateTime<Utc>,
    pub last_power_ts: Option<DateTime<Utc>>,
    pub energy_used_wh: f64,
    pub peak_charge_kw: f64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeDetectorState {
    vehicle_id: Uuid,
    battery: ChargeBatteryReadings,
    phase: ChargePhase,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChargeBatteryReadings {
    #[serde(with = "optional_float_bits")]
    soc: Option<f64>,
    #[serde(with = "optional_float_bits")]
    limit: Option<f64>,
    #[serde(with = "optional_float_bits")]
    capacity_wh: Option<f64>,
}

impl ChargeBatteryReadings {
    fn observe(&mut self, event: &TelemetryEvent) {
        if event.battery_level.is_some() {
            self.soc = event.battery_level;
        }
        if event.battery_limit.is_some() {
            self.limit = event.battery_limit;
        }
        if event.battery_capacity_wh.is_some() {
            self.capacity_wh = event.battery_capacity_wh;
        }
    }

    fn added_wh(&self, start_soc: Option<f64>) -> Option<f64> {
        let delta = self.soc? - start_soc?;
        let capacity = self.capacity_wh?;
        (delta > 0.0).then_some(delta / 100.0 * capacity)
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
enum ChargePhase {
    #[default]
    Idle,
    Active(ActiveCharge),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveCharge {
    id: Uuid,
    started_at: DateTime<Utc>,
    #[serde(with = "location_bits")]
    location: (Option<f64>, Option<f64>),
    #[serde(with = "optional_float_bits")]
    start_soc: Option<f64>,
    last_activity: DateTime<Utc>,
    #[serde(default, with = "optional_float_bits")]
    last_charge_soc: Option<f64>,
    #[serde(default, with = "optional_float_bits")]
    last_charge_odometer: Option<f64>,
    #[serde(default, with = "float_bits")]
    last_charge_peak_kw: f64,
    #[serde(default, with = "float_bits")]
    last_charge_energy_wh: f64,
    grid: ChargeGridEnergy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChargeGridEnergy {
    last_sample_at: Option<DateTime<Utc>>,
    #[serde(with = "float_bits")]
    watt_hours: f64,
    #[serde(with = "float_bits")]
    peak_kw: f64,
    #[serde(default, with = "optional_float_bits")]
    pending_dc_reading_kw: Option<f64>,
}

impl ChargeGridEnergy {
    fn observe(&mut self, at: DateTime<Utc>, power: Option<f64>) {
        let Some(kw) = power.map(f64::abs) else {
            return;
        };
        if !(kw > 0.0 && kw <= PLAUSIBLE_MAX_CHARGE_KW) {
            return;
        }
        let ac_baseline = self.peak_kw > 0.0 && self.peak_kw < DC_MIN_RATE_KW;
        let is_dc_rate = kw >= DC_MIN_RATE_KW;
        if ac_baseline && is_dc_rate && self.pending_dc_reading_kw.is_none() {
            self.pending_dc_reading_kw = Some(kw);
            return;
        }
        if let Some(pending) = self.pending_dc_reading_kw.take() {
            if is_dc_rate {
                self.peak_kw = self.peak_kw.max(pending);
            }
        }
        if let Some(previous) = self.last_sample_at {
            // A late reading must not rewind the next integration interval.
            if at < previous {
                return;
            }
            let hours = (at - previous).num_milliseconds() as f64 / 3_600_000.0;
            self.watt_hours += kw * 1_000.0 * hours;
        }
        self.peak_kw = self.peak_kw.max(kw);
        self.last_sample_at = Some(at);
    }
}

struct ChargeSignals {
    starts: bool,
    sustains: bool,
    stops: bool,
}

impl ChargeSignals {
    fn read(event: &TelemetryEvent) -> Self {
        let starts = event.is_actively_charging();
        let remaining = event
            .time_to_end_of_charge_min
            .is_some_and(|value| value > 0);
        let stops = matches!(
            event.charger_state,
            Some(ChargerState::Done | ChargerState::Disconnected)
        ) || event.charger_status.as_deref() == Some("chrgr_sts_not_connected")
            || (event.charger_status.as_deref() == Some("chrgr_sts_connected_no_chrg")
                && !remaining);
        Self {
            starts,
            sustains: starts || remaining,
            stops,
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ChargeEvent {
    SessionStarted,
    SessionEnded(CompletedChargeSession),
    NoChange,
}

impl ChargeDetectorState {
    pub fn new(vehicle_id: Uuid) -> Self {
        Self {
            vehicle_id,
            ..Self::default()
        }
    }

    pub fn from_snapshot(vehicle_id: Uuid, snapshot: ActiveChargeSessionSnapshot) -> Self {
        Self {
            vehicle_id,
            battery: ChargeBatteryReadings {
                soc: snapshot.last_soc,
                limit: snapshot.charge_limit,
                capacity_wh: snapshot.battery_capacity_wh,
            },
            phase: ChargePhase::Active(ActiveCharge {
                id: snapshot.session_id,
                started_at: snapshot.started_at,
                location: (snapshot.location_lat, snapshot.location_lng),
                start_soc: snapshot.soc_start,
                last_activity: snapshot.last_charge_ts,
                last_charge_soc: snapshot.last_soc,
                last_charge_odometer: None,
                last_charge_peak_kw: snapshot.peak_charge_kw.max(0.0),
                last_charge_energy_wh: snapshot.energy_used_wh.max(0.0),
                grid: ChargeGridEnergy {
                    last_sample_at: snapshot.last_power_ts,
                    watt_hours: snapshot.energy_used_wh.max(0.0),
                    peak_kw: snapshot.peak_charge_kw.max(0.0),
                    pending_dc_reading_kw: None,
                },
            }),
        }
    }

    /// Returns the identity stamped on telemetry while a session is active.
    pub fn active_session_id(&self) -> Option<Uuid> {
        match &self.phase {
            ChargePhase::Idle => None,
            ChargePhase::Active(session) => Some(session.id),
        }
    }

    pub(crate) fn vehicle_id(&self) -> Uuid {
        self.vehicle_id
    }

    pub(crate) fn active_snapshot(&self) -> Option<ActiveChargeSessionSnapshot> {
        let ChargePhase::Active(active) = &self.phase else {
            return None;
        };
        Some(ActiveChargeSessionSnapshot {
            session_id: active.id,
            started_at: active.started_at,
            location_lat: active.location.0,
            location_lng: active.location.1,
            soc_start: active.start_soc,
            last_soc: self.battery.soc,
            charge_limit: self.battery.limit,
            battery_capacity_wh: self.battery.capacity_wh,
            last_charge_ts: active.last_activity,
            last_power_ts: active.grid.last_sample_at,
            energy_used_wh: active.grid.watt_hours,
            peak_charge_kw: active.grid.peak_kw,
        })
    }

    pub fn process(&mut self, event: &TelemetryEvent) -> ChargeEvent {
        self.battery.observe(event);
        let (phase, transition) =
            std::mem::take(&mut self.phase).advance(event, self.vehicle_id, &self.battery);
        self.phase = phase;
        transition
    }
}

impl ChargePhase {
    fn advance(
        self,
        event: &TelemetryEvent,
        vehicle_id: Uuid,
        battery: &ChargeBatteryReadings,
    ) -> (Self, ChargeEvent) {
        let signals = ChargeSignals::read(event);
        match self {
            Self::Idle if signals.starts => (
                Self::Active(ActiveCharge::begin(event)),
                ChargeEvent::SessionStarted,
            ),
            Self::Idle => (Self::Idle, ChargeEvent::NoChange),
            Self::Active(mut session) => {
                session.observe(event);
                let soc_rising = matches!(
                    (event.battery_level, session.last_charge_soc),
                    (Some(now), Some(then)) if now - then >= SOC_RISE_MIN_PCT
                );
                let charging_now = signals.sustains || soc_rising;
                if !charging_now && session.moved_since_last_charge(event) {
                    return (
                        Self::Idle,
                        ChargeEvent::SessionEnded(session.finish_at_last_charge(
                            vehicle_id,
                            event.ts,
                            battery,
                        )),
                    );
                }
                if charging_now {
                    session.last_activity = event.ts;
                    session.last_charge_soc = battery.soc;
                    session.last_charge_peak_kw = session.grid.peak_kw;
                    session.last_charge_energy_wh = session.grid.watt_hours;
                    if event.odometer_miles.is_some() {
                        session.last_charge_odometer = event.odometer_miles;
                    }
                } else if session.last_charge_odometer.is_none() {
                    session.last_charge_odometer = event.odometer_miles;
                }
                let expired =
                    (event.ts - session.last_activity).num_seconds() > CHARGE_TIMEOUT_SECS;
                if signals.stops {
                    (
                        Self::Idle,
                        ChargeEvent::SessionEnded(session.finish(vehicle_id, event.ts, battery)),
                    )
                } else if expired {
                    (
                        Self::Idle,
                        ChargeEvent::SessionEnded(session.finish_at_last_charge(
                            vehicle_id,
                            event.ts,
                            battery,
                        )),
                    )
                } else {
                    (Self::Active(session), ChargeEvent::NoChange)
                }
            }
        }
    }
}

impl ActiveCharge {
    fn begin(event: &TelemetryEvent) -> Self {
        let location = valid_location_pair(event.latitude, event.longitude)
            .map(|(lat, lng)| (Some(lat), Some(lng)))
            .unwrap_or_default();
        Self {
            id: Uuid::new_v4(),
            started_at: event.ts,
            location,
            start_soc: event.battery_level,
            last_activity: event.ts,
            last_charge_soc: event.battery_level,
            last_charge_odometer: event.odometer_miles,
            last_charge_peak_kw: 0.0,
            last_charge_energy_wh: 0.0,
            grid: ChargeGridEnergy {
                // The opening frame anchors time; subsequent frames supply power.
                last_sample_at: Some(event.ts),
                watt_hours: 0.0,
                peak_kw: 0.0,
                pending_dc_reading_kw: None,
            },
        }
    }

    fn observe(&mut self, event: &TelemetryEvent) {
        if self.start_soc.is_none() {
            self.start_soc = event.battery_level;
        }
        if self.location == (None, None) {
            if let Some((lat, lng)) = valid_location_pair(event.latitude, event.longitude) {
                self.location = (Some(lat), Some(lng));
            }
        }
        self.grid.observe(event.ts, event.power_kw);
    }

    /// A fresh odometer reading, taken after charging was last seen, shows the
    /// vehicle moved. Older (replayed or delayed) readings and GNSS speed,
    /// which jitters while parked, are not used.
    fn moved_since_last_charge(&self, event: &TelemetryEvent) -> bool {
        let (Some(odometer), Some(baseline)) = (event.odometer_miles, self.last_charge_odometer)
        else {
            return false;
        };
        let read_at = event.odometer_miles_ts.unwrap_or(event.ts);
        read_at > self.last_activity
            && (event.ts - read_at).num_seconds() <= MOTION_READING_MAX_AGE_SECS
            && odometer - baseline >= MOVED_MIN_MILES
    }

    /// Close a session that ended without an explicit charger signal (by
    /// timeout or motion) when charging was last seen, with the state of
    /// charge, peak and energy from then.
    fn finish_at_last_charge(
        mut self,
        vehicle_id: Uuid,
        noticed_at: DateTime<Utc>,
        battery: &ChargeBatteryReadings,
    ) -> CompletedChargeSession {
        if self.last_activity <= self.started_at {
            return self.finish(vehicle_id, noticed_at, battery);
        }
        let ended_at = self.last_activity;
        let mut trimmed_battery = battery.clone();
        if self.last_charge_soc.is_some() {
            trimmed_battery.soc = self.last_charge_soc;
        }
        self.grid.peak_kw = self.last_charge_peak_kw;
        self.grid.watt_hours = self.last_charge_energy_wh;
        self.finish(vehicle_id, ended_at, &trimmed_battery)
    }

    fn finish(
        self,
        vehicle_id: Uuid,
        ended_at: DateTime<Utc>,
        battery: &ChargeBatteryReadings,
    ) -> CompletedChargeSession {
        let energy_used_wh = (self.grid.watt_hours > 0.0).then_some(self.grid.watt_hours);
        let energy_added_wh = battery.added_wh(self.start_soc);
        let hours = (ended_at - self.started_at).num_seconds() as f64 / 3_600.0;
        let avg_charge_rate_kw = energy_used_wh
            .or(energy_added_wh)
            .filter(|_| hours > 0.0)
            .map(|wh| wh / 1_000.0 / hours);
        let peak_charge_kw = (self.grid.peak_kw > 0.0).then_some(self.grid.peak_kw);
        CompletedChargeSession {
            session_id: self.id,
            vehicle_id,
            started_at: self.started_at,
            ended_at,
            location_lat: self.location.0,
            location_lng: self.location.1,
            soc_start: self.start_soc,
            soc_end: battery.soc,
            charge_limit: battery.limit,
            battery_capacity_wh: battery.capacity_wh,
            energy_used_wh,
            energy_added_wh,
            peak_charge_kw,
            avg_charge_rate_kw,
            charger_type: peak_charge_kw
                .or(avg_charge_rate_kw)
                .map(|kw| classify_charger_type(kw).to_owned()),
        }
    }
}

// Integer bit patterns keep checkpoint floats exact through JSONB round trips.
mod optional_float_bits {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub(super) fn serialize<S: Serializer>(
        value: &Option<f64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(f64::to_bits).serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<f64>, D::Error> {
        Ok(Option::<u64>::deserialize(deserializer)?.map(f64::from_bits))
    }
}

mod float_bits {
    use serde::{Deserialize, Deserializer, Serializer};
    pub(super) fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.to_bits())
    }
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        Ok(f64::from_bits(u64::deserialize(deserializer)?))
    }
}

mod location_bits {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub(super) fn serialize<S: Serializer>(
        value: &(Option<f64>, Option<f64>),
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        (value.0.map(f64::to_bits), value.1.map(f64::to_bits)).serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<(Option<f64>, Option<f64>), D::Error> {
        let (lat, lng) = <(Option<u64>, Option<u64>)>::deserialize(deserializer)?;
        Ok((lat.map(f64::from_bits), lng.map(f64::from_bits)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn charge_core_at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + seconds, 0).unwrap()
    }

    fn charge_core_sample(seconds: i64) -> TelemetryEvent {
        TelemetryEvent::empty(Uuid::from_u128(1), charge_core_at(seconds))
    }

    fn charge_core_started() -> ChargeDetectorState {
        let mut detector = ChargeDetectorState::new(Uuid::from_u128(1));
        let mut start = charge_core_sample(0);
        start.charger_state = Some(ChargerState::Charging);
        assert!(matches!(
            detector.process(&start),
            ChargeEvent::SessionStarted
        ));
        detector
    }

    fn charge_core_finish(
        detector: &mut ChargeDetectorState,
        seconds: i64,
    ) -> CompletedChargeSession {
        let mut end = charge_core_sample(seconds);
        end.charger_state = Some(ChargerState::Done);
        match detector.process(&end) {
            ChargeEvent::SessionEnded(session) => session,
            other => panic!("expected completed session, got {other:?}"),
        }
    }

    #[test]
    fn charge_core_independent_start_signals_and_conflicting_end() {
        use crate::models::telemetry::PowerState;
        for signal in 0..3 {
            let mut start = charge_core_sample(0);
            match signal {
                0 => start.power_state = Some(PowerState::Charging),
                1 => start.charger_state = Some(ChargerState::Charging),
                _ => start.charger_status = Some("chrgr_sts_connected_charging".into()),
            }
            let mut detector = ChargeDetectorState::new(start.vehicle_id);
            assert!(matches!(
                detector.process(&start),
                ChargeEvent::SessionStarted
            ));
            assert!(detector.active_session_id().is_some());
        }
        let mut conflict = charge_core_sample(0);
        conflict.power_state = Some(PowerState::Charging);
        conflict.charger_state = Some(ChargerState::Done);
        let mut detector = ChargeDetectorState::new(conflict.vehicle_id);
        assert!(matches!(
            detector.process(&conflict),
            ChargeEvent::SessionStarted
        ));
        conflict.ts = charge_core_at(1);
        assert!(matches!(
            detector.process(&conflict),
            ChargeEvent::SessionEnded(_)
        ));
    }

    #[test]
    fn charge_core_power_and_remaining_time_alone_do_not_start() {
        for remaining in [None, Some(-1), Some(0), Some(1)] {
            let mut detector = ChargeDetectorState::new(Uuid::from_u128(1));
            let mut sample = charge_core_sample(0);
            sample.time_to_end_of_charge_min = remaining;
            sample.power_kw = Some(-10.0);
            assert!(matches!(detector.process(&sample), ChargeEvent::NoChange));
            assert_eq!(detector.active_session_id(), None);
        }
    }

    #[test]
    fn charge_core_end_signals_override_remaining_time_except_connected_idle() {
        for signal in 0..4 {
            for remaining in [None, Some(-1), Some(0), Some(1)] {
                let mut detector = charge_core_started();
                let mut sample = charge_core_sample(60);
                sample.time_to_end_of_charge_min = remaining;
                match signal {
                    0 => sample.charger_state = Some(ChargerState::Done),
                    1 => sample.charger_state = Some(ChargerState::Disconnected),
                    2 => sample.charger_status = Some("chrgr_sts_not_connected".into()),
                    _ => sample.charger_status = Some("chrgr_sts_connected_no_chrg".into()),
                }
                let ended = matches!(detector.process(&sample), ChargeEvent::SessionEnded(_));
                assert_eq!(ended, signal != 3 || remaining != Some(1));
            }
        }
    }

    #[test]
    fn charge_core_timeout_uses_whole_seconds_and_refreshes_before_checking() {
        for milliseconds in [1_800_000, 1_800_999, 1_801_000] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(0);
            sample.ts += Duration::milliseconds(milliseconds);
            match detector.process(&sample) {
                ChargeEvent::SessionEnded(session) => {
                    assert_eq!(milliseconds, 1_801_000);
                    assert_eq!(session.ended_at, sample.ts);
                }
                ChargeEvent::NoChange => assert!(milliseconds < 1_801_000),
                other => panic!("unexpected {other:?}"),
            }
        }
        for remaining in [None, Some(-1), Some(0), Some(1)] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(1_801);
            sample.time_to_end_of_charge_min = remaining;
            assert_eq!(
                matches!(detector.process(&sample), ChargeEvent::NoChange),
                remaining == Some(1)
            );
        }
        let mut detector = charge_core_started();
        let mut resumed = charge_core_sample(3_600);
        resumed.charger_state = Some(ChargerState::Charging);
        assert!(matches!(detector.process(&resumed), ChargeEvent::NoChange));
    }

    #[test]
    fn charge_core_first_power_sets_clock_and_end_power_contributes() {
        let mut detector = ChargeDetectorState::new(Uuid::from_u128(1));
        let mut start = charge_core_sample(0);
        start.charger_state = Some(ChargerState::Charging);
        start.power_kw = Some(200.0);
        detector.process(&start);
        let mut end = charge_core_sample(60);
        end.charger_state = Some(ChargerState::Done);
        end.power_kw = Some(-12.0);
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completion");
        };
        assert_eq!(session.energy_used_wh, Some(200.0));
        assert_eq!(session.peak_charge_kw, Some(12.0));
        assert_eq!(session.avg_charge_rate_kw, Some(12.0));
        assert_eq!(session.charger_type.as_deref(), Some("ac"));
    }

    #[test]
    fn charge_core_power_ceiling_spike_and_classification_boundaries() {
        for (kw, expected) in [
            (11.999, Some("ac")),
            (12.0, Some("ac")),
            (24.999, Some("ac")),
            (25.0, Some("dc")),
            (300.0, Some("dc")),
            (300.001, None),
        ] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(60);
            sample.power_kw = Some(kw);
            detector.process(&sample);
            let session = charge_core_finish(&mut detector, 120);
            assert_eq!(session.charger_type.as_deref(), expected, "{kw}");
            assert_eq!(session.peak_charge_kw, expected.map(|_| kw));
        }
        for (baseline, next, accepted) in
            [(8.0, 40.0, true), (8.0, 40.001, false), (50.0, 300.0, true)]
        {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(60);
            sample.power_kw = Some(baseline);
            detector.process(&sample);
            sample.ts = charge_core_at(120);
            sample.power_kw = Some(next);
            detector.process(&sample);
            let session = charge_core_finish(&mut detector, 180);
            let total =
                baseline * 1_000.0 / 60.0 + if accepted { next * 1_000.0 / 60.0 } else { 0.0 };
            assert!((session.energy_used_wh.unwrap() - total).abs() < 1e-9);
            assert_eq!(
                session.peak_charge_kw,
                Some(if accepted { next } else { baseline })
            );
        }
    }

    #[test]
    fn charge_core_missing_zero_and_nonfinite_power_do_not_add_energy() {
        for power in [
            None,
            Some(0.0),
            Some(-0.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(60);
            sample.power_kw = power;
            detector.process(&sample);
            let session = charge_core_finish(&mut detector, 120);
            assert_eq!(session.energy_used_wh, None);
            assert_eq!(session.peak_charge_kw, None);
            assert_eq!(session.avg_charge_rate_kw, None);
            assert_eq!(session.charger_type, None);
        }
    }

    #[test]
    fn charge_core_grid_estimate_does_not_depend_on_capacity_quality() {
        for capacity in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(60);
            sample.power_kw = Some(12.0);
            sample.battery_level = Some(40.0);
            sample.battery_capacity_wh = capacity;
            detector.process(&sample);
            sample.ts = charge_core_at(120);
            sample.battery_level = Some(50.0);
            sample.charger_state = Some(ChargerState::Done);
            let ChargeEvent::SessionEnded(session) = detector.process(&sample) else {
                panic!("expected completion");
            };
            assert_eq!(session.energy_used_wh, Some(400.0));
            assert_eq!(session.avg_charge_rate_kw, Some(12.0));
            assert_eq!(session.peak_charge_kw, Some(12.0));
        }
    }

    #[test]
    fn charge_core_snapshot_without_power_clock_preserves_identity_and_partial_location() {
        for energy in [0.0, -1.0, f64::NAN] {
            let session_id = Uuid::from_u128(20);
            let mut detector = ChargeDetectorState::from_snapshot(
                Uuid::from_u128(1),
                ActiveChargeSessionSnapshot {
                    session_id,
                    started_at: charge_core_at(0),
                    location_lat: Some(30.0),
                    location_lng: None,
                    soc_start: Some(40.0),
                    last_soc: Some(40.0),
                    charge_limit: None,
                    battery_capacity_wh: None,
                    last_charge_ts: charge_core_at(0),
                    last_power_ts: None,
                    energy_used_wh: energy,
                    peak_charge_kw: -1.0,
                },
            );
            let mut sample = charge_core_sample(60);
            sample.latitude = Some(40.0);
            sample.longitude = Some(50.0);
            sample.power_kw = Some(12.0);
            detector.process(&sample);
            let session = charge_core_finish(&mut detector, 120);
            assert_eq!(session.session_id, session_id);
            assert_eq!(session.energy_used_wh, None);
            assert_eq!(session.peak_charge_kw, Some(12.0));
            assert_eq!(
                (session.location_lat, session.location_lng),
                (Some(30.0), None)
            );
        }
    }

    #[test]
    fn charge_core_soc_options_pack_fallback_and_grid_precedence() {
        for (start_soc, end_soc, capacity, grid_kw, expected_pack, expected_avg) in [
            (None, None, None, None, None, None),
            (
                Some(0.0),
                Some(10.0),
                Some(100_000.0),
                None,
                Some(10_000.0),
                Some(10.0),
            ),
            (Some(40.0), Some(40.0), Some(100_000.0), None, None, None),
            (Some(40.0), Some(39.0), Some(100_000.0), None, None, None),
            (Some(40.0), Some(50.0), None, None, None, None),
            (
                Some(40.0),
                Some(50.0),
                Some(100_000.0),
                Some(20.0),
                Some(10_000.0),
                Some(20.0),
            ),
        ] {
            let mut detector = ChargeDetectorState::new(Uuid::from_u128(1));
            let mut sample = charge_core_sample(0);
            sample.charger_state = Some(ChargerState::Charging);
            sample.battery_level = start_soc;
            sample.battery_capacity_wh = capacity;
            detector.process(&sample);
            sample.ts = charge_core_at(3_600);
            sample.charger_state = Some(ChargerState::Done);
            sample.battery_level = end_soc;
            sample.power_kw = grid_kw;
            let ChargeEvent::SessionEnded(session) = detector.process(&sample) else {
                panic!("expected completion");
            };
            assert_eq!(session.energy_added_wh, expected_pack);
            assert_eq!(session.avg_charge_rate_kw, expected_avg);
        }
    }

    #[test]
    fn charge_core_location_waits_for_complete_finite_pair() {
        for (lat, lng) in [
            (None, Some(1.0)),
            (Some(1.0), None),
            (Some(0.0), Some(0.0)),
            (Some(f64::NAN), Some(1.0)),
            (Some(1.0), Some(f64::INFINITY)),
        ] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(30);
            sample.latitude = lat;
            sample.longitude = lng;
            detector.process(&sample);
            sample.ts = charge_core_at(60);
            sample.latitude = Some(0.0);
            sample.longitude = Some(1.0);
            detector.process(&sample);
            let session = charge_core_finish(&mut detector, 90);
            assert_eq!(
                (session.location_lat, session.location_lng),
                (Some(0.0), Some(1.0))
            );
        }
    }

    #[test]
    fn charge_core_duplicate_timestamp_has_zero_duration() {
        let mut detector = charge_core_started();
        let mut sample = charge_core_sample(0);
        sample.power_kw = Some(12.0);
        detector.process(&sample);
        let session = charge_core_finish(&mut detector, 0);
        assert_eq!(session.energy_used_wh, None);
        assert_eq!(session.peak_charge_kw, Some(12.0));
        assert_eq!(session.avg_charge_rate_kw, None);
    }

    #[test]
    fn charge_core_restart_matches_uninterrupted_completion_and_identity() {
        let mut uninterrupted = charge_core_started();
        let mut sample = charge_core_sample(60);
        sample.power_kw = Some(12.0);
        sample.battery_level = Some(40.0);
        sample.battery_limit = Some(80.0);
        sample.battery_capacity_wh = Some(100_000.0);
        sample.latitude = Some(30.0);
        sample.longitude = Some(-97.0);
        sample.time_to_end_of_charge_min = Some(10);
        uninterrupted.process(&sample);
        let id = uninterrupted.active_session_id().unwrap();
        let mut restarted = ChargeDetectorState::from_snapshot(
            sample.vehicle_id,
            ActiveChargeSessionSnapshot {
                session_id: id,
                started_at: charge_core_at(0),
                location_lat: sample.latitude,
                location_lng: sample.longitude,
                soc_start: sample.battery_level,
                last_soc: sample.battery_level,
                charge_limit: sample.battery_limit,
                battery_capacity_wh: sample.battery_capacity_wh,
                last_charge_ts: sample.ts,
                last_power_ts: Some(sample.ts),
                energy_used_wh: 200.0,
                peak_charge_kw: 12.0,
            },
        );
        sample.ts = charge_core_at(120);
        sample.battery_level = Some(41.0);
        sample.charger_state = Some(ChargerState::Done);
        let actual = restarted.process(&sample);
        let expected = uninterrupted.process(&sample);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        let ChargeEvent::SessionEnded(session) = actual else {
            panic!("expected completion")
        };
        assert_eq!(session.session_id, id);
        assert_eq!(session.energy_used_wh, Some(400.0));
        assert_eq!(session.energy_added_wh, Some(1_000.0));
        assert_eq!(restarted.active_session_id(), None);
    }

    #[test]
    fn charge_core_next_session_resets_session_fields_but_keeps_vehicle_metadata() {
        let mut detector = charge_core_started();
        let mut sample = charge_core_sample(60);
        sample.battery_level = Some(40.0);
        sample.battery_limit = Some(80.0);
        sample.battery_capacity_wh = Some(100_000.0);
        sample.latitude = Some(30.0);
        sample.longitude = Some(-97.0);
        detector.process(&sample);
        let first = charge_core_finish(&mut detector, 120);
        sample = charge_core_sample(180);
        sample.charger_state = Some(ChargerState::Charging);
        detector.process(&sample);
        let second = charge_core_finish(&mut detector, 240);
        assert_ne!(first.session_id, second.session_id);
        assert_eq!(second.started_at, charge_core_at(180));
        assert_eq!(
            (second.location_lat, second.location_lng, second.soc_start),
            (None, None, None)
        );
        assert_eq!(
            (
                second.soc_end,
                second.charge_limit,
                second.battery_capacity_wh
            ),
            (Some(40.0), Some(80.0), Some(100_000.0))
        );
        assert_eq!(second.energy_used_wh, None);
    }

    #[test]
    fn charge_core_vehicle_instances_keep_independent_sessions() {
        let mut first = charge_core_started();
        let mut second = ChargeDetectorState::new(Uuid::from_u128(2));
        let mut sample = TelemetryEvent::empty(Uuid::from_u128(2), charge_core_at(60));
        sample.charger_state = Some(ChargerState::Charging);
        sample.battery_level = Some(60.0);
        second.process(&sample);
        let second_id = second.active_session_id();
        let first_session = charge_core_finish(&mut first, 120);
        assert_eq!(first_session.vehicle_id, Uuid::from_u128(1));
        assert_eq!(first_session.soc_start, None);
        assert_eq!(second.active_session_id(), second_id);
        sample.ts = charge_core_at(180);
        sample.charger_state = Some(ChargerState::Done);
        let ChargeEvent::SessionEnded(second_session) = second.process(&sample) else {
            panic!("expected completion")
        };
        assert_eq!(second_session.vehicle_id, Uuid::from_u128(2));
        assert_eq!(second_session.soc_start, Some(60.0));
        assert_ne!(first_session.session_id, second_session.session_id);
    }

    #[test]
    fn charge_core_out_of_order_power_cannot_subtract_energy_or_rewind_clock() {
        for (at, expected_peak) in [
            (charge_core_at(30), 12.0),
            (charge_core_at(60) - Duration::nanoseconds(1), 12.0),
            (charge_core_at(60), 24.0),
        ] {
            let mut detector = charge_core_started();
            let mut sample = charge_core_sample(60);
            sample.power_kw = Some(12.0);
            detector.process(&sample);
            sample.ts = at;
            sample.power_kw = Some(24.0);
            detector.process(&sample);
            sample.ts = charge_core_at(120);
            sample.power_kw = Some(12.0);
            sample.charger_state = Some(ChargerState::Done);
            let ChargeEvent::SessionEnded(session) = detector.process(&sample) else {
                panic!("expected completion");
            };
            assert_eq!(session.energy_used_wh, Some(400.0));
            assert_eq!(session.peak_charge_kw, Some(expected_peak));
        }
    }

    #[test]
    fn charge_core_restart_rejects_power_older_than_snapshot_clock() {
        let session_id = Uuid::from_u128(20);
        let mut detector = ChargeDetectorState::from_snapshot(
            Uuid::from_u128(1),
            ActiveChargeSessionSnapshot {
                session_id,
                started_at: charge_core_at(0),
                location_lat: None,
                location_lng: None,
                soc_start: None,
                last_soc: None,
                charge_limit: None,
                battery_capacity_wh: None,
                last_charge_ts: charge_core_at(60),
                last_power_ts: Some(charge_core_at(60)),
                energy_used_wh: 200.0,
                peak_charge_kw: 12.0,
            },
        );
        let mut late = charge_core_sample(30);
        late.power_kw = Some(24.0);
        detector.process(&late);
        let mut end = charge_core_sample(120);
        end.power_kw = Some(12.0);
        end.charger_state = Some(ChargerState::Done);
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completion");
        };
        assert_eq!(session.session_id, session_id);
        assert_eq!(session.energy_used_wh, Some(400.0));
        assert_eq!(session.peak_charge_kw, Some(12.0));
    }

    fn event(
        vehicle_id: Uuid,
        ts: DateTime<Utc>,
        charger_state: Option<ChargerState>,
    ) -> TelemetryEvent {
        TelemetryEvent {
            charger_state,
            ..TelemetryEvent::empty(vehicle_id, ts)
        }
    }

    #[test]
    fn timed_out_session_ends_when_charging_was_last_seen() {
        // Shape of the Nov 6 R2 DC fast charge: no power readings, 28% to 72%
        // in 16 minutes, then "unknown" charger state until the timeout.
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(2);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.battery_level = Some(28.0);
        start.battery_capacity_wh = Some(91_500.0);
        detector.process(&start);

        let last_charging = started_at + Duration::minutes(16);
        let mut last = event(vehicle_id, last_charging, None);
        last.battery_level = Some(72.0);
        last.time_to_end_of_charge_min = Some(9);
        assert!(matches!(detector.process(&last), ChargeEvent::NoChange));

        // The level falls after charging stops (the car drives away without
        // odometer readings here), and the timeout must not keep that level.
        let mut idle = event(vehicle_id, last_charging + Duration::minutes(10), None);
        idle.battery_level = Some(64.0);
        assert!(matches!(detector.process(&idle), ChargeEvent::NoChange));

        let noticed = event(vehicle_id, last_charging + Duration::minutes(31), None);
        let ChargeEvent::SessionEnded(session) = detector.process(&noticed) else {
            panic!("expected the timeout to close the session");
        };

        assert_eq!(session.ended_at, last_charging);
        assert_eq!(session.soc_end, Some(72.0));
        let avg = session
            .avg_charge_rate_kw
            .expect("average rate from SOC delta");
        assert!(avg > 140.0, "average over charging time only, got {avg}");
        assert_eq!(session.charger_type.as_deref(), Some("dc"));
        assert_eq!(session.peak_charge_kw, None);
    }

    #[test]
    fn driving_away_ends_the_session_at_the_last_charging_reading() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(2);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.battery_level = Some(40.0);
        start.battery_capacity_wh = Some(91_500.0);
        start.odometer_miles = Some(12_000.0);
        detector.process(&start);

        let last_charging = started_at + Duration::minutes(20);
        let mut last = event(vehicle_id, last_charging, None);
        last.battery_level = Some(71.0);
        last.time_to_end_of_charge_min = Some(11);
        detector.process(&last);

        let mut parked = event(vehicle_id, last_charging + Duration::minutes(2), None);
        parked.odometer_miles = Some(12_000.04);
        assert!(matches!(detector.process(&parked), ChargeEvent::NoChange));

        let mut driving = event(vehicle_id, last_charging + Duration::minutes(6), None);
        driving.battery_level = Some(69.5);
        driving.odometer_miles = Some(12_002.3);
        let ChargeEvent::SessionEnded(session) = detector.process(&driving) else {
            panic!("motion must end the session");
        };
        assert_eq!(session.ended_at, last_charging);
        assert_eq!(session.soc_end, Some(71.0));
        assert_eq!(session.charger_type.as_deref(), Some("dc"));
        assert_eq!(detector.active_session_id(), None);
    }

    #[test]
    fn keeps_a_dc_ramp_that_starts_at_a_low_rate() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(1);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        detector.process(&event(vehicle_id, started_at, Some(ChargerState::Charging)));
        let mut handshake = event(
            vehicle_id,
            started_at + Duration::seconds(10),
            Some(ChargerState::Charging),
        );
        handshake.power_kw = Some(6.0);
        detector.process(&handshake);
        let mut ramp = event(
            vehicle_id,
            started_at + Duration::seconds(40),
            Some(ChargerState::Charging),
        );
        ramp.power_kw = Some(184.0);
        detector.process(&ramp);
        // A second DC-rate reading confirms the ramp.
        let mut steady = event(
            vehicle_id,
            started_at + Duration::seconds(70),
            Some(ChargerState::Charging),
        );
        steady.power_kw = Some(186.4);
        detector.process(&steady);

        let end = event(
            vehicle_id,
            started_at + Duration::minutes(20),
            Some(ChargerState::Done),
        );
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };
        assert_eq!(session.peak_charge_kw, Some(186.4));
        assert_eq!(session.charger_type.as_deref(), Some("dc"));
    }

    #[test]
    fn an_early_single_glitch_cannot_make_an_ac_session_dc() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(3);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        detector.process(&event(vehicle_id, started_at, Some(ChargerState::Charging)));
        for (seconds, kw) in [(30_i64, 11.0_f64), (60, 45.0), (90, 11.2), (120, 11.1)] {
            let mut reading = event(
                vehicle_id,
                started_at + Duration::seconds(seconds),
                Some(ChargerState::Charging),
            );
            reading.power_kw = Some(kw);
            detector.process(&reading);
        }
        let end = event(
            vehicle_id,
            started_at + Duration::minutes(40),
            Some(ChargerState::Done),
        );
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };
        assert_eq!(session.peak_charge_kw, Some(11.2));
        assert_eq!(session.charger_type.as_deref(), Some("ac"));
    }

    #[test]
    fn gnss_speed_jitter_and_stale_odometer_do_not_end_a_charge() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(1);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.odometer_miles = Some(12_000.0);
        detector.process(&start);
        let session_id = detector.active_session_id();

        // Parking-garage GNSS jitter on a frame without charger signals.
        let mut jitter = event(vehicle_id, started_at + Duration::minutes(3), None);
        jitter.speed_mph = Some(3.4);
        assert!(matches!(detector.process(&jitter), ChargeEvent::NoChange));

        let mut charging = event(
            vehicle_id,
            started_at + Duration::minutes(6),
            Some(ChargerState::Charging),
        );
        charging.speed_mph = Some(4.0);
        assert!(matches!(detector.process(&charging), ChargeEvent::NoChange));

        // A delayed odometer reading taken before the last charging reading.
        let mut stale = event(vehicle_id, started_at + Duration::minutes(8), None);
        stale.odometer_miles = Some(12_004.0);
        stale.odometer_miles_ts = Some(started_at + Duration::minutes(5));
        assert!(matches!(detector.process(&stale), ChargeEvent::NoChange));

        // One 1 km odometer step is below the motion limit.
        let mut step = event(vehicle_id, started_at + Duration::minutes(9), None);
        step.odometer_miles = Some(12_000.62);
        assert!(matches!(detector.process(&step), ChargeEvent::NoChange));
        assert_eq!(detector.active_session_id(), session_id);
    }

    #[test]
    fn rising_soc_counts_as_charging_without_charger_signals() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(2);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.battery_level = Some(40.0);
        start.battery_capacity_wh = Some(91_500.0);
        detector.process(&start);
        for (minutes, soc) in [(10_i64, 46.0_f64), (20, 52.0), (25, 52.2)] {
            let mut sample = event(vehicle_id, started_at + Duration::minutes(minutes), None);
            sample.battery_level = Some(soc);
            detector.process(&sample);
        }
        let noticed = event(vehicle_id, started_at + Duration::minutes(51), None);
        let ChargeEvent::SessionEnded(session) = detector.process(&noticed) else {
            panic!("expected the timeout to close the session");
        };
        // 52.2 is not 0.5 above 52.0, so the charge ended at minute 20.
        assert_eq!(session.ended_at, started_at + Duration::minutes(20));
        assert_eq!(session.soc_end, Some(52.0));
    }

    #[test]
    fn a_charge_without_later_evidence_keeps_the_timeout_close() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(2);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.battery_level = Some(40.0);
        detector.process(&start);
        let noticed_at = started_at + Duration::minutes(31);
        let ChargeEvent::SessionEnded(session) =
            detector.process(&event(vehicle_id, noticed_at, None))
        else {
            panic!("expected the timeout to close the session");
        };
        assert_eq!(session.ended_at, noticed_at);
    }

    #[test]
    fn power_after_the_last_charging_reading_is_not_kept() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(2);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        detector.process(&event(vehicle_id, started_at, Some(ChargerState::Charging)));
        let mut charging = event(
            vehicle_id,
            started_at + Duration::minutes(10),
            Some(ChargerState::Charging),
        );
        charging.power_kw = Some(11.0);
        detector.process(&charging);
        let last_charging = started_at + Duration::minutes(40);
        let mut last = event(vehicle_id, last_charging, Some(ChargerState::Charging));
        last.power_kw = Some(11.0);
        detector.process(&last);

        // Drive power after the charge stopped, without charger signals.
        let mut drive = event(vehicle_id, last_charging + Duration::minutes(5), None);
        drive.power_kw = Some(15.0);
        detector.process(&drive);

        let noticed = event(vehicle_id, last_charging + Duration::minutes(31), None);
        let ChargeEvent::SessionEnded(session) = detector.process(&noticed) else {
            panic!("expected the timeout to close the session");
        };
        assert_eq!(session.ended_at, last_charging);
        assert_eq!(session.peak_charge_kw, Some(11.0));
        // 11 kW from the start to minute 40; the drive minutes are not kept.
        let energy = session.energy_used_wh.expect("integrated energy");
        assert!(
            (energy - 7_333.3).abs() < 1.0,
            "40 min at 11 kW, got {energy}"
        );
    }

    #[test]
    fn still_drops_a_late_spike_on_an_ac_session() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::hours(3);
        let mut detector = ChargeDetectorState::new(vehicle_id);

        // The first event only starts the session, so the AC baseline comes
        // from the next reading.
        detector.process(&event(vehicle_id, started_at, Some(ChargerState::Charging)));
        let mut steady = event(
            vehicle_id,
            started_at + Duration::minutes(2),
            Some(ChargerState::Charging),
        );
        steady.power_kw = Some(11.0);
        detector.process(&steady);
        let mut spike = event(
            vehicle_id,
            started_at + Duration::minutes(25),
            Some(ChargerState::Charging),
        );
        spike.power_kw = Some(250.0);
        detector.process(&spike);

        let end = event(
            vehicle_id,
            started_at + Duration::minutes(50),
            Some(ChargerState::Done),
        );
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };
        assert_eq!(session.peak_charge_kw, Some(11.0));
        assert_eq!(session.charger_type.as_deref(), Some("ac"));
    }

    #[test]
    fn fills_start_soc_and_location_from_later_charge_samples() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now();
        let mut detector = ChargeDetectorState::new(vehicle_id);

        assert!(matches!(
            detector.process(&event(vehicle_id, started_at, Some(ChargerState::Charging))),
            ChargeEvent::SessionStarted
        ));

        let mut follow_up = event(vehicle_id, started_at + Duration::minutes(2), None);
        follow_up.battery_level = Some(42.0);
        follow_up.battery_capacity_wh = Some(111_000.0);
        follow_up.latitude = Some(29.81);
        follow_up.longitude = Some(-95.38);
        assert!(matches!(
            detector.process(&follow_up),
            ChargeEvent::NoChange
        ));

        let mut end = event(
            vehicle_id,
            started_at + Duration::minutes(62),
            Some(ChargerState::Done),
        );
        end.battery_level = Some(52.0);
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };

        assert_eq!(session.soc_start, Some(42.0));
        assert_eq!(session.location_lat, Some(29.81));
        assert_eq!(session.location_lng, Some(-95.38));
        assert!(session.energy_added_wh.is_some());
    }

    #[test]
    fn skips_zero_zero_location_until_a_real_sample_arrives() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now();
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.latitude = Some(0.0);
        start.longitude = Some(0.0);
        assert!(matches!(
            detector.process(&start),
            ChargeEvent::SessionStarted
        ));

        let mut follow_up = event(vehicle_id, started_at + Duration::minutes(3), None);
        follow_up.latitude = Some(29.81);
        follow_up.longitude = Some(-95.38);
        follow_up.battery_level = Some(43.0);
        assert!(matches!(
            detector.process(&follow_up),
            ChargeEvent::NoChange
        ));

        let mut end = event(
            vehicle_id,
            started_at + Duration::minutes(63),
            Some(ChargerState::Done),
        );
        end.battery_level = Some(55.0);
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };

        assert_eq!(session.location_lat, Some(29.81));
        assert_eq!(session.location_lng, Some(-95.38));
    }

    #[test]
    fn classifies_from_average_rate_when_peak_power_is_missing() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now();
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.battery_level = Some(40.0);
        start.battery_capacity_wh = Some(100_000.0);
        detector.process(&start);

        let mut end = event(
            vehicle_id,
            started_at + Duration::minutes(70),
            Some(ChargerState::Done),
        );
        end.battery_level = Some(50.0);
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };

        assert_eq!(session.charger_type.as_deref(), Some("ac"));
    }

    #[test]
    fn keeps_partial_charging_patches_in_one_session() {
        let vehicle_id = Uuid::new_v4();
        let started_at = Utc::now();
        let mut detector = ChargeDetectorState::new(vehicle_id);

        let mut start = event(vehicle_id, started_at, Some(ChargerState::Charging));
        start.battery_level = Some(33.0);
        start.battery_capacity_wh = Some(111_000.0);
        assert!(matches!(
            detector.process(&start),
            ChargeEvent::SessionStarted
        ));
        let session_id = detector.active_session_id();

        let mut patch = event(vehicle_id, started_at + Duration::minutes(45), None);
        patch.battery_level = Some(38.0);
        patch.time_to_end_of_charge_min = Some(250);
        assert!(matches!(detector.process(&patch), ChargeEvent::NoChange));
        assert_eq!(detector.active_session_id(), session_id);

        let mut end = event(vehicle_id, started_at + Duration::minutes(120), None);
        end.battery_level = Some(75.0);
        end.charger_status = Some("chrgr_sts_connected_no_chrg".to_string());
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };

        assert_eq!(session.session_id, session_id.unwrap());
        assert_eq!(session.soc_start, Some(33.0));
        assert_eq!(session.soc_end, Some(75.0));
    }

    #[test]
    fn rehydrated_session_keeps_existing_session_id_until_completion() {
        let vehicle_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::minutes(45);
        let mut detector = ChargeDetectorState::from_snapshot(
            vehicle_id,
            ActiveChargeSessionSnapshot {
                session_id,
                started_at,
                location_lat: Some(29.81),
                location_lng: Some(-95.38),
                soc_start: Some(40.0),
                last_soc: Some(48.0),
                charge_limit: Some(80.0),
                battery_capacity_wh: Some(111_000.0),
                last_charge_ts: started_at + Duration::minutes(40),
                last_power_ts: Some(started_at + Duration::minutes(40)),
                energy_used_wh: 18_000.0,
                peak_charge_kw: 10.5,
            },
        );

        let mut patch = event(vehicle_id, started_at + Duration::minutes(50), None);
        patch.battery_level = Some(50.0);
        patch.time_to_end_of_charge_min = Some(25);
        assert!(matches!(detector.process(&patch), ChargeEvent::NoChange));
        assert_eq!(detector.active_session_id(), Some(session_id));

        let mut end = event(
            vehicle_id,
            started_at + Duration::minutes(70),
            Some(ChargerState::Done),
        );
        end.battery_level = Some(60.0);
        let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
            panic!("expected completed charge session");
        };

        assert_eq!(session.session_id, session_id);
        assert_eq!(session.soc_start, Some(40.0));
        assert_eq!(session.soc_end, Some(60.0));
        assert!(session.energy_used_wh.unwrap_or_default() >= 18_000.0);
    }
}

#[cfg(test)]
mod checkpoint_roundtrip_tests {
    use super::*;

    #[test]
    fn complete_detector_and_completion_float_bits_survive_json_serialization() {
        let vehicle = Uuid::from_u128(120);
        let at = DateTime::parse_from_rfc3339("2026-10-16T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = ChargeDetectorState::new(vehicle);
        let mut metadata = TelemetryEvent::empty(vehicle, at);
        metadata.battery_level = Some(f64::from_bits(0x7ff8_0000_0000_0042));
        metadata.battery_limit = Some(f64::INFINITY);
        metadata.battery_capacity_wh = Some(f64::NEG_INFINITY);
        state.process(&metadata);
        let roundtrip = |state: &ChargeDetectorState| -> ChargeDetectorState {
            serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap()
        };
        let restored = roundtrip(&state);
        assert_eq!(
            restored.battery.soc.unwrap().to_bits(),
            metadata.battery_level.unwrap().to_bits()
        );
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&state).unwrap()
        );
        let mut start = TelemetryEvent::empty(vehicle, at);
        start.charger_state = Some(ChargerState::Charging);
        start.battery_level = Some(10.0);
        start.battery_capacity_wh = Some(f64::INFINITY);
        state.process(&start);
        for (millis, power) in [
            (1, 12.123456789012345),
            (500, 13.123456789012345),
            (300, 100.0),
            (900, f64::NAN),
            (1000, 12.87654321098765),
        ] {
            let mut event =
                TelemetryEvent::empty(vehicle, at + chrono::Duration::milliseconds(millis));
            event.power_kw = Some(power);
            state.process(&event);
        }
        let mut restored = roundtrip(&state);
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&state).unwrap()
        );
        let mut end = TelemetryEvent::empty(vehicle, at + chrono::Duration::seconds(3));
        end.charger_state = Some(ChargerState::Done);
        end.battery_level = Some(20.0);
        let ChargeEvent::SessionEnded(expected) = state.process(&end) else {
            panic!("completion")
        };
        let ChargeEvent::SessionEnded(actual) = restored.process(&end) else {
            panic!("restored completion")
        };
        assert_eq!(actual.energy_added_wh, Some(f64::INFINITY));
        let restored: CompletedChargeSession =
            serde_json::from_slice(&serde_json::to_vec(&actual).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(expected).unwrap(),
            serde_json::to_value(restored).unwrap()
        );
    }
}

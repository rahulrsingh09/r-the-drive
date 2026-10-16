//! Reconstruct the detector's accepted-power clock from ordered persisted samples.
use chrono::{DateTime, Utc};

pub(super) struct ReconstructedChargePower {
    pub last_power_ts: DateTime<Utc>,
    pub energy_used_wh: f64,
    pub peak_charge_kw: f64,
}

impl ReconstructedChargePower {
    pub(super) fn from_samples(
        started_at: DateTime<Utc>,
        samples: impl IntoIterator<Item = (DateTime<Utc>, Option<f64>)>,
    ) -> Self {
        let mut power = Self {
            last_power_ts: started_at,
            energy_used_wh: 0.0,
            peak_charge_kw: 0.0,
        };
        // The opening frame anchors time but does not establish a peak.
        for (at, reading) in samples.into_iter().skip(1) {
            let Some(kw) = reading.map(f64::abs) else {
                continue;
            };
            let spike = power.peak_charge_kw > 0.0
                && power.peak_charge_kw < 50.0
                && kw > 5.0 * power.peak_charge_kw;
            if !(kw > 0.0 && kw <= 300.0) || spike || at < power.last_power_ts {
                continue;
            }
            let hours = (at - power.last_power_ts).num_milliseconds() as f64 / 3_600_000.0;
            power.energy_used_wh += kw * 1_000.0 * hours;
            power.peak_charge_kw = power.peak_charge_kw.max(kw);
            power.last_power_ts = at;
        }
        power
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ingestion::charge_detector::{ChargeDetectorState, ChargeEvent},
        models::telemetry::{ChargerState, TelemetryEvent},
    };
    use chrono::Duration;
    use uuid::Uuid;

    #[test]
    fn reconstruction_tracks_detector_acceptance_at_boundaries() {
        let start: DateTime<Utc> = "2026-10-16T10:00:00Z".parse().unwrap();
        for powers in [
            vec![None, Some(10.0), Some(50.0), Some(300.0)],
            vec![Some(100.0), Some(10.0), Some(50.001), Some(10.0)],
            vec![Some(12.0), Some(300.0), Some(300.001), Some(-50.0)],
            vec![
                None,
                Some(f64::NAN),
                Some(f64::INFINITY),
                Some(0.0),
                Some(-12.0),
            ],
        ] {
            let mut detector = ChargeDetectorState::new(Uuid::nil());
            let samples: Vec<_> = powers
                .into_iter()
                .enumerate()
                .map(|(i, kw)| {
                    let at = start + Duration::milliseconds(i as i64 * 1001);
                    let mut event = TelemetryEvent::empty(Uuid::nil(), at);
                    event.charger_state = Some(ChargerState::Charging);
                    event.power_kw = kw;
                    detector.process(&event);
                    (at, kw)
                })
                .collect();
            let power = ReconstructedChargePower::from_samples(start, samples.clone());
            let mut end = TelemetryEvent::empty(Uuid::nil(), samples.last().unwrap().0);
            end.charger_state = Some(ChargerState::Done);
            let ChargeEvent::SessionEnded(session) = detector.process(&end) else {
                panic!("end")
            };
            assert_eq!(power.energy_used_wh, session.energy_used_wh.unwrap_or(0.0));
            assert_eq!(power.peak_charge_kw, session.peak_charge_kw.unwrap_or(0.0));
        }
    }

    #[test]
    fn rejected_late_and_empty_readings_do_not_move_reconstructed_clock() {
        let at: DateTime<Utc> = "2026-10-16T10:00:00Z".parse().unwrap();
        let rebuilt = ReconstructedChargePower::from_samples(
            at,
            [
                (at, Some(200.0)),
                (at + Duration::seconds(60), Some(12.0)),
                (at + Duration::seconds(30), Some(24.0)),
                (at + Duration::seconds(120), Some(61.0)),
                (at + Duration::seconds(180), None),
            ],
        );
        assert_eq!(rebuilt.last_power_ts, at + Duration::seconds(60));
        assert_eq!(rebuilt.energy_used_wh, 200.0);
        assert_eq!(rebuilt.peak_charge_kw, 12.0);
    }
}

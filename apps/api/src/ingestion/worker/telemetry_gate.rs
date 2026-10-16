//! Selection is planned independently from advancing the coalesced observation.
use super::*;

#[derive(Debug)]
pub(super) struct TelemetryPersistenceGate {
    vehicle_id: Uuid,
    pub(super) latest: Option<TelemetryEvent>,
    pub(super) last_persisted: Option<TelemetryEvent>,
    pub(super) last_persisted_at: Option<DateTime<Utc>>,
    last_persisted_location: Option<(f64, f64)>,
}

pub(super) struct Selection {
    pub(super) decision: PersistenceDecision,
    next: Option<TelemetryEvent>,
}

struct Metric {
    read: fn(&TelemetryEvent) -> Option<f64>,
    delta: f64,
}

struct Thresholds {
    minimum_seconds: Option<i64>,
    movement_m: f64,
    metrics: &'static [Metric],
    hvac: bool,
}

const DRIVING: Thresholds = Thresholds {
    minimum_seconds: Some(5),
    movement_m: 25.0,
    metrics: &[
        Metric {
            read: |e| e.speed_mph,
            delta: 1.0,
        },
        Metric {
            read: |e| e.battery_level,
            delta: 0.1,
        },
    ],
    hvac: false,
};
const CHARGING: Thresholds = Thresholds {
    minimum_seconds: Some(15),
    movement_m: f64::INFINITY,
    metrics: &[
        Metric {
            read: |e| e.battery_level,
            delta: 0.1,
        },
        Metric {
            read: |e| e.power_kw,
            delta: 0.5,
        },
    ],
    hvac: false,
};
const PARKED: Thresholds = Thresholds {
    minimum_seconds: None,
    movement_m: 100.0,
    metrics: &[
        Metric {
            read: |e| e.battery_level,
            delta: 0.5,
        },
        Metric {
            read: |e| e.tire_fl_psi,
            delta: 0.5,
        },
        Metric {
            read: |e| e.tire_fr_psi,
            delta: 0.5,
        },
        Metric {
            read: |e| e.tire_rl_psi,
            delta: 0.5,
        },
        Metric {
            read: |e| e.tire_rr_psi,
            delta: 0.5,
        },
        Metric {
            read: |e| e.cabin_temp_c,
            delta: 2.0,
        },
        Metric {
            read: |e| e.driver_temp_c,
            delta: 2.0,
        },
        Metric {
            read: |e| e.outside_temp_c,
            delta: 2.0,
        },
    ],
    hvac: true,
};

impl Thresholds {
    fn for_event(event: &TelemetryEvent) -> &'static Self {
        if matches!(event.charger_state, Some(ChargerState::Charging))
            || matches!(event.power_state, Some(PowerState::Charging))
        {
            &CHARGING
        } else if matches!(event.power_state, Some(PowerState::Drive | PowerState::Go)) {
            &DRIVING
        } else {
            &PARKED
        }
    }

    fn reached(
        &self,
        gate: &TelemetryPersistenceGate,
        event: &TelemetryEvent,
        next: &TelemetryEvent,
    ) -> bool {
        if self.minimum_seconds.is_some_and(|minimum| {
            gate.last_persisted_at
                .is_some_and(|at| event.ts.signed_duration_since(at).num_seconds() < minimum)
        }) {
            return false;
        }
        let moved = self.movement_m.is_finite()
            && location_moved_m(gate.last_persisted_location, next.latitude, next.longitude)
                .is_some_and(|meters| meters >= self.movement_m);
        moved
            || self.metrics.iter().any(|metric| {
                changed_f64(
                    (metric.read)(event),
                    gate.last_persisted.as_ref().and_then(metric.read),
                    metric.delta,
                )
            })
            || (self.hvac
                && event.hvac_active.is_some()
                && event.hvac_active != gate.last_persisted.as_ref().and_then(|e| e.hvac_active))
    }
}

impl TelemetryPersistenceGate {
    pub(super) fn new(vehicle_id: Uuid) -> Self {
        Self {
            vehicle_id,
            latest: None,
            last_persisted: None,
            last_persisted_at: None,
            last_persisted_location: None,
        }
    }

    pub(super) fn select(&self, event: &TelemetryEvent) -> Selection {
        let rejection = if !event_has_meaningful_payload(event) {
            Some(SuppressionReason::Empty)
        } else if self
            .latest
            .as_ref()
            .is_some_and(|latest| event_is_duplicate_patch(event, latest))
        {
            Some(SuppressionReason::Duplicate)
        } else {
            None
        };
        if let Some(reason) = rejection {
            return Selection {
                decision: PersistenceDecision::Suppress(reason),
                next: None,
            };
        }
        let mut next = self
            .latest
            .clone()
            .unwrap_or_else(|| blank_event(self.vehicle_id, event.ts));
        patch_event(&mut next, event);
        let persist = self
            .latest
            .as_ref()
            .is_none_or(|previous| immediate_change(event, previous))
            || Thresholds::for_event(&next).reached(self, event, &next);
        Selection {
            decision: if persist {
                PersistenceDecision::Persist
            } else {
                PersistenceDecision::Suppress(SuppressionReason::Threshold)
            },
            next: Some(next),
        }
    }

    pub(super) fn advance(&mut self, selection: Selection) {
        if let Some(next) = selection.next {
            if selection.decision == PersistenceDecision::Persist {
                self.last_persisted_at = Some(next.ts);
                self.last_persisted_location = next.latitude.zip(next.longitude);
                self.last_persisted = Some(next.clone());
            }
            self.latest = Some(next);
        }
    }

    #[cfg(test)]
    pub(super) fn decide(&mut self, event: &TelemetryEvent) -> PersistenceDecision {
        let selection = self.select(event);
        let decision = selection.decision;
        self.advance(selection);
        decision
    }
}

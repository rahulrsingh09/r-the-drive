use super::{
    boundary_gaps_are_valid, finite_optional, required_battery_samples,
    requires_interior_soc_support, PhantomCandidateRow, ValidationReason, MAX_ODOMETER_DELTA_MILES,
    MIN_SLEEP_COVERAGE_PCT, RANGE_EFFICIENCY_DRIFT_TOLERANCE, SOC_WINDOW_MISMATCH_TOLERANCE_PCT,
};

pub(super) struct ParkedActivity {
    pub(super) overlaps_trip: bool,
    pub(super) overlaps_charge: bool,
    pub(super) movement_detected: bool,
}

impl ParkedActivity {
    pub(super) fn read(row: &PhantomCandidateRow) -> Self {
        let overlaps_trip = row.trip_overlap_count > 0;
        let overlaps_charge = row.charge_overlap_count > 0;
        let movement_detected = overlaps_trip
            || overlaps_charge
            || row.odometer_delta_mi.unwrap_or(0.0) >= MAX_ODOMETER_DELTA_MILES
            || row.drive_sample_count >= 10;
        Self {
            overlaps_trip,
            overlaps_charge,
            movement_detected,
        }
    }
}

pub(super) struct SocChange {
    pub(super) start: Option<f64>,
    pub(super) end: Option<f64>,
    pub(super) gain_detected: bool,
    pub(super) loss: Option<f64>,
}

impl SocChange {
    pub(super) fn from_anchors(start: Option<f64>, end: Option<f64>) -> Self {
        let start = finite_optional(start);
        let end = finite_optional(end);
        let (gain_detected, loss) = match (start, end) {
            (Some(start), Some(end)) if start < end => (true, None),
            (Some(start), Some(end)) => (false, Some(start - end)),
            _ => (false, None),
        };
        Self {
            start,
            end,
            gain_detected,
            loss,
        }
    }
}

pub(super) struct StateCoverage {
    pub(super) coverage: Option<f64>,
    pub(super) sleep_share: Option<f64>,
}

impl StateCoverage {
    pub(super) fn project(row: &PhantomCandidateRow, seconds: f64) -> Self {
        let sleep = row.sleep_seconds.unwrap_or(0.0).clamp(0.0, seconds);
        let covered = row.covered_state_seconds.unwrap_or(0.0).clamp(0.0, seconds);
        let coverage = (seconds > 0.0).then(|| (covered / seconds).clamp(0.0, 1.0));
        let sleep_share = coverage
            .filter(|coverage| *coverage >= MIN_SLEEP_COVERAGE_PCT && seconds > 0.0)
            .map(|_| (sleep / seconds).clamp(0.0, 1.0));
        Self {
            coverage,
            sleep_share,
        }
    }
}

pub(super) struct DrainEnergy {
    pub(super) kwh: Option<f64>,
    pub(super) average_watts: Option<f64>,
}

impl DrainEnergy {
    pub(super) fn project(loss: Option<f64>, capacity: Option<f64>, hours: f64) -> Self {
        let kwh = match (loss, finite_optional(capacity)) {
            (Some(loss), Some(capacity)) if capacity > 10000.0 => {
                Some((loss / 100.0) * (capacity / 1000.0))
            }
            _ => None,
        };
        let average_watts = per_hour(kwh, hours).map(|rate| rate * 1000.0);
        Self { kwh, average_watts }
    }
}

pub(super) fn per_hour(value: Option<f64>, hours: f64) -> Option<f64> {
    value.filter(|_| hours > 0.0).map(|value| value / hours)
}

pub(super) enum ActivityExclusion {
    Charge,
    Trip,
    SocGain,
    Movement,
}

impl ActivityExclusion {
    pub(super) fn select(
        row: &PhantomCandidateRow,
        movement_detected: bool,
        soc_gain_detected: bool,
    ) -> Option<Self> {
        if row.charge_overlap_count > 0 {
            Some(Self::Charge)
        } else if row.trip_overlap_count > 0 {
            Some(Self::Trip)
        } else if soc_gain_detected {
            Some(Self::SocGain)
        } else if movement_detected {
            Some(Self::Movement)
        } else {
            None
        }
    }

    pub(super) fn reason(self) -> ValidationReason {
        match self {
            Self::Charge => ValidationReason::OverlapsCharge,
            Self::Trip => ValidationReason::OverlapsTrip,
            Self::SocGain => ValidationReason::NetGainDetected,
            Self::Movement => ValidationReason::MovementDetected,
        }
    }
}

pub(super) struct SocSupport<'a> {
    row: &'a PhantomCandidateRow,
    hours: f64,
    loss: Option<f64>,
}

impl<'a> SocSupport<'a> {
    pub(super) fn new(row: &'a PhantomCandidateRow, hours: f64, loss: Option<f64>) -> Self {
        Self { row, hours, loss }
    }

    pub(super) fn anchors_are_reliable(&self) -> bool {
        boundary_gaps_are_valid(
            self.hours,
            self.row.start_sample_gap_minutes,
            self.row.end_sample_gap_minutes,
        )
    }

    pub(super) fn samples_are_sufficient(&self) -> bool {
        if self.row.battery_sample_count < required_battery_samples(self.hours, self.loss) {
            return false;
        }
        !(requires_interior_soc_support(self.hours, self.loss)
            && self.row.interior_battery_sample_count < 1)
    }

    pub(super) fn contradicts_window(&self) -> bool {
        match (
            self.loss,
            finite_optional(self.row.min_soc_in_window),
            finite_optional(self.row.max_soc_in_window),
        ) {
            (Some(loss), Some(min), Some(max)) => {
                let span = (max - min).max(0.0);
                self.row.battery_sample_count >= 3
                    && loss - span > SOC_WINDOW_MISMATCH_TOLERANCE_PCT
            }
            _ => false,
        }
    }
}

pub(super) struct RangeAnchors {
    start_range: f64,
    end_range: f64,
    start_soc: f64,
    end_soc: f64,
    soc_loss: f64,
}

impl RangeAnchors {
    pub(super) fn select(
        row: &PhantomCandidateRow,
        start_soc: Option<f64>,
        end_soc: Option<f64>,
        loss: Option<f64>,
    ) -> Option<Self> {
        if !boundary_gaps_are_valid(
            row.duration_hours,
            row.start_range_gap_minutes,
            row.end_range_gap_minutes,
        ) || row.sample_count < 3
        {
            return None;
        }
        Some(Self {
            start_range: finite_optional(row.range_start_mi)?,
            end_range: finite_optional(row.range_end_mi)?,
            start_soc: start_soc.filter(|soc| *soc > 0.0)?,
            end_soc: end_soc.filter(|soc| *soc > 0.0)?,
            soc_loss: loss?,
        })
    }

    pub(super) fn project(self) -> RangeLoss {
        let miles = (self.start_range - self.end_range).max(0.0);
        let start_efficiency = (self.start_range / self.start_soc) * 100.0;
        let end_efficiency = (self.end_range / self.end_soc) * 100.0;
        let average_efficiency = (start_efficiency + end_efficiency) / 2.0;
        RangeLoss {
            miles,
            start_efficiency,
            end_efficiency,
            average_efficiency,
            soc_loss: self.soc_loss,
        }
    }
}

pub(super) struct RangeLoss {
    miles: f64,
    start_efficiency: f64,
    end_efficiency: f64,
    average_efficiency: f64,
    soc_loss: f64,
}

impl RangeLoss {
    pub(super) fn plausible_miles(self) -> Option<f64> {
        if self.average_efficiency <= 0.0 {
            return None;
        }
        let drift = (self.start_efficiency - self.end_efficiency).abs() / self.average_efficiency;
        if drift > RANGE_EFFICIENCY_DRIFT_TOLERANCE {
            return None;
        }
        let expected_loss = self.average_efficiency * (self.soc_loss / 100.0);
        if self.miles > expected_loss.max(5.0) * 3.0 {
            return None;
        }
        Some(self.miles)
    }
}

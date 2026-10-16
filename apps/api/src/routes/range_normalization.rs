pub const PLAUSIBLE_MAX_MI_PER_KWH: f64 = 3.4;
// The R2 is more efficient than R1 vehicles: Rivian's own R2 range estimates
// imply more than 3.6 mi/kWh, while a kilometre value misread as miles implies
// roughly 5.9 mi/kWh.
pub const R2_PLAUSIBLE_MAX_MI_PER_KWH: f64 = 4.5;

const KM_PER_MILE: f64 = 1.609_344;

pub fn plausible_max_mi_per_kwh(model: Option<&str>) -> f64 {
    match model.map(str::trim) {
        Some(name) if name.eq_ignore_ascii_case("R2") => R2_PLAUSIBLE_MAX_MI_PER_KWH,
        _ => PLAUSIBLE_MAX_MI_PER_KWH,
    }
}

struct BatteryCapacityKwh(f64);

impl BatteryCapacityKwh {
    fn from_reported(value: Option<f64>) -> Option<Self> {
        value
            .filter(|wh| wh.is_finite() && *wh > 0.0)
            .map(|wh| Self(if wh > 1000.0 { wh / 1000.0 } else { wh }))
    }

    fn range_ceiling(self, model: Option<&str>) -> f64 {
        self.0 * plausible_max_mi_per_kwh(model)
    }
}

struct ChargeRange {
    remaining_miles: f64,
    charge_pct: f64,
}

impl ChargeRange {
    fn from_optional(remaining_miles: Option<f64>, charge_pct: Option<f64>) -> Option<Self> {
        Some(Self {
            remaining_miles: remaining_miles.filter(|value| value.is_finite())?,
            charge_pct: charge_pct.filter(|value| value.is_finite())?,
        })
    }

    fn full_charge_miles(&self) -> Option<f64> {
        (self.charge_pct > 0.0).then(|| self.project(self.remaining_miles))
    }

    fn project(&self, miles: f64) -> f64 {
        // Division precedes scaling to preserve threshold rounding and IEEE overflow.
        miles / self.charge_pct * 100.0
    }

    fn assess(self, capacity: Option<BatteryCapacityKwh>, model: Option<&str>) -> RangeAssessment {
        let ceiling = match capacity {
            Some(capacity) if self.charge_pct > 0.0 => capacity.range_ceiling(model),
            _ => return RangeAssessment::Unchecked(self.remaining_miles),
        };
        [self.remaining_miles, self.remaining_miles / KM_PER_MILE]
            .into_iter()
            .find(|candidate| self.project(*candidate) <= ceiling)
            .map(RangeAssessment::Plausible)
            .unwrap_or(RangeAssessment::Implausible(self.remaining_miles))
    }
}

enum RangeAssessment {
    Unchecked(f64),
    Plausible(f64),
    Implausible(f64),
}

enum OutlierPolicy {
    Preserve,
    Reject,
}

impl RangeAssessment {
    fn resolve(self, policy: OutlierPolicy) -> Option<f64> {
        match (self, policy) {
            (Self::Unchecked(miles) | Self::Plausible(miles), _) => Some(miles),
            (Self::Implausible(miles), OutlierPolicy::Preserve) => Some(miles),
            (Self::Implausible(_), OutlierPolicy::Reject) => None,
        }
    }
}

fn normalize_with_policy(
    raw_range_mi: Option<f64>,
    battery_level_pct: Option<f64>,
    battery_capacity_wh: Option<f64>,
    model: Option<&str>,
    policy: OutlierPolicy,
) -> Option<f64> {
    ChargeRange::from_optional(raw_range_mi, battery_level_pct)?
        .assess(
            BatteryCapacityKwh::from_reported(battery_capacity_wh),
            model,
        )
        .resolve(policy)
}

pub fn normalize_remaining_range_miles(
    raw_range_mi: Option<f64>,
    battery_level_pct: Option<f64>,
    battery_capacity_wh: Option<f64>,
    model: Option<&str>,
) -> Option<f64> {
    normalize_with_policy(
        raw_range_mi,
        battery_level_pct,
        battery_capacity_wh,
        model,
        OutlierPolicy::Preserve,
    )
}

pub fn normalize_remaining_range_miles_strict(
    raw_range_mi: Option<f64>,
    battery_level_pct: Option<f64>,
    battery_capacity_wh: Option<f64>,
    model: Option<&str>,
) -> Option<f64> {
    normalize_with_policy(
        raw_range_mi,
        battery_level_pct,
        battery_capacity_wh,
        model,
        OutlierPolicy::Reject,
    )
}

pub fn projected_full_charge_range_miles(
    remaining_range_mi: Option<f64>,
    battery_level_pct: Option<f64>,
) -> Option<f64> {
    ChargeRange::from_optional(remaining_range_mi, battery_level_pct)?.full_charge_miles()
}

#[cfg(test)]
mod analytics_math_contract_tests {
    use super::*;

    type Normalizer = fn(Option<f64>, Option<f64>, Option<f64>, Option<&str>) -> Option<f64>;

    const NORMALIZERS: [Normalizer; 2] = [
        normalize_remaining_range_miles,
        normalize_remaining_range_miles_strict,
    ];

    #[test]
    fn missing_or_nonfinite_range_and_charge_are_absent() {
        for invalid in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ] {
            for normalize in NORMALIZERS {
                assert_eq!(normalize(invalid, Some(50.0), None, None), None);
                assert_eq!(normalize(Some(100.0), invalid, None, None), None);
            }
            assert_eq!(projected_full_charge_range_miles(invalid, Some(50.0)), None);
            assert_eq!(
                projected_full_charge_range_miles(Some(100.0), invalid),
                None
            );
        }
    }

    #[test]
    fn nonpositive_charge_and_unusable_capacity_bypass_plausibility() {
        for normalize in NORMALIZERS {
            for charge in [0.0, -0.0, -1.0, -100.0] {
                assert_eq!(
                    normalize(Some(9_999.0), Some(charge), Some(100.0), None),
                    Some(9_999.0)
                );
                assert_eq!(
                    projected_full_charge_range_miles(Some(9_999.0), Some(charge)),
                    None
                );
            }
            for capacity in [
                None,
                Some(0.0),
                Some(-0.0),
                Some(-1.0),
                Some(f64::NAN),
                Some(f64::INFINITY),
                Some(f64::NEG_INFINITY),
            ] {
                assert_eq!(
                    normalize(Some(9_999.0), Some(50.0), capacity, None),
                    Some(9_999.0)
                );
            }
        }
    }

    #[test]
    fn capacity_unit_threshold_is_strictly_greater_than_one_thousand() {
        let threshold = 1000.0_f64;
        for normalize in NORMALIZERS {
            for capacity in [threshold.next_down(), threshold] {
                assert_eq!(
                    normalize(Some(5.0), Some(100.0), Some(capacity), None),
                    Some(5.0)
                );
            }
            assert_eq!(
                normalize(Some(5.0), Some(100.0), Some(threshold.next_up()), None),
                Some(5.0 / 1.609_344)
            );
        }
        for capacity_kwh in [50.0, 91.585, 135.0, 200.0] {
            for normalize in NORMALIZERS {
                assert_eq!(
                    normalize(Some(380.0), Some(71.0), Some(capacity_kwh), None),
                    normalize(Some(380.0), Some(71.0), Some(capacity_kwh * 1000.0), None)
                );
            }
        }
    }

    #[test]
    fn only_trimmed_case_insensitive_r2_uses_the_larger_ceiling() {
        for model in [Some("R2"), Some("r2"), Some(" \tR2\n")] {
            assert_eq!(plausible_max_mi_per_kwh(model), 4.5);
        }
        for model in [
            None,
            Some(""),
            Some("R1T"),
            Some("R1S"),
            Some("R2S"),
            Some("R2 Performance"),
        ] {
            assert_eq!(plausible_max_mi_per_kwh(model), 3.4);
        }
    }

    #[test]
    fn candidate_thresholds_are_inclusive_and_miles_take_priority() {
        for model in [None, Some("R2")] {
            let capacity = 100.0;
            let limit = capacity * plausible_max_mi_per_kwh(model);
            for raw in [limit.next_down(), limit] {
                assert_eq!(
                    normalize_remaining_range_miles_strict(
                        Some(raw),
                        Some(100.0),
                        Some(capacity),
                        model
                    ),
                    Some(raw)
                );
            }
            let raw = limit.next_up();
            assert_eq!(
                normalize_remaining_range_miles_strict(
                    Some(raw),
                    Some(100.0),
                    Some(capacity),
                    model
                ),
                Some(raw / 1.609_344)
            );
            let km_limit = limit * 1.609_344;
            for raw in [km_limit.next_down(), km_limit] {
                assert_eq!(
                    normalize_remaining_range_miles_strict(
                        Some(raw),
                        Some(100.0),
                        Some(capacity),
                        model
                    ),
                    Some(raw / 1.609_344)
                );
            }
            let outside = km_limit.next_up().next_up();
            assert_eq!(
                normalize_remaining_range_miles_strict(
                    Some(outside),
                    Some(100.0),
                    Some(capacity),
                    model
                ),
                None
            );
            assert_eq!(
                normalize_remaining_range_miles(Some(outside), Some(100.0), Some(capacity), model),
                Some(outside)
            );
        }
    }

    #[test]
    fn negative_ranges_signed_zero_and_overflow_keep_ieee_semantics() {
        for normalize in NORMALIZERS {
            for raw in [-100.0, -0.0, 0.0] {
                let value = normalize(Some(raw), Some(50.0), Some(100.0), None).unwrap();
                assert_eq!(value.to_bits(), raw.to_bits());
            }
            assert_eq!(
                normalize(Some(100.0), Some(200.0), Some(100.0), None),
                Some(100.0)
            );
        }
        assert_eq!(
            projected_full_charge_range_miles(Some(-100.0), Some(50.0)),
            Some(-200.0)
        );
        assert_eq!(
            projected_full_charge_range_miles(Some(-0.0), Some(50.0))
                .unwrap()
                .to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(
            projected_full_charge_range_miles(Some(f64::MAX), Some(0.5)),
            Some(f64::INFINITY)
        );
        assert_eq!(
            normalize_remaining_range_miles_strict(
                Some(f64::MAX),
                Some(f64::MIN_POSITIVE),
                Some(100.0),
                None
            ),
            None
        );
        assert_eq!(
            normalize_remaining_range_miles(
                Some(f64::MAX),
                Some(f64::MIN_POSITIVE),
                Some(100.0),
                None
            ),
            Some(f64::MAX)
        );
    }

    #[test]
    fn normalization_and_projection_agree_across_synthetic_cohorts() {
        for model in [None, Some("R2")] {
            for capacity in [60.0, 100.0, 135.0, 180.0] {
                for charge in [1.0, 20.0, 50.0, 100.0, 125.0] {
                    for full in [0.0, 100.0, 200.0, 300.0, 400.0, 600.0, 1000.0] {
                        let raw = full * charge / 100.0;
                        let loose = normalize_remaining_range_miles(
                            Some(raw),
                            Some(charge),
                            Some(capacity),
                            model,
                        )
                        .unwrap();
                        let strict = normalize_remaining_range_miles_strict(
                            Some(raw),
                            Some(charge),
                            Some(capacity),
                            model,
                        );
                        assert!(loose == raw || loose == raw / 1.609_344);
                        if let Some(miles) = strict {
                            assert_eq!(loose, miles);
                            assert!(
                                projected_full_charge_range_miles(Some(miles), Some(charge))
                                    .unwrap()
                                    <= capacity * plausible_max_mi_per_kwh(model)
                            );
                            assert_eq!(
                                normalize_remaining_range_miles_strict(
                                    Some(miles),
                                    Some(charge),
                                    Some(capacity),
                                    model
                                ),
                                Some(miles)
                            );
                        } else {
                            assert_eq!(loose, raw);
                            assert!(
                                projected_full_charge_range_miles(
                                    Some(raw / 1.609_344),
                                    Some(charge)
                                )
                                .unwrap()
                                    > capacity * plausible_max_mi_per_kwh(model)
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_remaining_range_miles, normalize_remaining_range_miles_strict,
        projected_full_charge_range_miles,
    };

    #[test]
    fn keeps_plausible_miles() {
        let value = normalize_remaining_range_miles(Some(227.0), Some(71.0), Some(135_000.0), None);
        assert_eq!(value, Some(227.0));
    }

    #[test]
    fn converts_km_to_miles_when_plausible_after_conversion() {
        let value = normalize_remaining_range_miles(Some(380.0), Some(71.0), Some(135_000.0), None);
        assert_eq!(
            value.map(|miles| (miles * 10.0).round() / 10.0),
            Some(236.1)
        );
    }

    #[test]
    fn strict_mode_drops_impossible_readings() {
        let value =
            normalize_remaining_range_miles_strict(Some(520.0), Some(65.0), Some(135_000.0), None);
        assert_eq!(value, None);
    }

    #[test]
    fn keeps_efficient_r2_miles() {
        let value =
            normalize_remaining_range_miles(Some(208.16), Some(62.1), Some(91_585.0), Some("R2"));
        assert_eq!(value, Some(208.16));
    }

    #[test]
    fn converts_r2_km_value_to_miles() {
        let value =
            normalize_remaining_range_miles(Some(335.0), Some(62.1), Some(91_585.0), Some("R2"));
        assert_eq!(
            value.map(|miles| (miles * 10.0).round() / 10.0),
            Some(208.2)
        );
    }

    #[test]
    fn projects_full_charge_range_from_soc() {
        let value = projected_full_charge_range_miles(Some(210.0), Some(70.0));
        assert_eq!(value, Some(300.0));
    }
}

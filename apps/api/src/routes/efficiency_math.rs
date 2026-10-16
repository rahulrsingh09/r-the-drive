struct WeightedEfficiencyTotals {
    distance_miles: f64,
    distance_weighted_wh_per_mile: f64,
}

impl WeightedEfficiencyTotals {
    fn from_optional(distance_miles: Option<f64>, weighted: Option<f64>) -> Option<Self> {
        distance_miles
            .zip(weighted)
            .and_then(|(distance, weighted)| {
                (distance.is_finite() && distance > 0.0 && weighted.is_finite()).then_some(Self {
                    distance_miles: distance,
                    distance_weighted_wh_per_mile: weighted,
                })
            })
    }

    fn wh_per_mile(self) -> f64 {
        self.distance_weighted_wh_per_mile / self.distance_miles
    }
}

pub(crate) fn weighted_average_from_totals(
    total_distance_miles: Option<f64>,
    weighted_efficiency_wh_mi: Option<f64>,
) -> Option<f64> {
    WeightedEfficiencyTotals::from_optional(total_distance_miles, weighted_efficiency_wh_mi)
        .map(WeightedEfficiencyTotals::wh_per_mile)
}

#[cfg(test)]
mod analytics_math_contract_tests {
    use super::weighted_average_from_totals as average;

    #[test]
    fn missing_nonfinite_and_nonpositive_denominators_are_absent() {
        let invalid = [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ];
        for value in invalid {
            assert_eq!(average(value, Some(1.0)), None);
            assert_eq!(average(Some(1.0), value), None);
        }
        for distance in [0.0, -0.0, -1.0, -f64::MIN_POSITIVE] {
            assert_eq!(average(Some(distance), Some(1.0)), None);
        }
    }

    #[test]
    fn finite_signed_numerators_and_ieee_results_are_preserved() {
        assert_eq!(average(Some(2.0), Some(-500.0)), Some(-250.0));
        assert_eq!(average(Some(2.0), Some(0.0)), Some(0.0));
        assert_eq!(
            average(Some(2.0), Some(-0.0)).unwrap().to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(
            average(Some(f64::MIN_POSITIVE), Some(f64::MAX)),
            Some(f64::INFINITY)
        );
        assert_eq!(average(Some(f64::MAX), Some(f64::MIN_POSITIVE)), Some(0.0));
    }

    #[test]
    fn cohort_aggregation_is_invariant_to_partition_and_common_scale() {
        for short_distance in 1..20 {
            for long_distance in 20..40 {
                let a = f64::from(short_distance);
                let b = f64::from(long_distance);
                let expected = (a * -125.0 + b * 425.0) / (a + b);
                for scale in [0.5, 1.0, 2.0, 8.0] {
                    assert_eq!(
                        average(
                            Some((a + b) * scale),
                            Some((a * -125.0 + b * 425.0) * scale)
                        ),
                        Some(expected)
                    );
                }
                assert!((-125.0..=425.0).contains(&expected));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::weighted_average_from_totals;

    #[test]
    fn uses_distance_weighting_instead_of_trip_count_weighting() {
        let short_trip = 1.0_f64 * 1000.0_f64;
        let long_trip = 100.0_f64 * 333.0_f64;
        let average = weighted_average_from_totals(Some(101.0), Some(short_trip + long_trip));

        assert!(average.is_some());
        let value = average.unwrap();
        assert!(
            (value - 339.6039603960396).abs() < 1e-9,
            "unexpected weighted average: {value}"
        );
    }

    #[test]
    fn ignores_missing_or_zero_distance_totals() {
        assert_eq!(weighted_average_from_totals(Some(0.0), Some(123.0)), None);
        assert_eq!(weighted_average_from_totals(None, Some(123.0)), None);
        assert_eq!(weighted_average_from_totals(Some(12.0), None), None);
    }
}

use std::collections::HashMap;

use super::*;

pub(super) struct SourcePolicy {
    id: &'static str,
    fields: &'static [&'static str],
    pub(super) filters: &'static [&'static str],
    pub(super) marks: &'static [&'static str],
}

pub(super) fn lookup(id: &str) -> Option<&'static SourcePolicy> {
    SOURCE_POLICIES.iter().find(|policy| policy.id == id)
}

pub(super) struct Bindings<'a>(HashMap<&'a str, &'a ChartSourceBinding>);

impl<'a> Bindings<'a> {
    pub(super) fn new(sources: &'a [ChartSourceBinding]) -> Self {
        let mut index = HashMap::with_capacity(sources.len());
        for source in sources {
            // Duplicate IDs are rejected, but later diagnostics still use the first binding.
            index.entry(source.id.as_str()).or_insert(source);
        }
        Self(index)
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn get(&self, id: &str) -> Option<&'a ChartSourceBinding> {
        self.0.get(id).copied()
    }

    pub(super) fn validate(
        &self,
        reference: &ChartFieldRef,
        path: &str,
        errors: &mut Vec<ChartValidationError>,
    ) {
        let binding = self.get(&reference.source_binding_id);
        if binding.is_none() || !id(&reference.field) {
            errors.push(error(
                path,
                "Unknown source binding or invalid field identifier",
            ));
        }
        let Some(binding) = binding else { return };
        let available = if binding.source_id == "metrics.series" {
            reference.field == "timestamp"
                || binding
                    .params
                    .get("metric")
                    .and_then(Value::as_str)
                    .is_some_and(|metric| metric == reference.field && is_series_metric(metric))
        } else {
            lookup(&binding.source_id)
                .is_some_and(|policy| policy.fields.contains(&reference.field.as_str()))
        };
        if !available {
            errors.push(error(path, "Field is not available from the selected source or does not match its metric parameter"));
        }
    }
}

const SOURCE_POLICIES: &[SourcePolicy] = &[
    SourcePolicy {
        id: "metrics.series",
        fields: &["timestamp"],
        filters: &["metric"],
        marks: &["line", "area", "step", "bar", "scatter", "histogram"],
    },
    SourcePolicy {
        id: "battery.mileage",
        fields: &[
            "timestamp",
            "odometer_miles",
            "usable_kwh",
            "range_mi",
            "projected_max_range_mi",
            "degradation_pct",
        ],
        filters: &[],
        marks: &["line", "area", "step", "scatter"],
    },
    SourcePolicy {
        id: "battery.degradation",
        fields: &["timestamp", "capacity_pct", "degradation_pct"],
        filters: &[],
        marks: &["line", "area", "step", "scatter"],
    },
    SourcePolicy {
        id: "battery.idle-drain",
        fields: &[
            "day_start",
            "soc_lost_pct",
            "parked_hours",
            "drain_rate_pct_per_hour",
        ],
        filters: &[],
        marks: &["line", "area", "bar", "scatter"],
    },
    SourcePolicy {
        id: "charging.sessions",
        fields: &[
            "day_start",
            "total_energy_kwh",
            "session_count",
            "ac_energy_kwh",
            "dc_energy_kwh",
        ],
        filters: &[],
        marks: &["bar", "line", "area", "scatter"],
    },
    SourcePolicy {
        id: "charging.charge-curve",
        fields: &[
            "timestamp",
            "elapsed_minutes",
            "soc_pct",
            "power_kw",
            "energy_kwh",
        ],
        filters: &[],
        marks: &["line", "area", "step", "scatter"],
    },
    SourcePolicy {
        id: "charging.curve-analysis",
        fields: &["soc_pct", "power_kw", "trend_kw"],
        filters: &[],
        marks: &["scatter", "line", "area"],
    },
    SourcePolicy {
        id: "efficiency.trend",
        fields: &["timestamp", "trip_efficiency_wh_mi", "rolling_24h_wh_mi"],
        filters: &["trip_tag_id"],
        marks: &["line", "area", "step", "scatter"],
    },
    SourcePolicy {
        id: "efficiency.temperature",
        fields: &[
            "temp_c_low",
            "temp_c_high",
            "avg_efficiency_wh_mi",
            "total_miles",
            "avg_speed_mph",
        ],
        filters: &["trip_tag_id"],
        marks: &["scatter", "bar", "line"],
    },
    SourcePolicy {
        id: "efficiency.drive-mode",
        fields: &["drive_mode", "avg_efficiency_wh_mi", "trip_count"],
        filters: &["drive_mode"],
        marks: &["bar"],
    },
    SourcePolicy {
        id: "efficiency.trip-tags",
        fields: &[
            "tag_name",
            "tag_id",
            "avg_efficiency_wh_mi",
            "trip_count",
            "total_miles",
            "coverage",
        ],
        filters: &["tag_id", "trip_tag_id"],
        marks: &["bar"],
    },
    SourcePolicy {
        id: "trips.tire-pressure-timeline",
        fields: &[
            "timestamp",
            "front_left_psi",
            "front_right_psi",
            "rear_left_psi",
            "rear_right_psi",
        ],
        filters: &["tag_id", "trip_tag_id"],
        marks: &["line", "area", "step", "scatter"],
    },
];

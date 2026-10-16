use super::*;

pub(super) const METRICS: &[MetricDef] = &[
    MetricDef {
        id: "total_miles",
        label: "Total Miles",
        unit: Some("mi"),
        kind: "distance",
        source_label: "summary",
        supports_series: true,
        default_aggregation: "latest",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "trip_miles",
        label: "Trip Miles",
        unit: Some("mi"),
        kind: "distance",
        source_label: "trips",
        supports_series: true,
        default_aggregation: "sum",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "total_trips",
        label: "Total Trips",
        unit: None,
        kind: "number",
        source_label: "summary",
        supports_series: true,
        default_aggregation: "sum",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "energy_charged",
        label: "Energy Charged",
        unit: Some("kWh"),
        kind: "energy",
        source_label: "charging",
        supports_series: true,
        default_aggregation: "sum",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "charging_sessions",
        label: "Charging Sessions",
        unit: None,
        kind: "number",
        source_label: "charging",
        supports_series: true,
        default_aggregation: "sum",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "total_cost",
        label: "Total Cost",
        unit: Some("USD"),
        kind: "currency",
        source_label: "charging",
        supports_series: true,
        default_aggregation: "sum",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "avg_session_energy",
        label: "Avg Session Energy",
        unit: Some("kWh"),
        kind: "energy",
        source_label: "charging",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "avg_efficiency",
        label: "Avg Efficiency",
        unit: Some("Wh/mi"),
        kind: "number",
        source_label: "trips",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "avg_gross_efficiency",
        label: "Avg Gross Efficiency",
        unit: Some("Wh/mi"),
        kind: "number",
        source_label: "trips",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "avg_outside_temp_c",
        label: "Avg Outside (estimated)",
        unit: Some("C"),
        kind: "temperature",
        source_label: "trips",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "avg_trip_duration",
        label: "Avg Trip Duration",
        unit: Some("min"),
        kind: "number",
        source_label: "trips",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Summary,
    },
    MetricDef {
        id: "battery_level",
        label: "Battery Level",
        unit: Some("%"),
        kind: "percent",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("battery_level"),
    },
    MetricDef {
        id: "range_miles",
        label: "Estimated Range",
        unit: Some("mi"),
        kind: "distance",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("distance_to_empty_mi"),
    },
    MetricDef {
        id: "odometer_miles",
        label: "Odometer",
        unit: Some("mi"),
        kind: "distance",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "max",
        source: MetricSource::Telemetry("odometer_miles"),
    },
    MetricDef {
        id: "outside_temp_c",
        label: "Outside Temp",
        unit: Some("C"),
        kind: "temperature",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("outside_temp_c"),
    },
    MetricDef {
        id: "speed_mph",
        label: "Speed",
        unit: Some("mph"),
        kind: "speed",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("speed_mph"),
    },
    MetricDef {
        id: "power_kw",
        label: "Power",
        unit: Some("kW"),
        kind: "number",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("power_kw"),
    },
    MetricDef {
        id: "tire_fl_psi",
        label: "Front Left Tire",
        unit: Some("psi"),
        kind: "pressure",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("tire_fl_psi"),
    },
    MetricDef {
        id: "tire_fr_psi",
        label: "Front Right Tire",
        unit: Some("psi"),
        kind: "pressure",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("tire_fr_psi"),
    },
    MetricDef {
        id: "tire_rl_psi",
        label: "Rear Left Tire",
        unit: Some("psi"),
        kind: "pressure",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("tire_rl_psi"),
    },
    MetricDef {
        id: "tire_rr_psi",
        label: "Rear Right Tire",
        unit: Some("psi"),
        kind: "pressure",
        source_label: "telemetry",
        supports_series: true,
        default_aggregation: "avg",
        source: MetricSource::Telemetry("tire_rr_psi"),
    },
];

fn index() -> &'static std::collections::HashMap<&'static str, &'static MetricDef> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static MetricDef>> =
        std::sync::OnceLock::new();
    INDEX.get_or_init(|| METRICS.iter().map(|metric| (metric.id, metric)).collect())
}

pub(super) fn lookup(id: &str) -> Result<&'static MetricDef, AppError> {
    if id.len() > 128 || id.chars().any(char::is_control) {
        return Err(AppError::Validation("metric identifier is invalid".into()));
    }
    index()
        .get(id)
        .copied()
        .ok_or_else(|| AppError::Validation(format!("unknown metric: {id}")))
}

pub(super) fn entries() -> Vec<MetricCatalogEntry> {
    METRICS
        .iter()
        .map(|metric| MetricCatalogEntry {
            id: metric.id,
            label: metric.label,
            unit: metric.unit,
            kind: metric.kind,
            source: metric.source_label,
            supports_series: metric.supports_series,
            default_aggregation: metric.default_aggregation,
        })
        .collect()
}

pub(super) struct Selection {
    pub(super) metric: &'static MetricDef,
    pub(super) latest: bool,
    pub(super) series: bool,
}

pub(super) fn select(items: Vec<MetricBatchMetricRequest>) -> Result<Vec<Selection>, AppError> {
    if items.is_empty() {
        return Err(AppError::Validation(
            "at least one metric is required".into(),
        ));
    }
    if items.len() > 32 {
        return Err(AppError::Validation(
            "at most 32 metrics may be requested".into(),
        ));
    }
    let mut positions = std::collections::HashMap::new();
    let mut selected: Vec<Selection> = Vec::new();
    for item in items {
        let metric = lookup(&item.metric)?;
        let position = *positions.entry(metric.id).or_insert_with(|| {
            selected.push(Selection {
                metric,
                latest: false,
                series: false,
            });
            selected.len() - 1
        });
        selected[position].latest |= item.include_latest;
        selected[position].series |= item.include_series;
    }
    Ok(selected)
}

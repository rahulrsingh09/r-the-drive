use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;

use crate::routes::metrics::is_series_metric;

#[path = "chart_validation/bundled.rs"]
mod bundled;
#[cfg(test)]
#[path = "chart_validation/contract_tests.rs"]
mod contract_tests;
#[path = "chart_validation/source_policy.rs"]
mod source_policy;
#[path = "chart_validation/stages.rs"]
mod stages;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartDefinitionV1 {
    pub schema_version: u8,
    pub placements: Vec<ChartPlacement>,
    pub timeframe: ChartTimeframePolicy,
    pub sources: Vec<ChartSourceBinding>,
    pub x: ChartFieldEncoding,
    pub series: Vec<ChartSeriesDefinition>,
    pub axes: ChartAxesDefinition,
    pub display: ChartDisplayDefaults,
    pub interaction: ChartInteractionDefaults,
    pub annotations: Option<Vec<ChartAnnotationDefinition>>,
    pub renderer_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartPlacement {
    pub dashboard_slug: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum ChartTimeframePolicy {
    #[serde(rename = "dashboard")]
    Dashboard,
    #[serde(rename = "relative")]
    Relative { preset: String },
    #[serde(rename = "lifetime")]
    Lifetime,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartSourceBinding {
    pub id: String,
    pub source_id: String,
    #[serde(default)]
    pub params: serde_json::Map<String, Value>,
    #[serde(default)]
    pub filters: Vec<ChartSourceFilter>,
    pub inherit: ChartInheritance,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartInheritance {
    pub vehicle: bool,
    pub timeframe: bool,
    pub trip_tags: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartSourceFilter {
    pub field: String,
    pub operator: String,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartFieldEncoding {
    pub field: ChartFieldRef,
    pub kind: String,
    pub label: Option<String>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartFieldRef {
    pub source_binding_id: String,
    pub field: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartSeriesDefinition {
    pub id: String,
    pub label: String,
    pub y: ChartFieldRef,
    pub x: Option<ChartFieldRef>,
    pub mark: String,
    pub fill: Option<bool>,
    pub y_axis: String,
    pub color: ChartColorDefinition,
    pub stroke_width: Option<f64>,
    pub point_size: Option<f64>,
    pub stack_id: Option<String>,
    pub connect_gaps: Option<bool>,
    pub visible_in_legend: Option<bool>,
    #[serde(default)]
    pub transforms: Vec<ChartTransformDefinition>,
    pub value_format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum ChartColorDefinition {
    #[serde(rename = "token")]
    Token { token: String },
    #[serde(rename = "custom")]
    Custom { light: String, dark: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ChartTransformDefinition {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "scale")]
    Scale { factor: f64 },
    #[serde(rename = "offset")]
    Offset { amount: f64 },
    #[serde(rename = "delta")]
    Delta,
    #[serde(rename = "cumulative")]
    Cumulative,
    #[serde(rename = "rolling_average")]
    RollingAverage { window: u32 },
    #[serde(rename = "rolling_median")]
    RollingMedian { window: u32 },
    #[serde(rename = "fixed_time_bin")]
    FixedTimeBin { milliseconds: u64 },
    #[serde(rename = "histogram_bin")]
    HistogramBin { size: f64 },
    #[serde(rename = "expression")]
    Expression { formula: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartAxesDefinition {
    pub x: ChartAxisDefinition,
    pub y: ChartAxisDefinition,
    pub y2: Option<ChartAxisDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartAxisDefinition {
    pub label: Option<String>,
    pub unit: Option<String>,
    pub scale: String,
    pub domain: ChartAxisDomain,
    pub tick_format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum ChartAxisDomain {
    #[serde(rename = "auto")]
    Auto {
        #[serde(rename = "includeZero")]
        include_zero: Option<bool>,
        padding: Option<f64>,
    },
    #[serde(rename = "fixed")]
    Fixed { min: f64, max: f64 },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartDisplayDefaults {
    pub legend: String,
    pub grid: bool,
    pub tooltip: bool,
    pub time_filter: String,
    pub curve_smoothness: String,
    pub data_smoothing: Option<ChartSmoothing>,
    pub show_points: Option<bool>,
    pub empty_title: Option<String>,
    pub empty_description: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartSmoothing {
    pub kind: String,
    pub window: u32,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartInteractionDefaults {
    pub pan_zoom: bool,
    pub touch_explore: bool,
    pub connect_gaps: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartAnnotationDefinition {
    pub kind: String,
    pub value: f64,
    pub label: Option<String>,
    pub color: ChartColorDefinition,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChartValidationError {
    pub path: String,
    pub message: String,
}

const FORBIDDEN_KEYS: &[&str] = &[
    "url",
    "remoteurl",
    "sql",
    "javascript",
    "script",
    "templatehtml",
    "css",
];
const BUNDLED_RENDERER_CAPABILITIES: &str = include_str!("../../charts/bundled-renderers.json");
const BUNDLED_CHART_DEFAULTS: &str = include_str!("../../charts/defaults.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledRendererCapability {
    slug: String,
    renderer: String,
    editing: String,
    x: BundledRendererX,
    curves: Vec<BundledRendererCurve>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledRendererX {
    source_id: String,
    field: String,
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledRendererCurve {
    source_id: String,
    field: String,
}

fn id(value: &str) -> bool {
    match value.as_bytes() {
        [] => false,
        bytes if bytes.len() > 80 => false,
        bytes => !bytes
            .iter()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-')),
    }
}
fn finite(value: f64) -> bool {
    value.is_finite()
}

fn valid_chart_color_token(token: &str) -> bool {
    const NAMED: [&str; 12] = [
        "accent", "emerald", "amber", "sky", "violet", "rose", "teal", "indigo", "success",
        "warning", "danger", "muted",
    ];
    if NAMED.contains(&token) {
        return true;
    }
    match token.strip_prefix("series-") {
        Some(slot) if token.len() == 9 => slot.parse::<u8>().is_ok_and(|n| (1..=16).contains(&n)),
        _ => false,
    }
}

fn valid_custom_color(color: &str) -> bool {
    let value = color.trim();
    if let Some(digits) = value.strip_prefix('#') {
        return matches!(digits.len(), 3 | 4 | 6 | 8)
            && digits.bytes().all(|digit| digit.is_ascii_hexdigit());
    }
    if value.len() > 86 || !value.ends_with(')') {
        return false;
    }
    let Some((function, _)) = value.split_once('(') else {
        return false;
    };
    ["rgb", "rgba", "hsl", "hsla"]
        .iter()
        .any(|name| function.eq_ignore_ascii_case(name))
}

pub fn parse_and_validate(value: &Value) -> Result<ChartDefinitionV1, Vec<ChartValidationError>> {
    let mut errors = Vec::new();
    find_forbidden(value, "config", &mut errors);
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        errors.push(error(
            "config.schemaVersion",
            "Only schemaVersion 1 is supported",
        ));
    }
    let definition = match serde_json::from_value::<ChartDefinitionV1>(value.clone()) {
        Ok(definition) => definition,
        Err(failure) => {
            errors.push(error("config", &failure.to_string()));
            return Err(errors);
        }
    };
    stages::validate(&definition, &mut errors);
    if errors.is_empty() {
        Ok(definition)
    } else {
        Err(errors)
    }
}

pub fn parse_and_validate_for_slug(
    slug: &str,
    value: &Value,
) -> Result<ChartDefinitionV1, Vec<ChartValidationError>> {
    let definition = parse_and_validate(value)?;
    let errors = bundled::validate(slug, value, &definition);
    if errors.is_empty() {
        Ok(definition)
    } else {
        Err(errors)
    }
}

fn append_failures<'a>(
    errors: &mut Vec<ChartValidationError>,
    checks: impl IntoIterator<Item = (bool, &'a str, &'a str)>,
) {
    errors.extend(
        checks
            .into_iter()
            .filter(|(failed, _, _)| *failed)
            .map(|(_, path, message)| error(path, message)),
    );
}

fn error(path: &str, message: &str) -> ChartValidationError {
    ChartValidationError {
        path: path.into(),
        message: message.into(),
    }
}
fn finite_options(value: Option<f64>, min: f64, max: f64) -> bool {
    value
        .map(|v| finite(v) && v >= min && v <= max)
        .unwrap_or(true)
}
fn valid_filter_value(value: &Value) -> bool {
    fn scalar(value: &Value) -> bool {
        match value {
            Value::String(text) => text.len() <= 160,
            Value::Number(number) => number.as_f64().is_some_and(f64::is_finite),
            _ => false,
        }
    }
    if let Value::Array(items) = value {
        items.len() <= 100 && items.iter().all(scalar)
    } else {
        matches!(value, Value::Null | Value::Bool(_)) || scalar(value)
    }
}
fn validate_transform(
    transform: &ChartTransformDefinition,
    errors: &mut Vec<ChartValidationError>,
) {
    let (valid, path, message) = match transform {
        ChartTransformDefinition::Scale { factor }
        | ChartTransformDefinition::HistogramBin { size: factor } => (
            finite(*factor) && *factor > 0.0,
            "config.series.transforms",
            "Transform numeric values must be finite and positive",
        ),
        ChartTransformDefinition::Offset { amount } => (
            finite(*amount),
            "config.series.transforms",
            "Transform numeric values must be finite",
        ),
        ChartTransformDefinition::RollingAverage { window }
        | ChartTransformDefinition::RollingMedian { window } => (
            (2..=1000).contains(window),
            "config.series.transforms",
            "Transform windows must be between 2 and 1000",
        ),
        ChartTransformDefinition::FixedTimeBin { milliseconds } => (
            (1..=31_536_000_000).contains(milliseconds),
            "config.series.transforms",
            "Transform bin is out of bounds",
        ),
        ChartTransformDefinition::Expression { formula } => (
            !formula.trim().is_empty() && formula.len() <= 512,
            "config.series.transforms.formula",
            "Expression is empty or too long",
        ),
        _ => return,
    };
    if !valid {
        errors.push(error(path, message));
    }
}
fn validate_axis(axis: &ChartAxisDefinition, errors: &mut Vec<ChartValidationError>) {
    let valid_domain = match axis.domain {
        ChartAxisDomain::Auto { .. } => true,
        ChartAxisDomain::Fixed { min, max } => {
            finite(min)
                && finite(max)
                && min < max
                && (axis.scale != "log" || (min > 0.0 && max > 0.0))
        }
    };
    append_failures(
        errors,
        [
            (
                !["linear", "log"].contains(&axis.scale.as_str()),
                "config.axes",
                "Unsupported axis scale",
            ),
            (
                !valid_domain,
                "config.axes.domain",
                "Invalid fixed axis range",
            ),
        ],
    );
}
fn find_forbidden(value: &Value, path: &str, errors: &mut Vec<ChartValidationError>) {
    let mut pending = vec![(value, path.to_owned(), false)];
    while let Some((value, path, forbidden)) = pending.pop() {
        if forbidden {
            errors.push(error(
                &path,
                "Executable, remote, SQL, HTML, or CSS fields are not allowed",
            ));
        }
        match value {
            Value::Array(items) => {
                pending.extend(
                    items
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(index, child)| (child, format!("{path}.{index}"), false)),
                );
            }
            Value::Object(object) => {
                for (key, child) in object.iter().rev() {
                    let forbidden = FORBIDDEN_KEYS
                        .contains(&key.replace('_', "").to_ascii_lowercase().as_str());
                    pending.push((child, format!("{path}.{key}"), forbidden));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> Value {
        serde_json::json!({"schemaVersion":1,"placements":[{"dashboardSlug":"overview"}],"timeframe":{"mode":"dashboard"},"sources":[{"id":"main","sourceId":"metrics.series","params":{"metric":"battery_level"},"filters":[],"inherit":{"vehicle":true,"timeframe":true}}],"x":{"field":{"sourceBindingId":"main","field":"timestamp"},"kind":"time"},"series":[{"id":"soc","label":"SoC","y":{"sourceBindingId":"main","field":"battery_level"},"mark":"line","yAxis":"y","color":{"mode":"token","token":"accent"},"transforms":[]}],"axes":{"x":{"scale":"linear","domain":{"mode":"auto"}},"y":{"scale":"linear","domain":{"mode":"auto"}}},"display":{"legend":"auto","grid":true,"tooltip":true,"timeFilter":"raw","curveSmoothness":"straight"},"interaction":{"panZoom":true,"touchExplore":true,"connectGaps":true}})
    }
    #[test]
    fn accepts_valid_definition() {
        assert!(parse_and_validate(&valid()).is_ok());
    }
    #[test]
    fn accepts_optional_series_fill_metadata() {
        let mut value = valid();
        value["series"][0]["fill"] = true.into();
        let parsed = parse_and_validate(&value).expect("fill is backward-compatible metadata");
        assert_eq!(parsed.series[0].fill, Some(true));
    }
    #[test]
    fn rejects_unknown_schema_and_forbidden_keys() {
        let mut value = valid();
        value["schemaVersion"] = 2.into();
        value["url"] = "https://example.invalid".into();
        let errors = parse_and_validate(&value).expect_err("invalid");
        assert!(errors.iter().any(|e| e.path.contains("schemaVersion")));
        assert!(errors.iter().any(|e| e.path.contains("url")));
    }
    #[test]
    fn rejects_unknown_source_and_bad_axis() {
        let mut value = valid();
        value["sources"][0]["sourceId"] = "unknown.source".into();
        value["axes"]["y"]["domain"] = serde_json::json!({"mode":"fixed","min":2,"max":1});
        assert!(parse_and_validate(&value).is_err());
    }
    #[test]
    fn validates_legacy_and_series_color_tokens_and_custom_literals() {
        let mut slot = valid();
        slot["series"][0]["color"] = serde_json::json!({"mode":"token","token":"series-16"});
        assert!(parse_and_validate(&slot).is_ok());

        let mut unknown = valid();
        unknown["series"][0]["color"] = serde_json::json!({"mode":"token","token":"series-17"});
        assert!(parse_and_validate(&unknown).is_err());

        let mut legacy_custom = valid();
        legacy_custom["series"][0]["color"] =
            serde_json::json!({"mode":"custom","light":"rgb(1, 2, 3)","dark":"hsl(10, 20%, 30%)"});
        assert!(parse_and_validate(&legacy_custom).is_ok());

        let mut unsafe_custom = valid();
        unsafe_custom["series"][0]["color"] = serde_json::json!({"mode":"custom","light":"url(https://example.test)","dark":"#123456"});
        assert!(parse_and_validate(&unsafe_custom).is_err());
    }
    #[test]
    fn rejects_unknown_or_mismatched_metric_series_bindings() {
        let mut unknown = valid();
        unknown["sources"][0]["params"]["metric"] = "not_a_metric".into();
        let errors = parse_and_validate(&unknown).expect_err("unknown metric");
        assert!(errors
            .iter()
            .any(|error| error.path.contains("params.metric")));

        let mut mismatched = valid();
        mismatched["series"][0]["y"]["field"] = "range_miles".into();
        let errors = parse_and_validate(&mismatched).expect_err("mismatched field");
        assert!(errors.iter().any(|error| error.path.contains("series.0.y")));
    }

    #[test]
    fn bundled_renderer_rejects_configuration_it_cannot_apply() {
        let defaults: Vec<Value> =
            serde_json::from_str(include_str!("../../charts/defaults.json")).unwrap();
        let mut config = defaults
            .iter()
            .find(|chart| chart["slug"] == "battery-capacity-mileage")
            .unwrap()["definition"]
            .clone();
        assert!(parse_and_validate_for_slug("battery-capacity-mileage", &config).is_ok());

        config["timeframe"] = serde_json::json!({ "mode": "relative", "preset": "7d" });
        config["x"]["kind"] = "number".into();
        config["axes"]["y"]["unit"] = "percent".into();
        config["display"]["emptyTitle"] = "Edited empty state".into();
        config["rendererId"] = "other-renderer".into();
        config["series"][0]["transforms"] = serde_json::json!([{ "kind": "scale", "factor": 2.0 }]);
        let errors = parse_and_validate_for_slug("battery-capacity-mileage", &config)
            .expect_err("unsupported bundled geometry must not be silently accepted");
        assert!(errors.iter().any(|error| error.path == "config.x"));
        assert!(errors.iter().any(|error| error.path == "config.timeframe"));
        assert!(errors
            .iter()
            .any(|error| error.path == "config.axes.y.unit"));
        assert!(errors
            .iter()
            .any(|error| error.path == "config.display.emptyTitle"));
        assert!(errors.iter().any(|error| error.path == "config.rendererId"));
        assert!(errors
            .iter()
            .any(|error| error.path == "config.series.0.transforms"));

        assert!(parse_and_validate_for_slug("independent-custom-chart", &config).is_ok());
    }

    #[test]
    fn aggregate_bundled_renderer_rejects_no_op_visual_edits() {
        let defaults: Vec<Value> = serde_json::from_str(BUNDLED_CHART_DEFAULTS).unwrap();
        let mut config = defaults
            .iter()
            .find(|chart| chart["slug"] == "charging-weekly-energy")
            .unwrap()["definition"]
            .clone();
        config["series"][0]["label"] = "Edited label".into();
        config["display"]["grid"] = (!config["display"]["grid"].as_bool().unwrap()).into();
        let errors = parse_and_validate_for_slug("charging-weekly-energy", &config)
            .expect_err("ignored aggregate visual edits must be rejected");
        assert!(
            errors.iter().any(|error| error.path == "config.series"),
            "unexpected errors: {errors:?}"
        );
        assert!(
            errors.iter().any(|error| error.path == "config.display"),
            "unexpected errors: {errors:?}"
        );
    }

    #[test]
    fn charging_curve_analysis_rejects_fixed_geometry_edits() {
        let defaults: Vec<Value> = serde_json::from_str(BUNDLED_CHART_DEFAULTS).unwrap();
        let mut config = defaults
            .iter()
            .find(|chart| chart["slug"] == "charging-curve-analysis")
            .unwrap()["definition"]
            .clone();
        config["series"][0]["mark"] = "line".into();
        config["series"][0]["fill"] = true.into();
        let errors = parse_and_validate_for_slug("charging-curve-analysis", &config)
            .expect_err("constructed charge-curve geometry must remain fixed");
        assert!(
            errors.iter().any(|error| error.path == "config.series"),
            "unexpected errors: {errors:?}"
        );
    }

    #[test]
    fn bundled_renderer_manifest_matches_seed_inventory() {
        let defaults: Vec<Value> =
            serde_json::from_str(include_str!("../../charts/defaults.json")).unwrap();
        let capabilities: Vec<BundledRendererCapability> =
            serde_json::from_str(BUNDLED_RENDERER_CAPABILITIES).unwrap();
        let default_slugs = defaults
            .iter()
            .map(|chart| chart["slug"].as_str().unwrap())
            .collect::<HashSet<_>>();
        let capability_slugs = capabilities
            .iter()
            .map(|capability| capability.slug.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(default_slugs, capability_slugs);
        assert_eq!(capability_slugs.len(), capabilities.len());
        for chart in &defaults {
            let slug = chart["slug"].as_str().unwrap();
            parse_and_validate_for_slug(slug, &chart["definition"])
                .unwrap_or_else(|errors| panic!("invalid bundled default {slug}: {errors:?}"));
        }
    }
}

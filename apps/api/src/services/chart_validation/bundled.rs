use std::{collections::HashMap, sync::LazyLock};

use super::{source_policy::Bindings, *};

struct Contract {
    capability: BundledRendererCapability,
    canonical: Value,
    series_shape: Value,
    adjustable_y_domain: bool,
}

static CONTRACTS: LazyLock<HashMap<String, Contract>> = LazyLock::new(|| {
    let capabilities: Vec<BundledRendererCapability> =
        serde_json::from_str(BUNDLED_RENDERER_CAPABILITIES)
            .expect("bundled renderer capability manifest must be valid");
    let defaults: Vec<Value> =
        serde_json::from_str(BUNDLED_CHART_DEFAULTS).expect("bundled chart defaults must be valid");
    let mut contracts = HashMap::new();
    for capability in capabilities {
        let canonical = defaults
            .iter()
            .find(|chart| chart["slug"] == capability.slug)
            .and_then(|chart| chart.get("definition"))
            .expect("every bundled renderer capability must have a default definition")
            .clone();
        let series_shape = series_without_colors(&canonical);
        let adjustable_y_domain = matches!(
            capability.renderer.as_str(),
            "daily-charge-sessions"
                | "daily-energy-bars"
                | "phantom-drain"
                | "tire-pressure-timeline"
        );
        contracts
            .entry(capability.slug.clone())
            .or_insert(Contract {
                capability,
                canonical,
                series_shape,
                adjustable_y_domain,
            });
    }
    contracts
});

pub(super) fn validate(
    slug: &str,
    value: &Value,
    definition: &ChartDefinitionV1,
) -> Vec<ChartValidationError> {
    let Some(contract) = CONTRACTS.get(slug) else {
        return Vec::new();
    };
    let bindings = Bindings::new(&definition.sources);
    let mut errors = Vec::new();
    contract.encoding(slug, definition, &bindings, &mut errors);
    contract.owned_metadata(value, &mut errors);
    if contract.capability.editing != "rich" {
        contract.fixed_presentation(value, &mut errors);
    }
    contract.curves(slug, definition, &bindings, &mut errors);
    behavior(definition, &mut errors);
    errors
}

impl Contract {
    fn encoding(
        &self,
        slug: &str,
        definition: &ChartDefinitionV1,
        bindings: &Bindings<'_>,
        errors: &mut Vec<ChartValidationError>,
    ) {
        let expected = &self.capability.x;
        let compatible = bindings
            .get(&definition.x.field.source_binding_id)
            .is_some_and(|binding| {
                binding.source_id == expected.source_id
                    && definition.x.field.field == expected.field
                    && definition.x.kind == expected.kind
            });
        if !compatible {
            errors.push(error(
                "config.x",
                &format!(
                    "The {slug} production renderer requires {} X data from {}.{}.",
                    expected.kind, expected.source_id, expected.field
                ),
            ));
        }
    }

    fn owned_metadata(&self, value: &Value, errors: &mut Vec<ChartValidationError>) {
        if value.get("timeframe") != self.canonical.get("timeframe") {
            errors.push(error("config.timeframe", "Bundled charts inherit the active dashboard timeframe so preview and dashboard data stay identical."));
        }
        for axis in ["x", "y", "y2"] {
            let pointer = format!("/axes/{axis}/unit");
            if value.pointer(&pointer) != self.canonical.pointer(&pointer) {
                errors.push(error(
                    &format!("config.axes.{axis}.unit"),
                    "Bundled renderer units are owned by the production data adapter.",
                ));
            }
        }
        append_failures(
            errors,
            [
                (
                    ["/display/emptyTitle", "/display/emptyDescription"]
                        .iter()
                        .any(|path| value.pointer(path) != self.canonical.pointer(path)),
                    "config.display.emptyTitle",
                    "Bundled empty-state copy is owned by the production renderer.",
                ),
                (
                    value.get("rendererId") != self.canonical.get("rendererId"),
                    "config.rendererId",
                    "Bundled renderer selection cannot be changed.",
                ),
            ],
        );
    }

    fn fixed_presentation(&self, value: &Value, errors: &mut Vec<ChartValidationError>) {
        let mut axes = value.get("axes").cloned().unwrap_or(Value::Null);
        let canonical_axes = self.canonical.get("axes").cloned().unwrap_or(Value::Null);
        if self.adjustable_y_domain {
            axes["y"]["domain"] = canonical_axes["y"]["domain"].clone();
        }
        append_failures(errors, [
            (value.get("sources") != self.canonical.get("sources"), "config.sources",
                "This bundled renderer owns its data bindings; duplicate the chart to build a custom source configuration."),
            (series_without_colors(value) != self.series_shape, "config.series",
                "This bundled renderer supports curve color edits only; its curve order, labels, marks, and axes are fixed."),
            (value.get("x") != self.canonical.get("x"), "config.x",
                "This bundled renderer owns its X-axis encoding."),
            (axes != canonical_axes, "config.axes",
                "This bundled renderer supports only its production axis contract; use dashboard chart settings for a temporary range."),
            (value.get("display") != self.canonical.get("display"), "config.display",
                "This bundled renderer owns its legend, grid, tooltip, and smoothing presentation."),
            (value.get("interaction") != self.canonical.get("interaction"), "config.interaction",
                "This bundled renderer owns its interaction behavior."),
        ]);
    }

    fn curves(
        &self,
        slug: &str,
        definition: &ChartDefinitionV1,
        bindings: &Bindings<'_>,
        errors: &mut Vec<ChartValidationError>,
    ) {
        for (index, series) in definition.series.iter().enumerate() {
            let compatible = bindings
                .get(&series.y.source_binding_id)
                .is_some_and(|binding| {
                    self.capability.curves.iter().any(|curve| {
                        curve.source_id == binding.source_id && curve.field == series.y.field
                    })
                });
            if !compatible {
                errors.push(error(
                    &format!("config.series.{index}.y"),
                    &format!("The {slug} production renderer does not support this curve."),
                ));
            }
            let restrictions = [
                (
                    series.x.is_some(),
                    "x",
                    "Per-series X fields are supported only by independent custom charts.",
                ),
                (
                    series
                        .transforms
                        .iter()
                        .any(|transform| !matches!(transform, ChartTransformDefinition::None)),
                    "transforms",
                    "Transforms are not supported by bundled production renderers.",
                ),
                (
                    series.stack_id.is_some()
                        && !matches!(series.mark.as_str(), "bar" | "histogram"),
                    "stackId",
                    "Stacking is supported only for bar or histogram curves.",
                ),
                (
                    series.connect_gaps.is_some(),
                    "connectGaps",
                    "Use the chart-level connectGaps setting for bundled charts.",
                ),
                (
                    series.value_format.is_some(),
                    "valueFormat",
                    "Per-curve value formatting is not supported by bundled production renderers.",
                ),
            ];
            for (rejected, field, message) in restrictions {
                if rejected {
                    errors.push(error(&format!("config.series.{index}.{field}"), message));
                }
            }
        }
    }
}

fn behavior(definition: &ChartDefinitionV1, errors: &mut Vec<ChartValidationError>) {
    append_failures(
        errors,
        [
            (
                definition
                    .annotations
                    .as_ref()
                    .is_some_and(|annotations| !annotations.is_empty()),
                "config.annotations",
                "Annotations are supported only by independent custom charts.",
            ),
            (
                definition.display.data_smoothing.is_some(),
                "config.display.dataSmoothing",
                "Use curve smoothness for bundled production renderers.",
            ),
            (
                !definition.interaction.pan_zoom,
                "config.interaction.panZoom",
                "Bundled production charts require pan and zoom.",
            ),
            (
                !definition.interaction.touch_explore,
                "config.interaction.touchExplore",
                "Bundled production charts require touch exploration.",
            ),
        ],
    );
    for (name, axis) in [
        ("x", Some(&definition.axes.x)),
        ("y", Some(&definition.axes.y)),
        ("y2", definition.axes.y2.as_ref()),
    ] {
        let Some(axis) = axis else { continue };
        if axis.scale == "log" {
            errors.push(error(
                &format!("config.axes.{name}.scale"),
                "Logarithmic axes are not supported by bundled production renderers.",
            ));
        }
        if axis.tick_format.is_some() {
            errors.push(error(
                &format!("config.axes.{name}.tickFormat"),
                "Custom tick formats are not supported by bundled production renderers.",
            ));
        }
    }
}

fn series_without_colors(value: &Value) -> Value {
    let mut series = value.get("series").cloned().unwrap_or(Value::Null);
    if let Some(items) = series.as_array_mut() {
        for item in items {
            if let Some(object) = item.as_object_mut() {
                object.remove("color");
            }
        }
    }
    series
}

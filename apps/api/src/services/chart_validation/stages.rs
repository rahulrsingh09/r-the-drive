use super::{source_policy::Bindings, *};

pub(super) fn validate(definition: &ChartDefinitionV1, errors: &mut Vec<ChartValidationError>) {
    let bindings = Bindings::new(&definition.sources);
    structure(definition, &bindings, errors);
    sources(definition, errors);
    bindings.validate(&definition.x.field, "config.x.field", errors);
    for (index, series) in definition.series.iter().enumerate() {
        curve(definition, &bindings, index, series, errors);
    }
    presentation(definition, errors);
}

fn structure(
    definition: &ChartDefinitionV1,
    bindings: &Bindings<'_>,
    errors: &mut Vec<ChartValidationError>,
) {
    let counts = [
        (
            definition.schema_version != 1,
            "config.schemaVersion",
            "Only schemaVersion 1 is supported",
        ),
        (
            definition.placements.len() > 20,
            "config.placements",
            "At most 20 placements are allowed",
        ),
        (
            !(1..=4).contains(&definition.sources.len()),
            "config.sources",
            "Between 1 and 4 source bindings are required",
        ),
        (
            !(1..=32).contains(&definition.series.len()),
            "config.series",
            "Between 1 and 32 series are required",
        ),
    ];
    append_failures(errors, counts);
    if let ChartTimeframePolicy::Relative { preset } = &definition.timeframe {
        if !["1h", "6h", "24h", "7d", "30d", "90d", "1y"].contains(&preset.as_str()) {
            errors.push(error(
                "config.timeframe.preset",
                "Unsupported relative timeframe",
            ));
        }
    }
    if bindings.len() != definition.sources.len() {
        errors.push(error("config.sources", "Source binding IDs must be unique"));
    }
    let mut series_ids = HashSet::new();
    if definition
        .series
        .iter()
        .filter(|series| series_ids.insert(&series.id))
        .count()
        != definition.series.len()
    {
        errors.push(error("config.series", "Series IDs must be unique"));
    }
    for placement in &definition.placements {
        if !id(&placement.dashboard_slug) {
            errors.push(error(
                "config.placements",
                "Invalid dashboard placement slug",
            ));
        }
    }
}

fn sources(definition: &ChartDefinitionV1, errors: &mut Vec<ChartValidationError>) {
    for binding in &definition.sources {
        if !id(&binding.id) || !id(&binding.source_id) {
            errors.push(error("config.sources", "Invalid source binding identifier"));
        }
        let policy = source_policy::lookup(&binding.source_id);
        if policy.is_none() {
            errors.push(error(
                &format!("config.sources.{}.sourceId", binding.id),
                "Unknown chart source ID",
            ));
        }
        if binding.source_id == "metrics.series"
            && !binding
                .params
                .get("metric")
                .and_then(Value::as_str)
                .is_some_and(is_series_metric)
        {
            errors.push(error(
                &format!("config.sources.{}.params.metric", binding.id),
                "Metric series requires a known series-capable metric parameter",
            ));
        }
        for filter in &binding.filters {
            let valid_operator = [
                "eq",
                "neq",
                "gt",
                "gte",
                "lt",
                "lte",
                "in",
                "not_in",
                "is_null",
                "is_not_null",
            ]
            .contains(&filter.operator.as_str());
            let supported_field =
                policy.is_some_and(|rule| rule.filters.contains(&filter.field.as_str()));
            let unary = matches!(filter.operator.as_str(), "is_null" | "is_not_null");
            append_failures(
                errors,
                [
                    (
                        !id(&filter.field) || !valid_operator,
                        "config.sources.filters",
                        "Invalid filter field or operator",
                    ),
                    (
                        !supported_field,
                        "config.sources.filters.field",
                        "Filter field is not supported by this source",
                    ),
                    (
                        unary != filter.value.is_none(),
                        "config.sources.filters.value",
                        "Null checks require no value; other operators require one",
                    ),
                    (
                        filter
                            .value
                            .as_ref()
                            .is_some_and(|value| !valid_filter_value(value)),
                        "config.sources.filters.value",
                        "Filter values must be scalar or a string/number array",
                    ),
                ],
            );
        }
    }
}

fn curve(
    definition: &ChartDefinitionV1,
    bindings: &Bindings<'_>,
    index: usize,
    series: &ChartSeriesDefinition,
    errors: &mut Vec<ChartValidationError>,
) {
    let path = format!("config.series.{index}");
    match &series.color {
        ChartColorDefinition::Token { token } if !valid_chart_color_token(token) => errors.push(
            error(&format!("{path}.color.token"), "Unknown chart color token"),
        ),
        ChartColorDefinition::Custom { light, dark }
            if !valid_custom_color(light) || !valid_custom_color(dark) =>
        {
            errors.push(error(
                &format!("{path}.color"),
                "Custom chart colors must be hex, rgb, rgba, hsl, or hsla values",
            ))
        }
        _ => {}
    }
    bindings.validate(&series.y, &format!("{path}.y"), errors);
    if let Some(x) = &series.x {
        bindings.validate(x, &format!("{path}.x"), errors);
    }
    if let Some(binding) = bindings.get(&series.y.source_binding_id) {
        let mark = match series.mark.as_str() {
            "step" => "line",
            "histogram" => "bar",
            other => other,
        };
        if !source_policy::lookup(&binding.source_id)
            .is_some_and(|policy| policy.marks.contains(&mark))
        {
            errors.push(error(
                &format!("{path}.mark"),
                "Mark is not supported by the selected source",
            ));
        }
    }
    append_failures(
        errors,
        [
            (
                !finite_options(series.stroke_width, 0.5, 12.0)
                    || !finite_options(series.point_size, 1.0, 24.0),
                "config.series",
                "Series dimensions must be finite and within bounds",
            ),
            (
                series.y_axis == "y2" && definition.axes.y2.is_none(),
                "config.axes.y2",
                "A right-axis series requires y2",
            ),
        ],
    );
    for transform in &series.transforms {
        validate_transform(transform, errors);
    }
}

fn presentation(definition: &ChartDefinitionV1, errors: &mut Vec<ChartValidationError>) {
    let transforms: usize = definition
        .series
        .iter()
        .map(|series| series.transforms.len())
        .sum();
    if transforms > 12 {
        errors.push(error(
            "config.series.transforms",
            "At most 12 transforms are allowed",
        ));
    }
    for axis in [
        Some(&definition.axes.x),
        Some(&definition.axes.y),
        definition.axes.y2.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_axis(axis, errors);
    }
    if let Some(smoothing) = &definition.display.data_smoothing {
        if !["rolling_average", "rolling_median"].contains(&smoothing.kind.as_str())
            || !(2..=1000).contains(&smoothing.window)
        {
            errors.push(error(
                "config.display.dataSmoothing",
                "Invalid smoothing definition",
            ));
        }
    }
}

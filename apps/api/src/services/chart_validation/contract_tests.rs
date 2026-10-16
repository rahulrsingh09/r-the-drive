use super::*;
use serde_json::json;

#[test]
fn complete_validation_contract_matrix() {
    let fixtures: Value = serde_json::from_str(include_str!("inputs.json")).unwrap();
    let outputs = fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let mut value = case.get("value").unwrap_or(&fixtures["base"]).clone();
            if let Some(edits) = case["edits"].as_array() {
                for edit in edits {
                    let (parent, key) = edit[0].as_str().unwrap().rsplit_once('/').unwrap();
                    let target = value.pointer_mut(parent).unwrap();
                    if let Some(array) = target.as_array_mut() {
                        array[key.parse::<usize>().unwrap()] = edit[1].clone();
                    } else {
                        target[key] = edit[1].clone();
                    }
                }
            }
            if let Some(removals) = case["remove"].as_array() {
                for pointer in removals {
                    let (parent, key) = pointer.as_str().unwrap().rsplit_once('/').unwrap();
                    value
                        .pointer_mut(parent)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(key);
                }
            }
            let result = parse_and_validate_for_slug(case["slug"].as_str().unwrap(), &value);
            let output = match result {
                Ok(definition) => json!({"definition": format!("{definition:#?}")}),
                Err(errors) => json!({"errors": errors}),
            };
            json!({"name": case["name"], "output": output})
        })
        .collect::<Vec<_>>();
    let actual = json!(outputs);
    if let Ok(directory) = std::env::var("CHART_CONTRACT_CAPTURE") {
        std::fs::write(
            format!("{directory}/validation-expected.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
        return;
    }
    let expected: Value = serde_json::from_str(include_str!("expected.json")).unwrap();
    assert_eq!(
        actual.as_array().unwrap().len(),
        expected.as_array().unwrap().len()
    );
    for (actual, expected) in actual
        .as_array()
        .unwrap()
        .iter()
        .zip(expected.as_array().unwrap())
    {
        assert_eq!(actual, expected, "case {}", actual["name"]);
    }
}

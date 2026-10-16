use super::*;
use serde_json::json;

fn bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

#[test]
fn derived_series_full_fallback_contract() {
    let start = DateTime::from_timestamp(0, 0).unwrap();
    let mut cases = Vec::new();
    for offsets in [
        vec![],
        vec![-1001],
        vec![0],
        vec![999],
        vec![-1001, -1, 0, 999, 1001, 120001],
        vec![1000, 1000, 1000],
        vec![120001, 60000, -1001],
        vec![0, 120000, 60000],
    ] {
        for (soc_start, soc_end) in [
            (20.0, 80.0),
            (80.0, 20.0),
            (-0.0, -0.0),
            (-5.0, 105.0),
            (f64::NAN, 80.0),
            (f64::MAX, f64::MAX),
            (f64::NEG_INFINITY, f64::INFINITY),
        ] {
            for cap in [-0.0, 22.0, f64::INFINITY] {
                let powers = [
                    None,
                    Some(-0.0),
                    Some(-1.0),
                    Some(f64::NAN),
                    Some(f64::INFINITY),
                    Some(22.0_f64.next_up()),
                ];
                let points: Vec<_> = offsets
                    .iter()
                    .enumerate()
                    .map(|(i, ms)| RawCurvePointRow {
                        ts: start + chrono::Duration::milliseconds(*ms),
                        power_kw: powers[i % powers.len()],
                        sample_source: if i % 2 == 0 { "parallax" } else { "rivian" }.into(),
                    })
                    .collect();
                let result = build_fallback_curve_samples(
                    Uuid::from_u128(1),
                    start,
                    start + chrono::Duration::milliseconds(60999),
                    None,
                    soc_start,
                    soc_end,
                    &points,
                    cap,
                );
                cases.push(json!({
                    "offsets":offsets,"soc_start":bits(soc_start),"soc_end":bits(soc_end),"cap":bits(cap),
                    "result":result,
                    "float_bits":result.iter().map(|r| json!([bits(r.minutes_elapsed),bits(r.charge_rate_kw),r.soc.map(bits)])).collect::<Vec<_>>()
                }));
            }
        }
    }
    for power in [
        None,
        Some(-0.0),
        Some(0.0),
        Some(22.0_f64.next_down()),
        Some(22.0),
        Some(22.0_f64.next_up()),
        Some(f64::NAN),
        Some(f64::NEG_INFINITY),
        Some(f64::INFINITY),
    ] {
        let points = [RawCurvePointRow {
            ts: start,
            power_kw: power,
            sample_source: "".into(),
        }];
        let result = build_fallback_curve_samples(
            Uuid::from_u128(2),
            start,
            start,
            Some(" DCFC "),
            20.0,
            80.0,
            &points,
            22.0,
        );
        cases.push(
            json!({"power":power.map(bits),"result":result,"bits":bits(result[0].charge_rate_kw)}),
        );
    }
    let actual = json!(cases);
    if let Ok(root) = std::env::var("DERIVED_SERIES_EVIDENCE") {
        std::fs::write(
            format!("{root}/fallback.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        serde_json::to_string_pretty(&actual).unwrap(),
        include_str!("derived-charge-fallback-expected.json").trim_end()
    );
}

#[test]
fn derived_series_invalid_cap_contract() {
    let start = DateTime::from_timestamp(0, 0).unwrap();
    for cap in [-1.0, f64::NEG_INFINITY, f64::NAN] {
        let points = [RawCurvePointRow {
            ts: start,
            power_kw: None,
            sample_source: "synthetic".into(),
        }];
        assert!(build_fallback_curve_samples(
            Uuid::nil(),
            start,
            start,
            None,
            0.0,
            100.0,
            &[],
            cap
        )
        .is_empty());
        assert!(std::panic::catch_unwind(|| build_fallback_curve_samples(
            Uuid::nil(),
            start,
            start,
            None,
            0.0,
            100.0,
            &points,
            cap
        ))
        .is_err());
    }
}

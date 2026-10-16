use super::*;
use serde_json::json;
#[test]
fn energy_efficiency_boundary_contract() {
    let base = "2026-05-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let mut outputs = Vec::new();
    for offset in [0, 1, 86399, 86400, 86401, 172800] {
        for distances in [[10.0, 30.0], [0.0, 0.0], [0.1, 10000.0]] {
            let samples = vec![
                TrendSample {
                    ts: base,
                    trip_efficiency_wh_mi: 300.0,
                    distance_miles: distances[0],
                },
                TrendSample {
                    ts: base + chrono::Duration::seconds(offset),
                    trip_efficiency_wh_mi: 400.0,
                    distance_miles: distances[1],
                },
            ];
            outputs.push(
                json!({"offset":offset,"distances":distances,"points":with_rolling_24h(samples)}),
            );
        }
    }
    outputs.push(json!({"empty":with_rolling_24h(vec![])}));
    let actual = json!(outputs);
    if let Ok(dir) = std::env::var("ENERGY_CONTRACT_CAPTURE") {
        std::fs::write(
            format!("{dir}/energy_efficiency.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    } else {
        assert_eq!(
            serde_json::to_string_pretty(&actual).unwrap(),
            include_str!("energy_efficiency.json").trim()
        );
    }
}

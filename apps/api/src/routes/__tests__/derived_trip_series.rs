use super::*;
use serde_json::{json, Value};

fn ts(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap()
}

fn bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

fn freeze(name: &str, actual: Value, expected: &str) {
    if let Ok(root) = std::env::var("DERIVED_SERIES_EVIDENCE") {
        std::fs::write(
            format!("{root}/{name}.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        serde_json::to_string_pretty(&actual).unwrap(),
        expected.trim_end()
    );
}

fn power_row(ms: i64, soc: Option<f64>, capacity: Option<f64>) -> TripPowerTelemetryRow {
    TripPowerTelemetryRow {
        ts: ts(ms),
        battery_level: soc,
        battery_capacity_wh: capacity,
        power_kw: None,
        regen_power_kw: None,
    }
}

fn power_case(label: String, rows: impl AsRef<[TripPowerTelemetryRow]>, duration: i32) -> Value {
    let rows = rows.as_ref();
    let data = derive_soc_power(rows, duration);
    let intervals: Vec<_> = data
        .intervals
        .iter()
        .map(|i| {
            json!({
                "started_at": i.started_at, "ended_at": i.ended_at,
                "net_power_kw": i.net_power_kw, "net_power_bits": bits(i.net_power_kw),
                "before": data.estimated_at(i.started_at - chrono::Duration::milliseconds(1)).map(bits),
                "start": data.estimated_at(i.started_at).map(bits),
                "last": data.estimated_at(i.ended_at - chrono::Duration::milliseconds(1)).map(bits),
                "end": data.estimated_at(i.ended_at).map(bits),
            })
        })
        .collect();
    json!({
        "case": label, "duration": duration,
        "rows": rows.iter().map(|r| json!({
            "ts":r.ts,"soc":r.battery_level.map(bits),"capacity":r.battery_capacity_wh.map(bits),
            "power":r.power_kw.map(bits),"regen":r.regen_power_kw.map(bits)
        })).collect::<Vec<_>>(),
        "metadata": data.metadata,
        "metadata_bits": [
            data.metadata.median_interval_seconds.map(bits),
            data.metadata.p90_interval_seconds.map(bits),
            data.metadata.coverage_percent.map(bits)
        ],
        "intervals":intervals
    })
}

#[test]
fn derived_series_full_power_contract() {
    let mut cases = vec![power_case("empty".into(), vec![], 0)];
    for capacity in [
        None,
        Some(39.999999),
        Some(40.0),
        Some(250.0),
        Some(250.000001),
        Some(39999.999999),
        Some(40000.0),
        Some(250000.0),
        Some(250000.000001),
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(f64::NEG_INFINITY),
    ] {
        cases.push(power_case(
            format!("capacity {capacity:?}"),
            vec![
                power_row(0, Some(80.0), capacity),
                power_row(30000, Some(79.75), None),
            ],
            60,
        ));
    }
    for ms in [
        -1, 0, 1999, 2000, 2001, 5999, 6000, 6001, 299999, 300000, 300001,
    ] {
        for delta in [
            0.05_f64.next_down(),
            0.05,
            0.05_f64.next_up(),
            0.5,
            0.5_f64.next_up(),
            -0.5,
        ] {
            cases.push(power_case(
                format!("exact regen interval {ms} delta {}", bits(delta)),
                vec![
                    power_row(0, Some(0.0), Some(250.0)),
                    power_row(ms, Some(delta.abs()), None),
                ],
                300,
            ));
            cases.push(power_case(
                format!("interval {ms} delta {}", bits(delta)),
                vec![
                    power_row(0, Some(0.5), Some(250.0)),
                    power_row(ms, Some(0.5 - delta), None),
                ],
                300,
            ));
        }
    }
    for duration in [i32::MIN, -1, 0, 1, 20, 400, i32::MAX] {
        let mut rows = vec![
            power_row(0, Some(80.0), Some(40.0)),
            power_row(2500, Some(79.9), Some(120000.0)),
            power_row(4000, Some(79.9), Some(250.0)),
            power_row(10000, None, None),
            power_row(11000, Some(f64::NAN), None),
            power_row(12000, Some(79.7), None),
            power_row(13000, Some(79.7 + 1e-7), None),
            power_row(60000, Some(79.8), None),
            power_row(390000, Some(79.9), None),
            power_row(392001, Some(79.7), None),
        ];
        cases.push(power_case(
            format!("ordered duration {duration}"),
            &rows,
            duration,
        ));
        rows.reverse();
        cases.push(power_case(
            format!("reversed duration {duration}"),
            rows,
            duration,
        ));
    }
    for soc in [
        None,
        Some(-0.0),
        Some(0.0),
        Some(-0.1),
        Some(100.0),
        Some(100.1),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        cases.push(power_case(
            format!("SOC {soc:?}"),
            vec![
                power_row(0, soc, Some(100.0)),
                power_row(10000, Some(99.9), None),
            ],
            10,
        ));
    }
    for delta in [1e-6_f64.next_down(), 1e-6, 1e-6_f64.next_up()] {
        cases.push(power_case(
            format!("dedup {}", bits(delta)),
            vec![
                power_row(0, Some(0.0), Some(100.0)),
                power_row(1000, Some(delta), None),
                power_row(11000, Some(0.1), None),
                power_row(11000, Some(0.2), None),
                power_row(21000, Some(0.3), None),
            ],
            30,
        ));
    }
    for power in [
        None,
        Some(-0.0),
        Some(-5.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        for second in [false, true] {
            let mut rows = vec![
                power_row(0, Some(80.0), None),
                power_row(2000, Some(79.9), None),
            ];
            rows[0].power_kw = power;
            rows[0].regen_power_kw = power;
            rows[1].regen_power_kw = second.then_some(0.0);
            cases.push(power_case(
                format!("direct {power:?} second {second}"),
                rows,
                2,
            ));
        }
    }
    freeze(
        "power",
        json!(cases),
        include_str!("derived-trip-power-expected.json"),
    );
}

fn trip(source: Option<&str>) -> TripRow {
    TripRow {
        id: Uuid::from_u128(1),
        started_at: ts(0),
        ended_at: ts(600000),
        duration_seconds: Some(600),
        distance_miles: None,
        efficiency_wh_per_mile: None,
        max_speed_mph: None,
        drive_mode: None,
        soc_start: None,
        soc_end: None,
        start_lat: None,
        start_lng: None,
        end_lat: None,
        end_lng: None,
        start_place: None,
        end_place: None,
        start_address: None,
        end_address: None,
        outside_temp_c: None,
        outside_temp_source: source.map(str::to_owned),
        tags: json!([]),
    }
}

#[test]
fn derived_series_full_temperature_contract() {
    let mut cases = Vec::new();
    for source in [
        None,
        Some(""),
        Some("vehicle"),
        Some("mixed"),
        Some("open_meteo"),
        Some("unavailable"),
        Some("custom"),
    ] {
        for raw in [vec![], vec![0, 30, 60], vec![999]] {
            for offsets in [
                vec![],
                vec![-31, 31],
                vec![-30, 30],
                vec![30, -30],
                vec![0, 0],
                vec![60],
            ] {
                let weather: Vec<_> = offsets
                    .iter()
                    .enumerate()
                    .map(|(i, elapsed)| TripWeatherRow {
                        sampled_at: ts(123000 + i as i64 * 1000),
                        elapsed_seconds: *elapsed,
                        temperature_c: -100.0 + i as f64,
                    })
                    .collect();
                let samples = TripDetailSamples {
                    elapsed_s: vec![-1, 0, 0, 29, 30, 31, 60, 61, 1000],
                    outside_temp_c: vec![
                        Some(-0.0),
                        Some(0.0),
                        Some(f64::NAN),
                        None,
                        Some(f64::INFINITY),
                        Some(f64::NEG_INFINITY),
                        Some(15.5),
                        Some(-20.0),
                    ],
                    ..Default::default()
                };
                let raw_elapsed = raw.iter().copied().collect();
                let result =
                    build_outside_temperature(&trip(source), &samples, &weather, &raw_elapsed);
                cases.push(json!({
                    "source":source,"raw":raw,"weather_offsets":offsets,"result":result,
                    "temperature_bits":result.samples.iter().map(|s| bits(s.temperature_c)).collect::<Vec<_>>()
                }));
            }
        }
    }
    for (elapsed, temperature) in [
        (vec![], vec![Some(10.0)]),
        (vec![0], vec![]),
        (vec![0], vec![None]),
    ] {
        let samples = TripDetailSamples {
            elapsed_s: elapsed,
            outside_temp_c: temperature,
            ..Default::default()
        };
        cases.push(json!({"case":"empty/sparse","result":build_outside_temperature(&trip(None), &samples, &[], &Default::default())}));
    }
    freeze(
        "temperature",
        json!(cases),
        include_str!("derived-trip-temperature-expected.json"),
    );
}

#[test]
fn derived_series_temperature_overflow_contract() {
    let weather = [TripWeatherRow {
        sampled_at: ts(0),
        elapsed_seconds: i32::MIN,
        temperature_c: 1.0,
    }];
    let samples = TripDetailSamples {
        elapsed_s: vec![i32::MAX],
        outside_temp_c: vec![Some(10.0)],
        ..Default::default()
    };
    assert!(std::panic::catch_unwind(|| build_outside_temperature(
        &trip(None),
        &samples,
        &weather,
        &Default::default()
    ))
    .is_err());
}

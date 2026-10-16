use super::*;
use serde_json::{json, Value};

fn row() -> PhantomCandidateRow {
    PhantomCandidateRow {
        period_start: "2026-05-01T00:00:00Z".parse().unwrap(),
        period_end: "2026-05-01T12:00:00Z".parse().unwrap(),
        duration_hours: 12.0,
        soc_start: Some(70.0),
        soc_end: Some(66.0),
        start_sample_at: Some("2026-05-01T00:05:00Z".parse().unwrap()),
        end_sample_at: Some("2026-05-01T11:56:00Z".parse().unwrap()),
        start_sample_gap_minutes: Some(5.0),
        end_sample_gap_minutes: Some(4.0),
        range_start_mi: Some(250.0),
        range_end_mi: Some(236.0),
        start_range_gap_minutes: Some(5.0),
        end_range_gap_minutes: Some(4.0),
        capacity_wh: Some(131000.0),
        sample_count: 8,
        battery_sample_count: 6,
        interior_battery_sample_count: 4,
        min_soc_in_window: Some(66.0),
        max_soc_in_window: Some(70.0),
        odometer_delta_mi: Some(0.0),
        drive_sample_count: 0,
        trip_overlap_count: 0,
        charge_overlap_count: 0,
        sleep_seconds: Some(41000.0),
        covered_state_seconds: Some(43200.0),
        has_reduced_range: Some(false),
    }
}

fn record(label: String, row: PhantomCandidateRow) -> Value {
    let period = derive_phantom_period(row);
    let bits: Vec<_> = [
        period.duration_hours,
        period.sleep_share_pct,
        period.state_coverage_pct,
        period.soc_start,
        period.soc_end,
        period.soc_lost_pct,
        period.drain_pct_per_hour,
        period.range_start_mi,
        period.range_end_mi,
        period.range_lost_mi,
        period.range_lost_per_hour_mi,
        period.energy_drained_kwh,
        period.avg_power_w,
    ]
    .map(|v| v.map(|v| format!("{:016x}", v.to_bits())))
    .into();
    json!({"case":label,"period":period,"float_bits":bits})
}

#[test]
fn idle_projection_full_contract() {
    let mut outputs = Vec::new();
    for (name, value) in [
        ("none", None),
        ("nan", Some(f64::NAN)),
        ("inf", Some(f64::INFINITY)),
        ("neg_inf", Some(f64::NEG_INFINITY)),
        ("neg_zero", Some(-0.0)),
        ("zero", Some(0.0)),
        ("negative", Some(-1.0)),
        ("tiny", Some(f64::MIN_POSITIVE)),
        ("max", Some(f64::MAX)),
    ] {
        for field in [
            "duration",
            "soc_start",
            "soc_end",
            "capacity",
            "sleep",
            "coverage",
            "soc_gap_start",
            "soc_gap_end",
            "range_gap_start",
            "range_gap_end",
            "range_start",
            "range_end",
            "window_min",
            "window_max",
            "odometer",
        ] {
            let mut r = row();
            match field {
                "duration" => r.duration_hours = value.unwrap_or(0.0),
                "soc_start" => r.soc_start = value,
                "soc_end" => r.soc_end = value,
                "capacity" => r.capacity_wh = value,
                "sleep" => r.sleep_seconds = value,
                "coverage" => r.covered_state_seconds = value,
                "soc_gap_start" => r.start_sample_gap_minutes = value,
                "soc_gap_end" => r.end_sample_gap_minutes = value,
                "range_gap_start" => r.start_range_gap_minutes = value,
                "range_gap_end" => r.end_range_gap_minutes = value,
                "range_start" => r.range_start_mi = value,
                "range_end" => r.range_end_mi = value,
                "window_min" => r.min_soc_in_window = value,
                "window_max" => r.max_soc_in_window = value,
                "odometer" => r.odometer_delta_mi = value,
                _ => unreachable!(),
            }
            outputs.push(record(format!("{field}/{name}"), r));
        }
    }
    for (duration, cutoff) in [(17.999, 45.0_f64), (18.0, 720.0), (48.0, 4320.0)] {
        for gap in [cutoff.next_down(), cutoff, cutoff.next_up()] {
            for end in [false, true] {
                let mut r = row();
                r.duration_hours = duration;
                if end {
                    r.end_sample_gap_minutes = Some(gap);
                    r.end_range_gap_minutes = Some(gap);
                } else {
                    r.start_sample_gap_minutes = Some(gap);
                    r.start_range_gap_minutes = Some(gap);
                }
                outputs.push(record(format!("gap/{duration}/{gap}/{end}"), r));
            }
        }
    }
    for mask in 0..128 {
        let mut r = row();
        r.charge_overlap_count = i64::from(mask & 1 != 0);
        r.trip_overlap_count = i64::from(mask & 2 != 0);
        if mask & 4 != 0 {
            r.soc_end = Some(71.0);
        }
        if mask & 8 != 0 {
            r.drive_sample_count = 10;
        }
        if mask & 16 != 0 {
            r.start_sample_gap_minutes = None;
        }
        if mask & 32 != 0 {
            r.battery_sample_count = 1;
        }
        if mask & 64 != 0 {
            r.min_soc_in_window = Some(70.0);
        }
        outputs.push(record(format!("precedence/{mask}"), r));
    }
    for variant in 0..20 {
        let mut r = row();
        match variant {
            0..=2 => {
                r.min_soc_in_window =
                    Some([69.0_f64.next_down(), 69.0, 69.0_f64.next_up()][variant]);
            }
            3..=5 => {
                r.range_start_mi = Some(100.0);
                r.soc_end = r.soc_start;
                r.range_end_mi =
                    Some([85.0_f64.next_down(), 85.0, 85.0_f64.next_up()][variant - 3]);
            }
            6..=8 => {
                r.soc_start = Some(100.0);
                r.soc_end = Some(100.0);
                r.range_start_mi = Some(82.5);
                r.range_end_mi =
                    Some([117.499999, 117.5, 117.500001][variant - 6]);
            }
            9 => {
                r.range_start_mi = Some(f64::MAX);
                r.range_end_mi = Some(f64::MAX);
                r.soc_start = Some(1.0);
                r.soc_end = Some(1.0);
            }
            10 => {
                r.soc_start = Some(f64::MAX);
                r.soc_end = Some(-f64::MAX);
            }
            11 => r.duration_hours = f64::from_bits(1),
            12 => {
                r.start_sample_at = None;
                r.end_sample_at = None;
                r.has_reduced_range = None;
                r.sample_count = -1;
            }
            13 => {
                r.start_sample_at = r.end_sample_at;
                r.has_reduced_range = Some(true);
                r.sample_count = 3;
            }
            14 => {
                r.range_start_mi = Some(0.0);
                r.range_end_mi = Some(0.0);
            }
            15 => r.range_end_mi = Some(251.0),
            16 => {
                r.duration_hours = 18.0;
                r.interior_battery_sample_count = 0;
            }
            17 => {
                r.soc_end = Some(62.0);
                r.interior_battery_sample_count = 0;
            }
            18 => {
                r.battery_sample_count = i64::MAX;
                r.interior_battery_sample_count = i64::MAX;
                r.sample_count = i64::MAX;
            }
            _ => {
                r.trip_overlap_count = -1;
                r.charge_overlap_count = -1;
                r.drive_sample_count = -1;
                r.covered_state_seconds = Some(43200.0_f64.next_up());
                r.sleep_seconds = Some(43200.0_f64.next_up());
            }
        }
        outputs.push(record(format!("boundary/{variant}"), r));
    }
    let actual = serde_json::to_string_pretty(&outputs).unwrap();
    if let Ok(directory) = std::env::var("IDLE_LIVE_CAPTURE") {
        std::fs::write(format!("{directory}/idle_projection.json"), &actual).unwrap();
        return;
    }
    assert_eq!(actual, include_str!("idle_projection.json").trim());
    if let Ok(directory) = std::env::var("IDLE_LIVE_EXPORT") {
        std::fs::write(format!("{directory}/idle_projection.json"), actual).unwrap();
    }
}

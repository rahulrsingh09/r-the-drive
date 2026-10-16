use super::*;
use serde_json::json;
fn candidate_row() -> PhantomCandidateRow {
    PhantomCandidateRow {
        period_start: DateTime::parse_from_rfc3339("2026-05-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        period_end: DateTime::parse_from_rfc3339("2026-05-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        duration_hours: 12.0,
        soc_start: Some(70.0),
        soc_end: Some(66.0),
        start_sample_at: Some(
            DateTime::parse_from_rfc3339("2026-05-01T00:05:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ),
        end_sample_at: Some(
            DateTime::parse_from_rfc3339("2026-05-01T11:56:00Z")
                .unwrap()
                .with_timezone(&Utc),
        ),
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

#[test]
fn energy_idle_boundary_contract() {
    let mut output = Vec::new();
    for duration in [0.0, 0.25, 6.0, 17.999, 18.0, 47.999, 48.0, 72.0] {
        for loss in [-1.0, 0.0, 4.0, 7.999, 8.0, 16.0] {
            for count in [0, 2, 3, 4, 5] {
                let mut row = candidate_row();
                row.duration_hours = duration;
                row.soc_end = Some(70.0 - loss);
                row.battery_sample_count = count;
                row.interior_battery_sample_count = if count > 2 { 1 } else { 0 };
                output.push(json!({"duration":duration,"loss":loss,"count":count,"period":derive_phantom_period(row)}));
            }
        }
    }
    for variant in 0..22 {
        let mut row = candidate_row();
        match variant {
            0 => row.charge_overlap_count = 1,
            1 => {
                row.charge_overlap_count = 1;
                row.trip_overlap_count = 1;
                row.soc_end = Some(80.0);
            }
            2 => {
                row.trip_overlap_count = 1;
                row.soc_end = Some(80.0);
            }
            3 => row.odometer_delta_mi = Some(1.999),
            4 => row.odometer_delta_mi = Some(2.0),
            5 => row.drive_sample_count = 9,
            6 => row.drive_sample_count = 10,
            7 => row.start_sample_gap_minutes = Some(45.0),
            8 => row.start_sample_gap_minutes = Some(45.001),
            9 => row.start_sample_gap_minutes = None,
            10 => row.soc_start = Some(f64::NAN),
            11 => row.capacity_wh = Some(10000.0),
            12 => row.capacity_wh = Some(10000.1),
            13 => row.covered_state_seconds = Some(21599.0),
            14 => row.covered_state_seconds = Some(21600.0),
            15 => row.range_end_mi = Some(10.0),
            16 => row.sample_count = 2,
            17 => row.start_range_gap_minutes = None,
            18 => row.min_soc_in_window = Some(69.9),
            19 => row.capacity_wh = Some(f64::INFINITY),
            20 => row.duration_hours = -1.0,
            _ => {
                row.sleep_seconds = Some(-1.0);
                row.covered_state_seconds = Some(90000.0);
            }
        }
        output.push(json!({"variant":variant,"period":derive_phantom_period(row)}));
    }
    let actual = json!(output);
    if let Ok(dir) = std::env::var("ENERGY_CONTRACT_CAPTURE") {
        std::fs::write(
            format!("{dir}/energy_idle.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    } else {
        assert_eq!(
            serde_json::to_string_pretty(&actual).unwrap(),
            include_str!("energy_idle.json").trim()
        );
    }
}

#[test]
fn energy_default_time_windows() {
    for days in [7, 30, 90, 180, 365, 730] {
        let before = Utc::now();
        let (from, to) = resolve_time_bounds(None, None, false, days);
        let after = Utc::now();
        assert!(
            from >= before - chrono::Duration::days(days)
                && from <= after - chrono::Duration::days(days)
        );
        assert!(to >= before && to <= after);
    }
}

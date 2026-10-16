use super::tests::make_helper_state;
use super::*;

#[test]
fn demo_vehicle_keys_are_detected_explicitly() {
    assert!(super::is_demo_vehicle_key("demo-r1t-local"));
    assert!(super::is_demo_vehicle_key("demo-r2-local"));
    assert!(super::is_demo_vehicle_key("demo-r2s-local"));
    assert!(!super::is_demo_vehicle_key("rivian-1234"));
}

#[test]
fn r2_model_aliases_canonicalize_before_storage_and_response() {
    assert_eq!(super::canonical_vehicle_model("R2"), "R2");
    assert_eq!(super::canonical_vehicle_model(" r2s "), "R2");
    assert_eq!(super::canonical_vehicle_model("R2-S"), "R2");
    assert_eq!(super::canonical_vehicle_model("R1S"), "R1S");
    assert_eq!(super::canonical_battery_config("R2S"), "r2");
    assert_eq!(super::canonical_vehicle_name("R2S"), "R2");
}

#[test]
fn raw_telemetry_rejects_unknown_field_filters() {
    assert!(parse_raw_fields(Some("battery_level,not_a_telemetry_field")).is_err());
}

#[test]
fn raw_selectors_reject_control_and_oversized_input() {
    assert!(parse_raw_fields(Some("battery_level\n".repeat(300).as_str())).is_err());
    assert!(parse_telemetry_lanes(Some("battery\u{0000},drive")).is_err());
}

#[test]
fn raw_telemetry_selected_fields_are_bounded_to_known_columns() {
    let fields = parse_raw_fields(Some("battery_level,tire_fl_psi,battery_level"))
        .expect("known fields should parse");
    assert_eq!(fields, vec!["battery_level", "tire_fl_psi"]);

    let clause = raw_telemetry_where_clause(&fields, false);
    assert!(clause.contains("t.battery_level IS NOT NULL OR t.tire_fl_psi IS NOT NULL"));
    assert!(clause.contains("to_jsonb(t)::text ILIKE"));
}

#[test]
fn raw_telemetry_field_coverage_uses_typed_rows_not_wide_json() {
    let query = raw_field_coverage_query("t.vehicle_id = $1");

    assert_eq!(RAW_TELEMETRY_FIELDS.len(), 52);
    assert!(query.contains("unnest(ARRAY["));
    assert!(query.contains("AS field"));
    assert!(query.contains("AS sample_count"));
    assert!(!query.contains("jsonb_build_object"));
}

#[test]
fn telemetry_lane_queries_are_bounded_and_allowlisted() {
    let lanes = parse_telemetry_lanes(Some("battery,drive,battery")).expect("known lanes");
    assert_eq!(lanes, vec!["battery", "drive"]);
    assert!(parse_telemetry_lanes(Some("raw_payload")).is_err());

    let from = "2026-07-14T00:00:00Z".parse().expect("valid timestamp");
    let to = "2026-07-14T01:00:00Z".parse().expect("valid timestamp");
    assert_eq!(
        resolve_telemetry_resolution(Some("5m"), from, to, 256).unwrap(),
        300
    );
}

#[test]
fn telemetry_lane_runtime_query_aliases_every_row_field() {
    for field in [
        "latitude",
        "longitude",
        "altitude_m",
        "speed_mph",
        "battery_level",
        "battery_capacity_wh",
        "distance_to_empty_mi",
        "battery_limit",
        "time_to_end_of_charge_min",
        "cabin_temp_c",
        "driver_temp_c",
        "outside_temp_c",
        "power_kw",
        "regen_power_kw",
        "heading_deg",
        "odometer_miles",
        "tire_fl_psi",
        "tire_fr_psi",
        "tire_rl_psi",
        "tire_rr_psi",
    ] {
        assert!(
            TELEMETRY_LANES_QUERY.contains(&format!("AS {field}")),
            "telemetry lane query must alias {field}"
        );
    }
}

#[test]
fn raw_telemetry_rejects_inverted_time_bounds() {
    let from = "2026-07-14T00:00:00Z".parse().expect("valid timestamp");
    let to = "2026-07-13T00:00:00Z".parse().expect("valid timestamp");
    assert!(validate_raw_time_bounds(Some(from), Some(to)).is_err());
}

#[test]
fn resolved_image_url_keeps_a_first_party_path_when_local_file_is_missing() {
    let config = crate::config::Config {
        database_url: "postgres://localhost/test".into(),
        redis_url: "redis://localhost".into(),
        jwt_secret: None,
        jwt_public_key: None,
        age_encryption_key: None,
        port: 3001,
        allowed_origins: vec![],
        s3_endpoint: None,
        s3_access_key: None,
        s3_secret_key: None,
        backup_artifact_dir: std::env::temp_dir()
            .join("panda-test-backups")
            .to_string_lossy()
            .into_owned(),
        vehicle_image_cache_dir: std::env::temp_dir()
            .join("panda-test-missing-mirror")
            .to_string_lossy()
            .into_owned(),
        backup_driver: "pg_dump".into(),
        backup_poll_interval_seconds: 60,
        restore_agent_url: "http://127.0.0.1:3002".into(),
        restore_agent_key_file: "/backups/.restore-agent-key".into(),
        recovery: crate::config::RecoveryConfig::default(),
        origin_bind: crate::config::OriginBindConfig::default(),
        security: Default::default(),
        rivian_ws_reconnect_initial_seconds: 10,
        rivian_ws_reconnect_max_seconds: 900,
        rivian_raw_event_retention_days: 7,
        rivian_persist_raw_events: true,
        rivian_suppress_duplicate_telemetry: true,
        panda_env: None,
        cookie_insecure: None,
        allow_insecure_lan_http_auth: false,
        rate_limit: crate::config::RateLimitConfig::default(),
    };
    let metadata = serde_json::json!({
        "mirror_status": "ready",
        "mirror_key": "abc.webp",
        "mirror_relpath": "vehicle/abc.webp"
    });

    let url = super::resolved_image_url(
        &config,
        uuid::Uuid::nil(),
        "https://rivian.com/mobile/static/img/example.webp",
        &metadata,
    );

    assert_eq!(
        url,
        Some("/v1/vehicle-image-cache/00000000-0000-0000-0000-000000000000/abc.webp".into())
    );
}

#[tokio::test]
async fn resolved_image_url_keeps_a_first_party_path_for_a_corrupt_local_file() {
    use sha2::{Digest, Sha256};

    let vehicle_id = Uuid::new_v4();
    let mut config = make_helper_state("redis://127.0.0.1/".into()).config;
    config.vehicle_image_cache_dir = std::env::temp_dir()
        .join(format!("panda-artwork-checksum-{vehicle_id}"))
        .to_string_lossy()
        .into_owned();
    let relpath = format!("{vehicle_id}/artwork.webp");
    let path = super::mirror_file_path(&config, &relpath);
    std::fs::create_dir_all(path.parent().expect("cache parent")).expect("cache directory");
    std::fs::write(&path, b"valid-artwork").expect("cache asset");
    let metadata = serde_json::json!({
        "mirror_status": "ready",
        "mirror_key": "artwork.webp",
        "mirror_relpath": relpath,
        "sha256": hex::encode(Sha256::digest(b"valid-artwork"))
    });

    assert_eq!(
        super::resolved_image_url(
            &config,
            vehicle_id,
            "https://rivian.com/artwork.webp",
            &metadata
        ),
        Some(format!("/v1/vehicle-image-cache/{vehicle_id}/artwork.webp"))
    );

    std::fs::write(&path, b"corrupt-artwork").expect("corrupt cache asset");
    assert_eq!(
        super::resolved_image_url(
            &config,
            vehicle_id,
            "https://rivian.com/artwork.webp",
            &metadata
        ),
        Some(format!("/v1/vehicle-image-cache/{vehicle_id}/artwork.webp"))
    );
    let _ = std::fs::remove_dir_all(&config.vehicle_image_cache_dir);
}

use super::*;
use axum::body::to_bytes;
use serde_json::json;

fn active(now: DateTime<Utc>, variant: usize) -> ActiveLiveSession {
    let mut active = ActiveLiveSession {
        parallax_live_power_kw: Some(11.4),
        parallax_total_charged_kwh: Some(3.1),
        parallax_pack_energy_kwh: Some(2.8),
        parallax_thermal_energy_kwh: Some(0.3),
        parallax_time_remaining_minutes: Some(42),
        parallax_power_observed_at: Some(now),
        parallax_total_energy_observed_at: Some(now),
        parallax_pack_energy_observed_at: Some(now),
        parallax_thermal_energy_observed_at: Some(now),
        parallax_time_observed_at: Some(now),
    };
    let observed = match variant {
        0 => None,
        1 => Some(now - chrono::Duration::seconds(120) - chrono::Duration::nanoseconds(1)),
        2 => Some(now - chrono::Duration::seconds(120)),
        3 => Some(now - chrono::Duration::seconds(120) + chrono::Duration::nanoseconds(1)),
        4 => Some(now + chrono::Duration::days(100)),
        _ => Some(now),
    };
    active.parallax_power_observed_at = observed;
    active.parallax_total_energy_observed_at = observed;
    active.parallax_pack_energy_observed_at = observed;
    active.parallax_thermal_energy_observed_at = observed;
    active.parallax_time_observed_at = observed;
    match variant {
        5 => {
            active.parallax_live_power_kw = None;
            active.parallax_total_charged_kwh = None;
            active.parallax_pack_energy_kwh = None;
            active.parallax_thermal_energy_kwh = None;
            active.parallax_time_remaining_minutes = None;
        }
        6..=10 => {
            let v = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, f64::MAX][variant - 6];
            active.parallax_live_power_kw = Some(v);
            active.parallax_total_charged_kwh = Some(v);
            active.parallax_pack_energy_kwh = Some(v);
            active.parallax_thermal_energy_kwh = Some(v);
            active.parallax_time_remaining_minutes = Some(i32::MIN);
        }
        11 => {
            active.parallax_live_power_kw = None;
            active.parallax_total_energy_observed_at = None;
            active.parallax_thermal_energy_observed_at = Some(now - chrono::Duration::seconds(121));
            active.parallax_time_remaining_minutes = Some(i32::MAX);
        }
        12 => active.parallax_time_remaining_minutes = Some(0),
        _ => {}
    }
    active
}

#[tokio::test]
async fn live_projection_full_contract() {
    let now = "2026-05-01T12:00:00.123456789Z".parse().unwrap();
    let mut outputs = Vec::new();
    for (name, raw) in [
        ("absent", None),
        ("invalid", Some("{broken")),
        ("null", Some("null")),
        ("array", Some("[1,2]")),
        ("scalar", Some("42")),
        ("empty", Some("{}")),
        (
            "legacy",
            Some(
                r#"{"power_kw":7.2,"energy_kwh":2.1,"time_remaining_min":50,"ts":"unparsed-clock","extra":{"unchanged":true}}"#,
            ),
        ),
        (
            "null_fields",
            Some(r#"{"power_kw":null,"energy_kwh":null,"time_remaining_min":null,"ts":12}"#),
        ),
        (
            "non_numeric",
            Some(r#"{"power_kw":"7","energy_kwh":false,"time_remaining_min":[],"ts":null}"#),
        ),
        (
            "old_provenance",
            Some(
                r#"{"provenance":{"opaque":"preserved"},"pack_energy_kwh":7,"thermal_energy_kwh":8}"#,
            ),
        ),
        (
            "provenance_replaced",
            Some(r#"{"provenance":{"opaque":"discarded"},"power_kw":0}"#),
        ),
        (
            "duplicates",
            Some(r#"{"power_kw":1,"power_kw":2,"energy_kwh":null,"energy_kwh":0}"#),
        ),
        ("legacy_only_extra", Some(r#"{"extra":null}"#)),
    ] {
        for variant in 0..15 {
            let merged = merge_live_session(
                raw.map(str::to_string),
                (variant != 14).then(|| active(now, variant)),
                now,
            );
            let response = live_session_response(merged.clone());
            let headers: std::collections::BTreeMap<_, _> = response
                .headers()
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_str().unwrap().to_string()))
                .collect();
            let status = response.status().as_u16();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            outputs.push(json!({
                "case":format!("{name}/{variant}"),
                "merged":merged,
                "status":status,
                "headers":headers,
                "body":String::from_utf8(body.to_vec()).unwrap()
            }));
        }
    }
    let actual = serde_json::to_string_pretty(&outputs).unwrap();
    if let Ok(directory) = std::env::var("IDLE_LIVE_CAPTURE") {
        std::fs::write(format!("{directory}/live_projection.json"), &actual).unwrap();
        return;
    }
    assert_eq!(actual, include_str!("live_projection.json").trim());
    if let Ok(directory) = std::env::var("IDLE_LIVE_EXPORT") {
        std::fs::write(format!("{directory}/live_projection.json"), actual).unwrap();
    }
}

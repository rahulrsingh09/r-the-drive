use super::*;
use serde_json::{json, Value};
#[path = "diagnostic_support.rs"]
mod support;
use support::*;

const WINDOW: &str = "from=2026-10-01T00:00:00Z&to=2026-10-01T00:04:00Z";

#[tokio::test]
#[ignore = "requires isolated diagnostic TimescaleDB and Redis"]
async fn diagnostic_contract_raw_http_db() {
    let f = Fixture::new().await;
    f.seed().await;
    f.telemetry(false).await;
    let mut e = Evidence::default();
    e.events.push(json!({"case":"initial-state","state":f.snapshot().await,"protection":f.protection().await}));
    for actor in [
        "owner",
        "manager",
        "viewer",
        "foreign",
        "admin",
        "anonymous",
        "key",
    ] {
        for (vehicle, label) in [
            (101, "own"),
            (102, "empty"),
            (103, "foreign"),
            (999, "missing"),
        ] {
            e.record(
                &f,
                &format!("access-{actor}-{label}"),
                actor,
                vehicle,
                "raw-data",
                WINDOW,
            )
            .await;
        }
    }
    let fields = RAW_TELEMETRY_FIELDS.join(",");
    for (name, query) in [
        ("default", String::new()),
        ("window", WINDOW.into()),
        ("from-only", "from=2026-10-01T00:00:00Z".into()),
        ("to-only", "to=2026-10-01T00:00:00Z".into()),
        (
            "equal",
            "from=2026-10-01T00:00:00Z&to=2026-10-01T00:00:00Z".into(),
        ),
        (
            "reversed",
            "from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z".into(),
        ),
        (
            "ninety-days",
            "from=2026-07-03T00:00:00Z&to=2026-10-01T00:00:00Z".into(),
        ),
        (
            "over-ninety-days",
            "from=2026-07-02T23:59:59Z&to=2026-10-01T00:00:00Z".into(),
        ),
        ("all-null", format!("{WINDOW}&to=2026-10-01T00:00:00Z")),
        ("populated", "populated_only=true".into()),
        ("not-populated", "populated_only=false".into()),
        ("selected", "fields=battery_level,tire_fl_psi".into()),
        (
            "selected-dedup",
            "fields=tire_fl_psi,battery_level,tire_fl_psi".into(),
        ),
        ("all-fields", format!("fields={}", encode(&fields))),
        ("empty-fields", "fields=".into()),
        ("commas-fields", "fields=%20,%20,,".into()),
        ("unknown-field", "fields=not_a_column".into()),
        ("all-literal", "fields=all".into()),
        ("control-field", "fields=battery_level%00".into()),
        ("oversized-field", format!("fields={}", "a".repeat(2049))),
        ("field-boundary", format!("fields={}", "a".repeat(2048))),
        (
            "injection-field",
            "fields=battery_level%3BDROP%20TABLE%20telemetry".into(),
        ),
        ("search", "search=SPARSE-NEEDLE".into()),
        ("search-trim", "search=%20sparse-needle%20".into()),
        ("search-empty", "search=%20%20".into()),
        ("search-json-key", "search=vehicle_id".into()),
        ("search-wildcard", "search=%25".into()),
        ("search-none", "search=definitely-absent".into()),
        ("search-quote", "search=%27".into()),
        (
            "search-field",
            "search=connected&fields=battery_level".into(),
        ),
        ("page-one", "per_page=3&page=1".into()),
        ("page-two", "per_page=3&page=2".into()),
        ("page-last", "per_page=3&page=4".into()),
        ("page-beyond", "per_page=3&page=5".into()),
        ("limit-alias", "limit=3&page=2".into()),
        ("per-page-precedence", "per_page=2&limit=7".into()),
        ("page-zero", "page=0&per_page=0".into()),
        ("page-negative", "page=-50&limit=-10".into()),
        ("per-page-clamp", "per_page=201".into()),
        (
            "page-overflow",
            "page=9223372036854775807&per_page=200".into(),
        ),
        ("integer-overflow", "page=9223372036854775808".into()),
        ("bad-integer", "limit=no".into()),
        ("bad-bool", "populated_only=1".into()),
        ("bad-time", "from=wrong".into()),
        (
            "duplicate-field",
            "fields=speed_mph&fields=battery_level".into(),
        ),
        (
            "validation-priority",
            "from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z&fields=unknown".into(),
        ),
    ] {
        e.record(&f, name, "owner", 101, "raw-data", &query).await;
    }
    e.record(
        &f,
        "empty-populated",
        "owner",
        102,
        "raw-data",
        "populated_only=true",
    )
    .await;
    e.record(
        &f,
        "null-only",
        "owner",
        101,
        "raw-data",
        "from=2026-10-01T00:00:00Z&to=2026-10-01T00:00:00Z",
    )
    .await;
    e.record(
        &f,
        "null-only-filtered",
        "owner",
        101,
        "raw-data",
        "from=2026-10-01T00:00:00Z&to=2026-10-01T00:00:00Z&populated_only=true",
    )
    .await;
    let mut state = f.snapshot().await;
    for key in ["last_used_at", "updated_at"] {
        state["panda.api_keys"][0][key] = json!("<verified-api-key-clock>");
    }
    e.events
        .push(json!({"case":"final-state","state":state,"protection":f.protection().await}));
    e.finish("raw", include_str!("diagnostic_raw.json"));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated diagnostic TimescaleDB and Redis"]
async fn diagnostic_contract_lanes_http_db() {
    let f = Fixture::new().await;
    f.seed().await;
    f.telemetry(false).await;
    let mut e = Evidence::default();
    e.events.push(json!({"case":"initial-state","state":f.snapshot().await,"protection":f.protection().await}));
    for actor in [
        "owner",
        "manager",
        "viewer",
        "foreign",
        "admin",
        "anonymous",
        "key",
    ] {
        for (vehicle, label) in [
            (101, "own"),
            (102, "empty"),
            (103, "foreign"),
            (999, "missing"),
        ] {
            e.record(
                &f,
                &format!("access-{actor}-{label}"),
                actor,
                vehicle,
                "telemetry/lanes",
                WINDOW,
            )
            .await;
        }
    }
    for (name, options) in [
        ("default", ""),
        ("battery", "lanes=battery"),
        ("drive", "lanes=drive"),
        ("location", "lanes=location"),
        ("climate", "lanes=climate"),
        ("charging", "lanes=charging"),
        ("health", "lanes=health"),
        ("four", "lanes=health,climate,location,charging"),
        ("ordering", "lanes=drive,battery"),
        ("dedup", "lanes=battery,drive,battery"),
        ("trimming", "lanes=%20battery%20,,drive,"),
        ("empty", "lanes="),
        ("commas", "lanes=,,"),
        ("unknown", "lanes=unknown"),
        ("five", "lanes=battery,drive,location,climate,health"),
        ("case", "lanes=Battery"),
        ("control", "lanes=battery%00"),
        ("auto", "resolution=auto"),
        ("1m", "resolution=1m"),
        ("5m", "resolution=5m"),
        ("1h", "resolution=1h"),
        ("bad-resolution", "resolution=1s"),
        ("empty-resolution", "resolution="),
        ("points-zero", "max_points=0"),
        ("points-negative", "max_points=-1"),
        ("points63", "max_points=63"),
        ("points64", "max_points=64"),
        ("points512", "max_points=512"),
        ("points513", "max_points=513"),
        ("points-max", "max_points=9223372036854775807"),
        ("points-overflow", "max_points=9223372036854775808"),
        ("points-bad", "max_points=NaN"),
        ("duplicate", "lanes=battery&lanes=drive"),
    ] {
        e.record(
            &f,
            name,
            "owner",
            101,
            "telemetry/lanes",
            &format!("{WINDOW}&{options}"),
        )
        .await;
    }
    for (name, query) in [
        ("equal", "from=2026-10-01T00:00:00Z&to=2026-10-01T00:00:00Z"),
        (
            "equal-filter",
            "from=2026-10-01T00:00:30Z&to=2026-10-01T00:00:30Z",
        ),
        (
            "reversed",
            "from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z",
        ),
        (
            "ninety-days",
            "from=2026-07-03T00:00:00Z&to=2026-10-01T00:00:00Z",
        ),
        (
            "over-ninety-days",
            "from=2026-07-02T23:59:59Z&to=2026-10-01T00:00:00Z",
        ),
        ("to-only", "to=2026-10-01T00:04:00Z"),
        ("bad-time", "from=bad"),
        ("priority", "from=bad&lanes=unknown"),
        (
            "selector-before-window",
            "from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z&lanes=unknown",
        ),
        (
            "window-before-resolution",
            "from=2026-07-01T00:00:00Z&to=2026-10-01T00:00:00Z&resolution=bad",
        ),
    ] {
        e.record(&f, name, "owner", 101, "telemetry/lanes", query)
            .await;
    }
    for size in [256, 257] {
        e.record(
            &f,
            &format!("selector-size-{size}"),
            "owner",
            101,
            "telemetry/lanes",
            &format!("{WINDOW}&lanes={}", "a".repeat(size)),
        )
        .await;
    }
    for (name, query, implicit_from) in [
        ("missing-time", "", true),
        (
            "from-only",
            "from=2026-10-01T00:00:00Z&resolution=1m",
            false,
        ),
    ] {
        let start = Utc::now();
        let mut response = f.request("owner", 102, "telemetry/lanes", query).await;
        let end = Utc::now();
        assert_eq!(response["status"], 200);
        let to: chrono::DateTime<Utc> = response["body"]["window"]["to"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(to >= start && to <= end);
        if implicit_from {
            let from: chrono::DateTime<Utc> = response["body"]["window"]["from"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(to - from, chrono::Duration::hours(24));
        }
        if let Ok(dir) = std::env::var("VEHICLE_DIAGNOSTIC_EVIDENCE") {
            let run = std::env::var("VEHICLE_DIAGNOSTIC_RUN").unwrap_or_else(|_| "actual".into());
            std::fs::write(
                std::path::Path::new(&dir).join(format!("{run}-{name}-unnormalized.json")),
                serde_json::to_vec_pretty(&response).unwrap(),
            )
            .unwrap();
        }
        response["body"]["window"]["to"] = json!("<verified-current-time>");
        if implicit_from {
            response["body"]["window"]["from"] = json!("<verified-current-time-minus-24h>");
        }
        if response["headers"].get("content-length").is_some() {
            response["headers"]["content-length"] = json!("<verified-body-byte-length>");
        }
        e.events.push(json!({"case":name,"response":response}));
    }
    let mut state = f.snapshot().await;
    for key in ["last_used_at", "updated_at"] {
        state["panda.api_keys"][0][key] = json!("<verified-api-key-clock>");
    }
    e.events
        .push(json!({"case":"final-state","state":state,"protection":f.protection().await}));
    e.finish("lanes", include_str!("diagnostic_lanes.json"));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated diagnostic TimescaleDB and Redis"]
async fn diagnostic_contract_dense_http_db() {
    let f = Fixture::new().await;
    f.seed().await;
    f.telemetry(true).await;
    let mut e = Evidence::default();
    e.events.push(json!({"case":"initial-state","state":f.snapshot().await,"protection":f.protection().await}));
    for (name, query) in [
        ("default", ""),
        ("max200", "per_page=200"),
        ("max201", "per_page=201"),
        ("second", "per_page=200&page=2"),
        ("third", "per_page=200&page=3"),
        ("past", "per_page=200&page=4"),
        ("selected", "fields=battery_level&per_page=200&page=2"),
    ] {
        e.record(&f, name, "owner", 101, "raw-data", query).await;
    }
    for (name, options) in [
        ("default", ""),
        ("auto64", "max_points=64"),
        ("auto512", "max_points=512"),
        ("1m64", "resolution=1m&max_points=64"),
        ("1m63", "resolution=1m&max_points=63"),
        ("1m512", "resolution=1m&max_points=512"),
        ("1m513", "resolution=1m&max_points=513"),
        ("5m", "resolution=5m"),
        ("1h", "resolution=1h"),
    ] {
        e.record(
            &f,
            name,
            "owner",
            101,
            "telemetry/lanes",
            &format!("from=2026-10-01T00:00:00Z&to=2026-10-01T10:00:00Z&{options}"),
        )
        .await;
    }
    for (name, to) in [
        ("exact-capacity", "2026-10-01T01:03:00Z"),
        ("below-capacity", "2026-10-01T01:02:00Z"),
    ] {
        e.record(
            &f,
            name,
            "owner",
            101,
            "telemetry/lanes",
            &format!("from=2026-10-01T00:00:00Z&to={to}&resolution=1m&max_points=64"),
        )
        .await;
    }
    e.events.push(
        json!({"case":"final-state","state":f.snapshot().await,"protection":f.protection().await}),
    );
    e.finish("dense", include_str!("diagnostic_dense.json"));
    f.cleanup().await;
}

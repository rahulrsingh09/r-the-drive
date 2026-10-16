use super::*;
use serde_json::{json, Value};
mod fixture;
use fixture::{id, Fixture};

fn compare(name: &str, actual: &Value, expected: &str) {
    if let Ok(root) = std::env::var("TRIP_CONTRACT_RUN") {
        std::fs::write(
            format!("{root}/{name}-actual.json"),
            serde_json::to_string_pretty(actual).unwrap(),
        )
        .unwrap();
    }
    if std::env::var_os("TRIP_CONTRACT_CAPTURE").is_none() {
        let expected: Value = serde_json::from_str(expected).unwrap();
        assert_eq!(
            actual.as_array().unwrap().len(),
            expected.as_array().unwrap().len(),
            "{name} count"
        );
        for (i, (a, e)) in actual
            .as_array()
            .unwrap()
            .iter()
            .zip(expected.as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, e, "{name} complete case {i}");
        }
    }
}

#[test]
fn trip_query_pure_contract() {
    let mut cases = Vec::new();
    for d in [
        -1,
        0,
        1,
        1499,
        1500,
        1501,
        2999,
        3000,
        3001,
        7499,
        7500,
        7501,
        15000,
        15001,
        36000,
        86400,
        i32::MAX,
    ] {
        cases.push(json!({"case":"detail bucket","duration":d,"bucket":detail_bucket_seconds(d)}));
    }
    for cap in [
        None,
        Some(39.9),
        Some(40.0),
        Some(250.0),
        Some(250.1),
        Some(39999.0),
        Some(40000.0),
        Some(250000.0),
        Some(250001.0),
        Some(f64::NAN),
    ] {
        cases.push(json!({"case":"capacity", "input":format!("{cap:?}"), "output":normalize_capacity_wh(cap)}));
    }
    for dt in [1, 2, 3, 30, 299, 300, 301] {
        for delta in [-0.51, -0.5, -0.05, 0.0, 0.049, 0.05, 0.5, 0.51] {
            for direct in [0, 1, 2] {
                let rows = [
                    TripPowerTelemetryRow {
                        ts: DateTime::from_timestamp(0, 0).unwrap(),
                        battery_level: Some(80.0),
                        battery_capacity_wh: Some(111_500.0),
                        power_kw: (direct > 0).then_some(20.0),
                        regen_power_kw: None,
                    },
                    TripPowerTelemetryRow {
                        ts: DateTime::from_timestamp(dt, 0).unwrap(),
                        battery_level: Some(80.0 - delta),
                        battery_capacity_wh: None,
                        power_kw: None,
                        regen_power_kw: (direct > 1).then_some(3.0),
                    },
                ];
                let data = derive_soc_power(&rows, dt as i32);
                cases.push(json!({"case":"power","dt":dt,"delta":delta,"direct":direct,"metadata":data.metadata,"at_start":data.estimated_at(rows[0].ts),"at_end":data.estimated_at(rows[1].ts)}));
            }
        }
    }
    compare("pure", &json!(cases), include_str!("pure-expected.json"));
}

async fn record(
    f: &Fixture,
    out: &mut Vec<Value>,
    label: &str,
    method: &str,
    path: &str,
    user: Option<u128>,
    body: Value,
    persisted: bool,
) -> Value {
    let response = f.request(method, path, user, body).await;
    let mut row = json!({"case":label,"response":response});
    if persisted {
        row["persisted"] = f.snapshot().await;
    }
    out.push(row);
    response
}

#[tokio::test]
#[ignore = "requires isolated TRIP_CONTRACT_TEST_DATABASE_URL and fixture Redis"]
async fn trip_query_http_database_contract() {
    let mut f = Fixture::new().await;
    f.seed().await;
    let vid = id(101);
    let empty = id(103);
    let trip = id(201);
    let dates = "from=2026-01-01T00:00:00Z&to=2026-02-01T00:00:00Z";
    let base = format!("/v1/trips?vehicle_id={vid}&{dates}");
    let tags = format!("/v1/vehicles/{vid}/trip-tags");
    let mut out = vec![json!({"case":"seed","persisted":f.snapshot().await})];
    for suffix in [
        "",
        "&limit=1",
        "&limit=1&page=2",
        "&per_page=2&limit=1&offset=1&page=99",
        "&limit=0&offset=-1",
        "&limit=999",
        "&page=-1",
        "&offset=1000",
        "&search=100%25_Road",
        "&search=alpha",
        "&search=%20%20",
        "&search=%5C",
        "&untagged=true",
        "&lifetime=true",
    ] {
        record(
            &f,
            &mut out,
            &format!("list {suffix}"),
            "GET",
            &(base.clone() + suffix),
            Some(1),
            json!(null),
            false,
        )
        .await;
    }
    for suffix in [
        format!("&tag_ids={}", id(301)),
        format!("&tag_ids={},{}&tag_match=any", id(301), id(302)),
        format!("&tag_ids={},{}&tag_match=all", id(301), id(302)),
        format!("&tag_ids={}&untagged=true", id(301)),
        format!("&tag_ids={}", id(303)),
        "&tag_ids=broken".into(),
        "&tag_match=broken".into(),
        "&limit=broken".into(),
        "&from=broken".into(),
    ] {
        record(
            &f,
            &mut out,
            &format!("filter {suffix}"),
            "GET",
            &(base.clone() + &suffix),
            Some(1),
            json!(null),
            false,
        )
        .await;
    }
    for tid in [201, 202, 203, 204, 205, 206, 999] {
        for endpoint in [
            "",
            "/detail",
            "/track",
            "/speed",
            "/elevation",
            "/power",
            "/series",
        ] {
            record(
                &f,
                &mut out,
                &format!("trip {tid}{endpoint}"),
                "GET",
                &format!("/v1/trips/{}{endpoint}?vehicle_id={vid}", id(tid)),
                Some(1),
                json!(null),
                false,
            )
            .await;
        }
    }
    // Repeated cache reads are characterized separately from cold responses.
    for endpoint in ["detail", "track"] {
        record(
            &f,
            &mut out,
            &format!("cache hit {endpoint}"),
            "GET",
            &format!("/v1/trips/{trip}/{endpoint}?vehicle_id={vid}"),
            Some(1),
            json!(null),
            false,
        )
        .await;
    }
    record(
        &f,
        &mut out,
        "power alias",
        "GET",
        &format!("/v1/vehicles/{vid}/drives/{trip}/power"),
        Some(1),
        json!(null),
        false,
    )
    .await;
    for (label, v) in [("populated", vid), ("empty", empty)] {
        for endpoint in [
            "/v1/trips",
            "/v1/trips/map",
            "/v1/trips/tire-pressure-timeline",
        ] {
            record(
                &f,
                &mut out,
                &format!("{label} {endpoint}"),
                "GET",
                &format!("{endpoint}?vehicle_id={v}&{dates}"),
                Some(1),
                json!(null),
                true,
            )
            .await;
        }
    }
    for endpoint in ["map", "tire-pressure-timeline"] {
        record(
            &f,
            &mut out,
            &format!("tagged {endpoint}"),
            "GET",
            &format!(
                "/v1/trips/{endpoint}?vehicle_id={vid}&{dates}&tag_ids={}",
                id(301)
            ),
            Some(1),
            json!(null),
            true,
        )
        .await;
        record(&f,&mut out,&format!("future empty {endpoint}"),"GET",&format!("/v1/trips/{endpoint}?vehicle_id={vid}&from=2027-01-01T00:00:00Z&to=2027-02-01T00:00:00Z"),Some(1),json!(null),false).await;
    }
    for user in [None, Some(2), Some(3), Some(4), Some(5)] {
        for path in [
            base.clone(),
            format!("/v1/trips/{trip}?vehicle_id={vid}"),
            format!("/v1/trips/{trip}/detail?vehicle_id={vid}"),
            format!("/v1/trips/{trip}/power?vehicle_id={vid}"),
            format!("/v1/trips/{trip}/series?vehicle_id={vid}"),
            tags.clone(),
        ] {
            record(
                &f,
                &mut out,
                &format!("access {user:?} {path}"),
                "GET",
                &path,
                user,
                json!(null),
                false,
            )
            .await;
        }
    }
    for path in [
        "/v1/trips".into(),
        "/v1/trips?vehicle_id=invalid".into(),
        format!("/v1/trips/not-uuid?vehicle_id={vid}"),
        format!("/v1/trips/{trip}?vehicle_id={}", id(102)),
    ] {
        record(
            &f,
            &mut out,
            &format!("malformed {path}"),
            "GET",
            &path,
            Some(1),
            json!(null),
            false,
        )
        .await;
    }
    let key = "panda_synthetic_trip_fixture_local_only";
    sqlx::query("INSERT INTO panda.api_keys(id,user_id,vehicle_id,key_hash,access_level,name) VALUES ($1,$2,$3,$4,'read','synthetic local only')")
        .bind(id(701)).bind(id(1)).bind(vid).bind(crate::routes::api_keys::hash_api_key(key).as_slice()).execute(&f.state.pool).await.unwrap();
    for (label, method, path, body) in [
        ("key allowed list", "GET", base.clone(), "null"),
        (
            "key foreign vehicle denied",
            "GET",
            format!("/v1/trips?vehicle_id={}&{dates}", id(102)),
            "null",
        ),
        (
            "key alias allowed",
            "GET",
            format!("/v1/vehicles/{vid}/drives/{trip}/power"),
            "null",
        ),
        (
            "key mutation denied",
            "POST",
            tags.clone(),
            r#"{"name":"Denied"}"#,
        ),
    ] {
        let response = f.raw(method, &path, None, body, Some(key)).await;
        out.push(json!({"case":label,"response":response,"persisted":f.snapshot().await}));
    }
    for (label, query) in [
        (
            "backwards range",
            "from=2026-02-01T00:00:00Z&to=2026-01-01T00:00:00Z",
        ),
        (
            "inclusive boundary",
            "from=2026-01-02T12:00:03Z&to=2026-01-02T12:00:03Z",
        ),
        ("default time range", ""),
    ] {
        record(
            &f,
            &mut out,
            label,
            "GET",
            &format!("/v1/trips?vehicle_id={vid}&{query}"),
            Some(1),
            json!(null),
            false,
        )
        .await;
    }
    // Every mutation records the entire affected history/catalog/assignment evidence.
    record(
        &f,
        &mut out,
        "tag viewer denied",
        "POST",
        &tags,
        Some(3),
        json!({"name":"denied"}),
        true,
    )
    .await;
    record(
        &f,
        &mut out,
        "tag outsider denied",
        "POST",
        &tags,
        Some(4),
        json!({"name":"denied"}),
        true,
    )
    .await;
    for body in [
        json!({}),
        json!({"name":" "}),
        json!({"name":"x".repeat(65)}),
        json!({"name":"alpha"}),
        json!({"name":" ALPHA "}),
        json!({"name":"new","color_token":"#ffff00"}),
        json!({"name":42}),
    ] {
        record(
            &f,
            &mut out,
            &format!("invalid create {body}"),
            "POST",
            &tags,
            Some(1),
            body,
            true,
        )
        .await;
    }
    let created = record(
        &f,
        &mut out,
        "manager creates Unicode tag",
        "POST",
        &tags,
        Some(2),
        json!({"name":"  Cafe\u{301}   Stop ","color_token":"success"}),
        true,
    )
    .await;
    let new_tag = f.register(&created, "<created-tag>");
    for body in [
        json!({}),
        json!({"name":null,"color_token":null}),
        json!({"name":"alpha"}),
        json!({"name":"Unique","color_token":"wrong"}),
        json!({"color_token":"danger"}),
        json!({"name":"Updated"}),
    ] {
        record(
            &f,
            &mut out,
            &format!("patch {body}"),
            "PATCH",
            &format!("{tags}/{new_tag}"),
            Some(2),
            body,
            true,
        )
        .await;
    }
    let assign = format!("{tags}/assignments");
    for (label, body) in [
        (
            "add",
            json!({"trip_ids":[id(201),id(202)],"tag_ids":[new_tag],"mode":"add"}),
        ),
        (
            "add repeat",
            json!({"trip_ids":[id(201),id(202)],"tag_ids":[new_tag],"mode":"add"}),
        ),
        (
            "remove",
            json!({"trip_ids":[id(201)],"tag_ids":[id(301)],"mode":"remove"}),
        ),
        (
            "replace",
            json!({"trip_ids":[id(201)],"tag_ids":[new_tag],"mode":"replace"}),
        ),
        (
            "replace clear",
            json!({"trip_ids":[id(202)],"tag_ids":[],"mode":"replace"}),
        ),
        (
            "empty trips",
            json!({"trip_ids":[],"tag_ids":[],"mode":"replace"}),
        ),
        (
            "empty tags",
            json!({"trip_ids":[trip],"tag_ids":[],"mode":"add"}),
        ),
        (
            "duplicate trips",
            json!({"trip_ids":[trip,trip],"tag_ids":[id(301)],"mode":"add"}),
        ),
        (
            "duplicate tags",
            json!({"trip_ids":[trip],"tag_ids":[id(301),id(301)],"mode":"add"}),
        ),
        (
            "foreign trip",
            json!({"trip_ids":[id(205)],"tag_ids":[id(303)],"mode":"add"}),
        ),
        (
            "foreign tag",
            json!({"trip_ids":[trip],"tag_ids":[id(303)],"mode":"add"}),
        ),
        (
            "unknown trip",
            json!({"trip_ids":[id(999)],"tag_ids":[id(301)],"mode":"replace"}),
        ),
        (
            "unknown tag",
            json!({"trip_ids":[trip],"tag_ids":[id(999)],"mode":"replace"}),
        ),
        (
            "too many trips",
            json!({"trip_ids":vec![trip;501],"tag_ids":[],"mode":"replace"}),
        ),
        (
            "too many tags",
            json!({"trip_ids":[trip],"tag_ids":vec![id(301);51],"mode":"replace"}),
        ),
        (
            "bad mode",
            json!({"trip_ids":[trip],"tag_ids":[],"mode":"invalid"}),
        ),
    ] {
        record(&f, &mut out, label, "POST", &assign, Some(2), body, true).await;
    }
    record(
        &f,
        &mut out,
        "viewer assignment denied",
        "POST",
        &assign,
        Some(3),
        json!({"trip_ids":[trip],"tag_ids":[],"mode":"replace"}),
        true,
    )
    .await;
    for tid in [id(303), id(999), new_tag, new_tag] {
        record(
            &f,
            &mut out,
            &format!(
                "delete {}",
                if tid == new_tag {
                    "created".into()
                } else {
                    tid.to_string()
                }
            ),
            "DELETE",
            &format!("{tags}/{tid}"),
            Some(1),
            json!(null),
            true,
        )
        .await;
    }
    for (label, body) in [
        ("malformed JSON", "{".to_owned()),
        (
            "oversized JSON",
            serde_json::to_string(&json!({"name":"x".repeat(65536)})).unwrap(),
        ),
    ] {
        let response = f.raw("POST", &tags, Some(1), &body, None).await;
        out.push(json!({"case":label,"response":response,"persisted":f.snapshot().await}));
    }
    // A failing second replace stage must roll back its preceding delete.
    sqlx::raw_sql("CREATE FUNCTION panda.synthetic_reject_assignment() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic assignment failure' USING ERRCODE='23514'; END $$; CREATE TRIGGER synthetic_reject BEFORE INSERT ON panda.trip_tag_assignments FOR EACH ROW EXECUTE FUNCTION panda.synthetic_reject_assignment();").execute(&f.state.pool).await.unwrap();
    record(
        &f,
        &mut out,
        "replace insert failure rollback",
        "POST",
        &assign,
        Some(1),
        json!({"trip_ids":[id(203)],"tag_ids":[id(301)],"mode":"replace"}),
        true,
    )
    .await;
    sqlx::raw_sql("DROP TRIGGER synthetic_reject ON panda.trip_tag_assignments; DROP FUNCTION panda.synthetic_reject_assignment();").execute(&f.state.pool).await.unwrap();
    let denied = sqlx::query("DELETE FROM panda.trips WHERE id=$1")
        .bind(trip)
        .execute(&f.state.pool)
        .await
        .unwrap_err();
    out.push(json!({"case":"protected history denial","sqlstate":denied.as_database_error().unwrap().code(),"persisted":f.snapshot().await}));
    let raw = json!(out);
    assert_raw_write_invariants(&raw);
    if let Ok(root) = std::env::var("TRIP_CONTRACT_RUN") {
        std::fs::write(
            format!("{root}/http-raw.json"),
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();
    }
    let mut normalized = raw;
    f.normalize(&mut normalized);
    compare("http", &normalized, include_str!("http-expected.json"));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "runs original ignored route tests on an authentic restricted disposable database"]
async fn trip_query_original_auth_contract() {
    let f = Fixture::new().await;
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["routes::trips::tests::", "--ignored", "--test-threads=1"])
        .env("DATABASE_URL", &f.state.config.database_url)
        .env("REDIS_URL", "redis://redis:6379/0")
        .output()
        .unwrap();
    if let Ok(root) = std::env::var("TRIP_CONTRACT_RUN") {
        std::fs::write(format!("{root}/original-auth-tests.log"), &output.stdout).unwrap();
    }
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("9 passed; 0 failed; 0 ignored"));
    f.cleanup().await;
}

fn assert_raw_write_invariants(raw: &Value) {
    let rows = raw.as_array().unwrap();
    let persisted = |name: &str| &rows.iter().find(|row| row["case"] == name).unwrap()["persisted"];
    for (before, after) in [
        ("add", "add repeat"),
        ("key mutation denied", "tag viewer denied"),
        ("tag viewer denied", "tag outsider denied"),
        ("oversized JSON", "replace insert failure rollback"),
        ("malformed JSON", "oversized JSON"),
    ] {
        assert_eq!(
            persisted(before),
            persisted(after),
            "raw state: {before} -> {after}"
        );
    }
    let deletes: Vec<_> = rows
        .iter()
        .filter(|row| row["case"] == "delete created")
        .collect();
    assert_eq!(deletes[0]["persisted"], deletes[1]["persisted"]);
    for row in rows {
        let body = &row["response"]["body"];
        if body.get("items").is_some() {
            assert_eq!(body["items"], body["data"]);
        }
    }
}

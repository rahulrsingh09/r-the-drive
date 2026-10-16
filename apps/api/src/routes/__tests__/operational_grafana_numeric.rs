use super::*;

fn series(metric: &str, points: &[[f64; 2]]) -> Value {
    json!({"target": metric, "datapoints": points})
}

// Hand-written expectations for the separately corrected numeric timestamp decoder.
pub(super) fn corrected_outputs(expected: &mut Value) {
    let t = 1_767_225_600_000.0;
    let fine = t + 1123.456;
    let battery = series("battery_level", &[[0.0, t], [50.0, fine]]);
    let power = series("power_kw", &[[-5.0, t], [12.0, fine], [0.0, t + 2000.0]]);
    let speed_first = series("speed_mph", &[[0.0, t]]);
    let outputs = [
        ("user-1/grafana", json!([battery.clone()])),
        ("user-2/grafana", json!([battery.clone()])),
        ("user-3/grafana", json!([battery.clone()])),
        ("user-7/grafana", json!([battery])),
        (
            "all_metrics",
            json!([
                series(
                    "battery_level",
                    &[[0.0, t], [50.0, fine], [100.0, t + 3000.0]]
                ),
                series("battery_capacity_wh", &[[135000.0, t]]),
                power.clone(),
                series("regen_power_kw", &[[0.0, t], [2.0, fine]]),
                series("speed_mph", &[[0.0, t], [45.0, fine]]),
                series("outside_temp_c", &[[-12.0, t], [20.0, fine]]),
                series("cabin_temp_c", &[[0.0, t], [22.0, fine]]),
                series("odometer_miles", &[[999.0, t], [1000.0, fine]])
            ]),
        ),
        ("duplicates", json!([speed_first.clone(), speed_first])),
        ("negative_max", json!([series("power_kw", &[[-5.0, t]])])),
        ("zero_max", json!([series("regen_power_kw", &[[0.0, t]])])),
        (
            "large_max",
            json!([series("outside_temp_c", &[[-12.0, t], [20.0, fine]])]),
        ),
        (
            "equal_bounds",
            json!([series("battery_level", &[[0.0, t]])]),
        ),
        ("fallback", json!([power])),
    ];
    for (name, body) in outputs {
        assert_eq!(
            expected[name]["status"], 500,
            "only the documented inherited decoder errors change"
        );
        expected[name] = json!({"status":200,"headers":{"content-type":"application/json","content-length":serde_json::to_vec(&body).unwrap().len().to_string()},"body":body});
    }
}

#[tokio::test]
#[ignore = "requires the dedicated fresh operational fixture; run with --include-ignored"]
async fn grafana_numeric_timestamp_regression() {
    let f = Fixture::new().await;
    f.seed().await;
    let actual = f
        .http(
            1,
            "POST",
            "/v1/grafana/query",
            Some(body(json!([target("power_kw", Some(10))]), Value::Null)),
        )
        .await;
    write("numeric-regression-response.json", &actual);
    let expected = json!([series(
        "power_kw",
        &[
            [-5.0, 1767225600000.0],
            [12.0, 1767225601123.456],
            [0.0, 1767225602000.0]
        ]
    )]);
    f.cleanup().await;
    assert_eq!(actual["status"], 200);
    assert_eq!(actual["body"], expected);
}

#[tokio::test]
#[ignore = "requires the dedicated fresh operational fixture; run with --include-ignored"]
async fn grafana_numeric_limit_and_epoch_boundaries() {
    let f = Fixture::new().await;
    f.seed().await;
    sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,power_kw) SELECT '2026-01-01T00:01:00Z'::timestamptz + i * interval '1 millisecond',$1,i + 0.5 FROM generate_series(0,10001)i")
        .bind(id(11)).execute(&f.state.pool).await.unwrap();
    let before = f.rows().await;
    let mut request = body(json!([target("power_kw", Some(11))]), json!(i64::MAX));
    request["range"] = json!({"from":"2026-01-01T00:01:00Z","to":"2026-01-01T00:02:00Z"});
    let actual = f.http(1, "POST", "/v1/grafana/query", Some(request)).await;
    let points: Vec<[f64; 2]> = (0..10000)
        .map(|i| [f64::from(i) + 0.5, 1767225660000.0 + f64::from(i)])
        .collect();
    assert_eq!(actual["status"], 200);
    assert_eq!(actual["body"], json!([series("power_kw", &points)]));
    assert_eq!(before, f.rows().await);
    sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,power_kw) VALUES('1969-12-31T23:59:59.123456Z',$1,-0.25),('1970-01-01Z',$1,'NaN'::float8)").bind(id(11)).execute(&f.state.pool).await.unwrap();
    let mut request = body(json!([target("power_kw", Some(11))]), Value::Null);
    request["range"] = json!({"from":"1969-12-31T23:59:59Z","to":"1970-01-01T00:00:00Z"});
    let epoch = f.http(1, "POST", "/v1/grafana/query", Some(request)).await;
    write(
        "numeric-boundaries.json",
        &json!({"limited":actual,"epoch":epoch}),
    );
    f.cleanup().await;
    assert_eq!(epoch["status"], 200);
    assert_eq!(
        epoch["body"],
        json!([{"target":"power_kw","datapoints":[[-0.25,-876.544],[null,0.0]]}])
    );
}

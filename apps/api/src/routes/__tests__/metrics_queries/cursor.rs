use super::*;

fn corrections() -> Vec<Value> {
    serde_json::from_str(include_str!("http-cursor-corrections.json")).unwrap()
}

pub(super) fn corrected_http() -> String {
    let mut baseline: Vec<Value> = serde_json::from_str(include_str!("http.json")).unwrap();
    let corrections = corrections();
    assert_eq!(
        corrections
            .iter()
            .map(|case| case["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [296, 299, 300, 302]
    );
    for correction in corrections {
        let original = &mut baseline[correction["index"].as_u64().unwrap() as usize];
        assert_eq!(original["case"], "batch");
        assert_eq!(original["input"], correction["input"]);
        assert_eq!(
            original["output"]["body_error"],
            "metric response interrupted"
        );
        assert_eq!(
            original["output"]["headers"],
            correction["output"]["headers"]
        );
        assert_eq!(original["output"]["status"], correction["output"]["status"]);
        assert_eq!(
            serde_json::from_str::<Value>(correction["output"]["wire"].as_str().unwrap()).unwrap(),
            correction["output"]["body"]
        );
        original["output"] = correction["output"].clone();
    }
    serde_json::to_string(&baseline).unwrap()
}

fn singular<'a>(baseline: &'a [Value], endpoint: &str, metric: &str, bucket: &str) -> &'a Value {
    let path = format!(
        "/v1/metrics/{endpoint}?vehicle_id={}&metric={metric}&from={FROM}&to={TO}&bucket={bucket}",
        id(10)
    );
    baseline.iter().find(|case| case["case"] == path).unwrap()
}

fn mixed_batch(baseline: &[Value], metrics: &[&str], bucket: &str) -> (Value, Value) {
    let full = bucket == "raw";
    let density = if full { "full" } else { "compact" };
    let input = json!({
        "vehicle_id":id(10), "from":FROM, "to":TO, "bucket":bucket, "density":density,
        "metrics":metrics.iter().map(|metric| json!({"metric":metric})).collect::<Vec<_>>()
    });
    let values = metrics
        .iter()
        .map(|metric| {
            singular(baseline, "value", metric, "day")["output"]["wire"]
                .as_str()
                .unwrap()
        })
        .collect::<Vec<_>>()
        .join(",");
    let series = metrics
        .iter()
        .map(|metric| {
            let points = singular(baseline, "series", metric, bucket)["output"]["wire"]
                .as_str()
                .unwrap();
            format!("{{\"metric\":\"{metric}\",\"points\":{points}}}")
        })
        .collect::<Vec<_>>()
        .join(",");
    let cap = if full { "" } else { ",\"max_points\":96" };
    let wire = format!(
        "{{\"values\":[{values}],\"series\":[{series}],\"bucket\":\"{bucket}\",\"density\":\"{density}\"{cap}}}"
    );
    let headers = baseline
        .iter()
        .find(|case| case["case"] == "batch")
        .unwrap()["output"]["headers"]
        .clone();
    let expected = json!({"status":200,"headers":headers,"wire":wire,"body":serde_json::from_str::<Value>(&wire).unwrap()});
    (input, expected)
}

#[tokio::test]
#[ignore = "requires the unique disposable metrics Docker database; execute explicitly"]
async fn metrics_cursor_reuses_one_connection_across_projections() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let fixture = DatabaseFixture::with_connections(1).await;
    seed(&fixture.pool).await;
    let state = state(&fixture.pool).await;
    let token =
        crate::middleware::auth::issue_access_token(id(1), Some(id(10)), &state.jwt_keys).unwrap();
    let app = TestApp {
        router: crate::routes::build_router(state),
    };
    let baseline: Vec<Value> = serde_json::from_str(include_str!("http.json")).unwrap();
    let before = persisted(&fixture.pool).await;
    let original_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    let mut pids = Vec::new();
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for repetition in 0..3 {
        for bucket in ["raw", "hour", "day"] {
            for metric in ["total_trips", "avg_efficiency", "battery_level"] {
                let case = singular(&baseline, "series", metric, bucket);
                let path = case["case"].as_str().unwrap();
                actual.push(json!({"repetition":repetition,"case":path,
                    "output":request(&app,Some(&token),"GET",path,None).await}));
                expected.push(json!({"repetition":repetition,"case":path,"output":case["output"]}));
                pids.push(
                    sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                        .fetch_one(&fixture.pool)
                        .await
                        .unwrap(),
                );
            }
        }
        for (metrics, bucket) in [
            (["total_trips", "avg_efficiency", "battery_level"], "hour"),
            (["avg_efficiency", "battery_level", "total_trips"], "raw"),
        ] {
            let (input, output) = mixed_batch(&baseline, &metrics, bucket);
            actual.push(json!({"repetition":repetition,"case":"mixed batch","input":input,
                "output":request(&app,Some(&token),"POST","/v1/metrics/batch",Some(input.to_string())).await}));
            expected.push(
                json!({"repetition":repetition,"case":"mixed batch","input":input,"output":output}),
            );
            pids.push(
                sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                    .fetch_one(&fixture.pool)
                    .await
                    .unwrap(),
            );
        }
    }
    let after = persisted(&fixture.pool).await;
    fixture.cleanup().await;
    assert!(pids.iter().all(|pid| *pid == original_pid), "{pids:?}");
    assert_eq!(before, after);
    let expected = serde_json::to_string(&expected).unwrap();
    compare("cursor-reuse", &json!(actual), &expected);
}

fn individual_output(batch: &Value, index: usize) -> Value {
    let mut output = batch.clone();
    let value = &batch["body"]["values"][index];
    let series = &batch["body"]["series"][index];
    output["body"]["values"] = json!([value]);
    output["body"]["series"] = json!([series]);
    let cap = batch["body"]
        .get("max_points")
        .map(|maximum| format!(",\"max_points\":{maximum}"))
        .unwrap_or_default();
    output["wire"] = json!(format!(
        "{{\"values\":[{{\"metric\":{},\"value\":{},\"unit\":{},\"label\":{},\"ts\":{}}}],\"series\":[{{\"metric\":{},\"points\":{}}}],\"bucket\":{},\"density\":{}{cap}}}",
        value["metric"], value["value"], value["unit"], value["label"], value["ts"],
        series["metric"], series["points"], batch["body"]["bucket"], batch["body"]["density"]
    ));
    output
}

#[tokio::test]
#[ignore = "requires the unique disposable metrics Docker database; execute explicitly"]
async fn metrics_cursor_corrected_batches_match_individual_metrics() {
    let fixture = DatabaseFixture::with_connections(1).await;
    seed(&fixture.pool).await;
    let state = state(&fixture.pool).await;
    let token =
        crate::middleware::auth::issue_access_token(id(1), Some(id(10)), &state.jwt_keys).unwrap();
    let app = TestApp {
        router: crate::routes::build_router(state),
    };
    let before = persisted(&fixture.pool).await;
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    let mut cadence = tokio::time::interval(std::time::Duration::from_millis(260));
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    for correction in corrections() {
        let batch = &correction["input"];
        cadence.tick().await;
        actual.push(json!({"input":batch,"output":request(&app,Some(&token),"POST","/v1/metrics/batch",Some(batch.to_string())).await}));
        expected.push(json!({"input":batch,"output":correction["output"]}));
        for (index, metric) in batch["metrics"].as_array().unwrap().iter().enumerate() {
            let mut input = batch.clone();
            input["metrics"] = json!([metric]);
            cadence.tick().await;
            actual.push(json!({"input":input,"output":request(&app,Some(&token),"POST","/v1/metrics/batch",Some(input.to_string())).await}));
            expected.push(
                json!({"input":input,"output":individual_output(&correction["output"],index)}),
            );
        }
    }
    let after = persisted(&fixture.pool).await;
    fixture.cleanup().await;
    assert_eq!(before, after);
    compare(
        "cursor-corrections",
        &json!(actual),
        &serde_json::to_string(&expected).unwrap(),
    );
}

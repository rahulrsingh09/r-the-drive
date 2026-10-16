use super::job_fixture::{evidence, id, snapshot, trip, vehicle, Fixture};
use super::*;
use serde_json::json;

tokio::task_local! { static OUTCOME: &'static str; }
pub(super) async fn injected_outcome() -> Option<Result<EnrichTripOutcome>> {
    let name = OUTCOME.try_with(|name| *name).ok()?;
    Some(match name {
        "complete" => Ok(EnrichTripOutcome::Complete {
            provider_called: true,
        }),
        "unavailable" => Ok(EnrichTripOutcome::Unavailable),
        "paused" => Err(WeatherPausedError.into()),
        "busy" => Err(WeatherBusyError.into()),
        "budget" => Err(WeatherBudgetError.into()),
        "retry" => Err(WeatherFetchError {
            status: reqwest::StatusCode::TOO_MANY_REQUESTS,
            retry_after_seconds: Some(900),
        }
        .into()),
        "cap" => Err(WeatherFetchError {
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
            retry_after_seconds: Some(100000),
        }
        .into()),
        "negative" => Err(WeatherFetchError {
            status: reqwest::StatusCode::BAD_GATEWAY,
            retry_after_seconds: Some(-10),
        }
        .into()),
        "pending" => std::future::pending().await,
        _ => Err(anyhow::anyhow!("synthetic provider failure")),
    })
}
async fn enabled(pool: &PgPool) {
    connections::ensure_defaults(pool).await.unwrap();
    sqlx::query("UPDATE panda.external_connection_settings SET enabled=true, mode='remote' WHERE id='open_meteo'").execute(pool).await.unwrap();
}
async fn state(pool: &PgPool, trip: Uuid) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('status',status,'attempts',attempts,'error',last_error,'completed',completed_at IS NOT NULL,'retry',CASE WHEN next_attempt_at<=updated_at THEN 'due' WHEN next_attempt_at=date_trunc('day',updated_at)+interval '1 day' THEN 'tomorrow' ELSE round(extract(epoch FROM next_attempt_at-updated_at))::text END) FROM panda.weather_enrichment_jobs WHERE trip_id=$1").bind(trip).fetch_one(pool).await.unwrap()
}
async fn run(pool: &PgPool, mode: &'static str) -> bool {
    OUTCOME
        .scope(
            mode,
            process_next_job(pool, &reqwest::Client::new(), "inert"),
        )
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; synthetic outcomes only"]
async fn jobs_weather_transitions_and_ordered_full_states() {
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 1).await;
    enabled(p).await;
    let mut observations = Vec::new();
    for (index, (mode, status, attempts, retry)) in [
        ("complete", "succeeded", 1, "due"),
        ("unavailable", "unavailable", 1, "due"),
        ("paused", "pending", 0, "due"),
        ("busy", "pending", 0, "60"),
        ("budget", "failed", 0, "tomorrow"),
        ("failure", "failed", 1, "120"),
        ("retry", "failed", 1, "900"),
        ("cap", "failed", 1, "86400"),
        ("negative", "failed", 1, "120"),
    ]
    .into_iter()
    .enumerate()
    {
        let t = trip(p, 100 + index as u128, v).await;
        enqueue(p, t).await.unwrap();
        assert!(run(p, mode).await);
        let actual = state(p, t).await;
        assert_eq!(actual["status"], status);
        assert_eq!(actual["attempts"], attempts);
        assert_eq!(actual["retry"], retry);
        observations.push(snapshot(p, json!({"mode":mode,"result":true,"state":actual})).await);
        sqlx::query("UPDATE panda.weather_enrichment_jobs SET next_attempt_at='2100-01-01Z' WHERE trip_id=$1").bind(t).execute(p).await.unwrap();
    }
    let t = trip(p, 150, v).await;
    enqueue(p, t).await.unwrap();
    for attempt in 1..=5 {
        assert!(run(p, "failure").await);
        let s = state(p, t).await;
        assert_eq!(s["attempts"], attempt);
        assert_eq!(s["retry"], (60_i64 * 2_i64.pow(attempt as u32)).to_string());
        observations.push(snapshot(p, s).await);
        sqlx::query(
            "UPDATE panda.weather_enrichment_jobs SET next_attempt_at=now() WHERE trip_id=$1",
        )
        .bind(t)
        .execute(p)
        .await
        .unwrap();
    }
    assert!(!run(p, "complete").await);
    enqueue(p, t).await.unwrap();
    assert!(!run(p, "complete").await);
    observations.push(snapshot(p, json!({"exhausted_reenqueue":state(p,t).await})).await);
    evidence("weather-transitions", &json!(observations));
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; no outbound calls"]
async fn jobs_weather_admission_recovery_cancel_and_policy() {
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 2).await;
    let t = trip(p, 200, v).await;
    enqueue(p, t).await.unwrap();
    connections::ensure_defaults(p).await.unwrap();
    let mut rows = Vec::new();
    assert!(!run(p, "complete").await);
    rows.push(snapshot(p, json!({"disabled":state(p,t).await})).await);
    enabled(p).await;
    sqlx::query("UPDATE panda.external_connection_settings SET forecast_url='http://127.0.0.1/forbidden' WHERE id='open_meteo'").execute(p).await.unwrap();
    assert!(!run(p, "complete").await);
    rows.push(snapshot(p, json!({"policy_denied":state(p,t).await})).await);
    sqlx::query(
        "UPDATE panda.external_connection_settings SET forecast_url=$1 WHERE id='open_meteo'",
    )
    .bind(weather::FORECAST_URL)
    .execute(p)
    .await
    .unwrap();
    sqlx::query("INSERT INTO panda.external_connection_activity(connection_id,request_count,usage_date) VALUES('open_meteo',$1,CURRENT_DATE)").bind(DAILY_REQUEST_BUDGET).execute(p).await.unwrap();
    assert!(!run(p, "complete").await);
    rows.push(snapshot(p, json!({"budget":state(p,t).await})).await);
    sqlx::query("UPDATE panda.external_connection_activity SET usage_date=CURRENT_DATE-1 WHERE connection_id='open_meteo'").execute(p).await.unwrap();
    let pool = p.clone();
    let task = tokio::spawn(async move { run(&pool, "pending").await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state(p, t).await["status"] == "running" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!run(p, "complete").await);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    rows.push(snapshot(p, json!({"cancelled":state(p,t).await})).await);
    sqlx::query("UPDATE panda.weather_enrichment_jobs SET updated_at=now()-interval '16 minutes' WHERE trip_id=$1").bind(t).execute(p).await.unwrap();
    sqlx::query(
        "UPDATE panda.external_connection_settings SET enabled=false WHERE id='open_meteo'",
    )
    .execute(p)
    .await
    .unwrap();
    assert!(!run(p, "complete").await);
    assert_eq!(state(p, t).await["attempts"], 0);
    rows.push(snapshot(p, json!({"recovered_disabled":state(p,t).await})).await);
    enabled(p).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let pool = p.clone();
        tasks.spawn(async move { run(&pool, "complete").await });
    }
    let mut claimed = 0;
    while let Some(result) = tasks.join_next().await {
        claimed += usize::from(result.unwrap());
    }
    assert_eq!(claimed, 1);
    rows.push(
        snapshot(
            p,
            json!({"concurrent_winners":claimed,"state":state(p,t).await}),
        )
        .await,
    );
    enqueue(p, t).await.unwrap();
    assert!(!run(p, "complete").await);
    rows.push(snapshot(p, json!({"succeeded_reenqueue":state(p,t).await})).await);
    evidence("weather-admission", &json!(rows));
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; actual local enrichment only"]
async fn jobs_weather_actual_enrichment_and_malformed_bounds() {
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 3).await;
    enabled(p).await;
    let client = reqwest::Client::new();
    let mut rows = Vec::new();
    for n in 300..303 {
        let t = trip(p, n, v).await;
        if n == 300 {
            sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,trip_id,outside_temp_c) VALUES($1,'2026-01-01T12:00Z',$2,18.5)").bind(v).bind(t).execute(p).await.unwrap();
        }
        if n == 301 {
            sqlx::query("UPDATE panda.trips SET start_lat=NULL,start_lng=NULL WHERE id=$1")
                .bind(t)
                .execute(p)
                .await
                .unwrap();
        }
        if n == 302 {
            sqlx::query(
                "UPDATE panda.trips SET ended_at=started_at+interval '25 hours' WHERE id=$1",
            )
            .bind(t)
            .execute(p)
            .await
            .unwrap();
        }
        enqueue(p, t).await.unwrap();
        assert!(process_next_job(p, &client, "").await.unwrap());
        rows.push(snapshot(p, state(p, t).await).await);
    }
    assert_eq!(state(p, id(300)).await["status"], "succeeded");
    assert_eq!(state(p, id(301)).await["status"], "unavailable");
    assert_eq!(state(p, id(302)).await["status"], "failed");
    let protected:bool=sqlx::query_scalar("SELECT NOT has_table_privilege(current_user,'panda.trips','DELETE') AND NOT has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE') AND NOT (SELECT rolsuper FROM pg_roles WHERE rolname=current_user)").fetch_one(p).await.unwrap();
    assert!(protected);
    evidence("weather-actual", &json!(rows));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; synthetic completion only"]
async fn jobs_weather_ordered_claims_and_failed_status_writes() {
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 4).await;
    enabled(p).await;
    let a = trip(p, 400, v).await;
    let b = trip(p, 401, v).await;
    enqueue(p, a).await.unwrap();
    enqueue(p, b).await.unwrap();
    sqlx::query("UPDATE panda.weather_enrichment_jobs SET next_attempt_at=CASE WHEN trip_id=$1 THEN '2020-01-02Z'::timestamptz ELSE '2020-01-01Z'::timestamptz END").bind(a).execute(p).await.unwrap();
    sqlx::query("ALTER TABLE panda.weather_enrichment_jobs ADD CONSTRAINT synthetic_reject_success CHECK(status<>'succeeded')").execute(p).await.unwrap();
    let error = OUTCOME
        .scope("complete", process_next_job(p, &reqwest::Client::new(), ""))
        .await
        .unwrap_err();
    assert_eq!(state(p, b).await["status"], "running");
    assert_eq!(state(p, a).await["status"], "pending");
    let mut rows = vec![
        snapshot(
            p,
            json!({"error":error.to_string(),"state":state(p,b).await}),
        )
        .await,
    ];
    sqlx::query(
        "ALTER TABLE panda.weather_enrichment_jobs DROP CONSTRAINT synthetic_reject_success",
    )
    .execute(p)
    .await
    .unwrap();
    sqlx::query("ALTER TABLE panda.weather_enrichment_jobs ADD CONSTRAINT synthetic_reject_failure CHECK(status<>'failed')").execute(p).await.unwrap();
    let error = OUTCOME
        .scope("failure", process_next_job(p, &reqwest::Client::new(), ""))
        .await
        .unwrap_err();
    rows.push(
        snapshot(
            p,
            json!({"error":error.to_string(),"state":state(p,a).await}),
        )
        .await,
    );
    enqueue(p, id(999)).await.unwrap();
    rows.push(snapshot(p, json!({"missing_trip_enqueued":state(p,id(999)).await})).await);
    sqlx::query(
        "ALTER TABLE panda.weather_enrichment_jobs DROP CONSTRAINT synthetic_reject_failure",
    )
    .execute(p)
    .await
    .unwrap();
    assert!(process_next_job(p, &reqwest::Client::new(), "")
        .await
        .unwrap());
    rows.push(snapshot(p, json!({"missing_trip_processed":state(p,id(999)).await})).await);
    evidence("weather-write-failures", &json!(rows));
    f.cleanup().await;
}

#[test]
fn jobs_retry_decision_preserves_caps_and_provider_precedence() {
    for (attempts, retry_after, expected) in [
        (0, 0, 60),
        (1, 0, 120),
        (5, 0, 1920),
        (8, 0, 15360),
        (99, 0, 15360),
        (1, -1, 120),
        (1, 900, 900),
        (1, i64::MAX, 86400),
    ] {
        assert_eq!(super::jobs::retry_seconds(attempts, retry_after), expected);
    }
}

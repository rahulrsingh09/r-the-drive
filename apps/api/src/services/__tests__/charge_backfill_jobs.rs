use super::*;
use crate::services::weather_enrichment::job_fixture::{
    credentials, evidence, id, snapshot, vehicle, Fixture,
};
use serde_json::json;

#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; inert credentials only"]
async fn jobs_charge_claim_concurrency_stale_errors_and_spawn() {
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 40).await;
    let mut rows = Vec::new();
    let errors = vec![
        claim(p, id(404)).await.unwrap_err().to_string(),
        claim(p, v).await.unwrap_err().to_string(),
    ];
    rows.push(
        snapshot(
            p,
            json!({"errors":errors,"status":get_status(p,v).await.unwrap()}),
        )
        .await,
    );
    sqlx::query(
        "UPDATE panda.vehicles SET history_backfill_status='running',updated_at=now() WHERE id=$1",
    )
    .bind(v)
    .execute(p)
    .await
    .unwrap();
    assert!(matches!(
        claim(p, v).await,
        Err(ChargeBackfillError::AlreadyRunning)
    ));
    sqlx::query("UPDATE panda.vehicles SET updated_at=now()-interval '121 minutes' WHERE id=$1")
        .bind(v)
        .execute(p)
        .await
        .unwrap();
    assert!(matches!(
        claim(p, v).await,
        Err(ChargeBackfillError::CredentialsNotFound)
    ));
    credentials(p, v).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let p = p.clone();
        tasks.spawn(async move { claim(&p, v).await });
    }
    let mut wins = Vec::new();
    let mut errors = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(c) => wins.push(c),
            Err(e) => errors.push(e.to_string()),
        }
    }
    assert_eq!(wins.len(), 1);
    assert_eq!(errors, vec!["backfill already running"; 7]);
    let claimed = wins.pop().unwrap();
    assert_eq!(claimed.encrypted_tokens, vec![0, 1, 2]);
    rows.push(snapshot(p,json!({"winners":1,"errors":errors,"claimed":{"vehicle_id":claimed.vehicle_id,"rivian_vehicle_id":claimed.rivian_vehicle_id,"encrypted_tokens":claimed.encrypted_tokens}})).await);
    let error = run_claimed(p, &reqwest::Client::new(), "bad-key", claimed)
        .await
        .unwrap_err();
    rows.push(
        snapshot(
            p,
            json!({"error":error.to_string(),"status":get_status(p,v).await.unwrap()}),
        )
        .await,
    );
    let _ = claim(p, v).await.unwrap();
    let pool = p.clone();
    let task = tokio::spawn(async move {
        execute_charge_history_backfill(&pool, v, std::future::pending()).await
    });
    tokio::task::yield_now().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(matches!(
        claim(p, v).await,
        Err(ChargeBackfillError::AlreadyRunning)
    ));
    rows.push(snapshot(p, json!({"cancelled":get_status(p,v).await.unwrap()})).await);
    sqlx::query("UPDATE panda.vehicles SET updated_at=now()-interval '121 minutes' WHERE id=$1")
        .bind(v)
        .execute(p)
        .await
        .unwrap();
    let admitted = spawn(p.clone(), "bad-key".into(), v).await.unwrap();
    assert_eq!(
        serde_json::to_value(&admitted).unwrap(),
        json!({"ok":true,"status":"running"})
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if get_status(p, v).await.unwrap().status.as_deref() == Some("error") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    rows.push(
        snapshot(
            p,
            json!({"spawn":admitted,"status":get_status(p,v).await.unwrap()}),
        )
        .await,
    );
    let claimed = claim(p, v).await.unwrap();
    let key = age::x25519::Identity::generate();
    use age::secrecy::ExposeSecret;
    let error = run_claimed(
        p,
        &reqwest::Client::new(),
        key.to_string().expose_secret(),
        claimed,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().starts_with("token decrypt failed:"));
    rows.push(snapshot(p, json!({"error":error.to_string()})).await);
    let _ = claim(p, v).await.unwrap();
    assert_eq!(
        execute_charge_history_backfill(p, v, async { Ok(()) })
            .await
            .unwrap(),
        0
    );
    rows.push(
        snapshot(
            p,
            json!({"synthetic_success":get_status(p,v).await.unwrap()}),
        )
        .await,
    );
    let _ = claim(p, v).await.unwrap();
    assert_eq!(
        execute_charge_history_backfill(p, v, async { Ok(()) })
            .await
            .unwrap(),
        0
    );
    rows.push(snapshot(p, json!({"repeat":get_status(p,v).await.unwrap()})).await);
    evidence("charge-jobs", &json!(rows));
    f.cleanup().await;
}
